defmodule OneCore.Plugins.Rename do
  @moduledoc """
  Renaming a plugin, as `git mv` does a file: its artifact, `<theirs>/plugins/<name>`, is copied to the new name with
  its history, deployed there and installed by the new name for whoever had it, and what it keeps for each of them
  (`<theirs>/own/<name>`) is copied too. Only once the new one runs and what it keeps is the same as the old is the
  old removed, so nothing is ever only in one place being written. The old name then names nothing. The plugins
  that keep something of another plugin by its name observe `renamed`, and follow it.

  What can't follow is refused before anything is copied: what its machines keep for a service, what it made for
  people, which they and their plugins may name, encrypted data, which is bound to its name, and the plugins that
  name it in their code.
  """
  alias OneCore.{Bucket, Hooks, Server, Servers}
  alias OneCore.Plugins
  alias OneCore.Plugins.{Deploy, Installed, Store}

  @doc """
  Renames the plugin `name` of `owner`'s, which is theirs, to `to`, for `asked`. Gives `:ok`, or `{:error, why}`
  with everything as it was.
  """
  def rename(owner, name, to, principal) do
    was = "#{owner}/plugins/#{name}"
    is = "#{owner}/plugins/#{to}"
    installs = installs(owner, was)
    moved = for {at, as} <- installs, do: {at, as, if(as == name, do: to, else: as)}

    with :ok <- Installed.named(to),
         :ok <- renamable(owner, name, to, was, is, moved),
         :ok <- follows(installs, name, was) do
      case moved(owner, name, to, was, is, moved) do
        :ok ->
          for {at, as, now} <- moved do
            about = %{"was" => was, "is" => is, "for" => at, "from" => as, "to" => now}
            Hooks.observe("renamed", about, {elem(principal, 0), at})
          end

          for {at, as, now} <- moved, as != now, blob <- Bucket.list("#{at}/own/#{as}/"), do: Bucket.remove(blob)
          for blob <- Bucket.list("#{was}/"), do: Bucket.remove(blob)
          Bucket.remove("network/deployed/#{owner}/#{name}")
          :ok

        {:error, why} ->
          # Whoever had it finds it by its old name again, and what was copied goes: the old is all there.
          installed(installs, fn at, installed ->
            installed
            |> Map.reject(&(elem(&1, 1) == is))
            |> Map.merge(Map.new(for {^at, as} <- installs, do: {as, was}))
          end)

          for {at, as, now} <- moved, as != now, blob <- Bucket.list("#{at}/own/#{now}/"), do: Bucket.remove(blob)
          for blob <- Bucket.list("#{is}/"), do: Bucket.remove(blob)
          Bucket.remove("network/deployed/#{owner}/#{to}")
          {:error, "#{name} wasn't renamed, since #{why} It's as it was."}
      end
    end
  end

  # Copies it to the new name and has whoever had it find it there, then copies what was written meanwhile, and
  # gives whether every copy is the same as what it was copied from.
  defp moved(owner, name, to, was, is, moved) do
    with :ok <- copy("#{was}/", "#{is}/", &(&1 != "latest" and not String.starts_with?(&1, "installed/"))) do
      # Whoever had it stops finding it by its old name before what it keeps is copied, so nothing's written there
      # after. Its latest version is copied last: until then the new name names no plugin.
      installs = for {at, as, _now} <- moved, do: {at, as}
      installed(installs, fn _at, installed -> Map.reject(installed, &(elem(&1, 1) == was)) end)
      {:ok, latest, _tag} = Bucket.read("#{was}/latest")
      :ok = Bucket.write("#{is}/latest", latest)
      {_number, code, _tag} = Store.latest(is)
      installed? = Enum.any?(installs, &(elem(&1, 0) == owner))

      with :ok <- each(moved, fn {at, as, now} -> copy("#{at}/own/#{as}/", "#{at}/own/#{now}/") end),
           files = skill(Store.source(is, code), name, to),
           {:ok, _number, _warnings} <- Deploy.deploy(owner, to, files, :stored, :stored, fn _ -> installed? end),
           :ok <- each(moved, fn {at, _as, now} -> if(at == owner, do: :ok, else: Installed.install(at, now, is)) end),
           :ok <- runs_renamed(moved, is) do
        each(moved, fn {at, as, now} -> copy("#{at}/own/#{as}/", "#{at}/own/#{now}/") end)
      end
    end
  end

  # Its skill is named as the plugin is (`OneCore.Plugins.Version`).
  defp skill(files, name, to) do
    Map.update(files, "SKILL.md", "", &String.replace(&1, ~r/^name:\s*#{name}\s*$/m, "name: #{to}", global: false))
  end

  # Only one's own plugin is renamed, by a name that's never had anything, and not one a server ships, which it
  # would bring back by its old name.
  defp renamable(owner, name, to, was, is, moved) do
    existing = for {at, as, now} <- moved, as != now, Bucket.list("#{at}/own/#{now}/") != [], do: at

    cond do
      Plugins.plugin(owner, name) not in [nil, was] or Store.latest(was) == nil ->
        {:error,
         "#{owner} has no plugin of their own named #{name}. One installed from someone else's is theirs to rename."}

      Plugins.plugin(owner, to) != nil or Bucket.list("#{is}/") != [] or existing != [] ->
        {:error, "#{to} is a plugin of #{owner}'s already, or was one and kept something. Choose another name."}

      is_map_key(Application.get_env(:one_core, :bundled, %{}), name) ->
        {:error, "#{name} ships with the server by that name, which would bring it back. It isn't renamed."}

      true ->
        :ok
    end
  end

  # Who it's installed for, each with the name it's installed by there: whoever's it is first, if they have it.
  defp installs(owner, was) do
    for at <- Enum.uniq([owner | Installed.installed_for(was)]),
        {installed, _tag} = Installed.read(at),
        {as, ^was} <- installed,
        do: {at, as}
  end

  # What would be left naming its old name, or couldn't be copied, refuses it before anything is changed.
  defp follows(installs, name, was) do
    servers = Enum.uniq(Enum.flat_map(installs, fn {at, _as} -> Servers.acting(at) end))

    serving =
      for key <- servers, {^was, service} <- Servers.installed(key), do: "#{service} on #{Servers.name(key) || key}"

    naming =
      for {_owner, installed} <- Plugins.installed(),
          {other, artifact} <- installed,
          artifact != was,
          version = Plugins.version(artifact),
          version != nil,
          name in version.uses or
            Enum.any?(version.hooks ++ version.permissions, &String.starts_with?(&1, [name <> ".", name <> "_"])),
          uniq: true,
          do: other

    {_number, code, _tag} = Store.latest(was)

    makes =
      Enum.any?(Store.source(was, code), fn {path, text} ->
        String.starts_with?(path, "server/") and text =~ ~r/\bmake\(/
      end)

    encrypted = for {at, as} <- installs, Bucket.list("#{at}/own/#{as}/keys/") != [], do: at

    cond do
      serving != [] ->
        {:error,
         "#{name}'s services keep what they need on their machines by its name: #{Enum.join(serving, ", ")}. " <>
           "Renaming a plugin with services installed isn't built yet."}

      naming != [] ->
        {:error,
         "#{Enum.join(naming, ", ")} #{if length(naming) == 1, do: "names", else: "name"} #{name} in its code. Change that first."}

      makes ->
        {:error,
         "#{name} makes artifacts for people (`OneCore.Call.make/2`), named with its name, which they and their " <>
           "plugins may keep. Renaming one that does isn't built yet."}

      encrypted != [] ->
        {:error,
         "What #{name} keeps for #{Enum.join(encrypted, ", ")} is encrypted, bound to its name, so it isn't renamed."}

      true ->
        :ok
    end
  end

  # Changes what's installed for each of `installs` as `change` says, and has their servers look.
  defp installed(installs, change) do
    owners = Enum.uniq(for {at, _as} <- installs, do: at)
    for at <- owners, do: :ok = Installed.note(at, &{:ok, change.(at, &1)})
    for at <- owners, do: Plugins.reload(at)
    Server.notify_servers(owners)
  end

  # Whether the new name runs for each of them, as the plugin renamed.
  defp runs_renamed(moved, is) do
    each(moved, fn {at, _as, now} ->
      case Plugins.versions([at]) do
        %{^now => %{artifact: ^is}} -> :ok
        _ -> {:error, "#{now} doesn't run for #{at}."}
      end
    end)
  end

  defp each(list, fun), do: Enum.find_value(list, :ok, &with(:ok <- fun.(&1), do: nil))

  # Copies the blobs under `from` to the same names under `to`, those `keep` says of their names there. One that's
  # there already is left if it's the same, and is why it stops otherwise.
  defp copy(from, to, keep \\ fn _ -> true end) do
    names = for blob <- Bucket.list(from), name = String.replace_prefix(blob, from, ""), keep.(name), do: name

    names
    |> Enum.chunk_every(32)
    |> Enum.find_value(:ok, fn names ->
      read = Bucket.read_all(Enum.map(names, &(from <> &1)))

      Enum.find_value(Enum.zip(names, read), fn
        {name, {:ok, data, _tag}} ->
          case Bucket.write(to <> name, data) do
            :ok ->
              nil

            :exists ->
              if match?({:ok, ^data, _}, Bucket.read(to <> name)),
                do: nil,
                else: {:error, "#{to}#{name} differs from #{from}#{name}."}
          end

        # Removed since it was listed.
        {_name, :absent} ->
          nil
      end)
    end)
  end
end
