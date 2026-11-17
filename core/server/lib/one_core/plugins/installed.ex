defmodule OneCore.Plugins.Installed do
  @moduledoc """
  Installing a plugin for someone and removing it. A plugin is installed for a user, for a group or for the network, and
  that's who it runs for. What's installed for someone is one blob under their prefix, `installed`: each plugin's name
  there, with the plugin it is. A call that runs for someone finds a plugin by its name among what's installed for
  them, and then among what's installed for the network, which is for everyone in it (`OneCore.Plugins.chain/1`). A
  plugin has an artifact of its own for each one it was installed for, `<theirs>/own/<name>`, and the plugins it uses
  are found the same way from whoever it was installed for.

  That someone installed a plugin is kept with the plugin too, as the blob `installed/<who>` in its artifact, which a
  server acting for them writes: so a deploy tells their servers that it has a new version.
  """
  alias OneCore.{Bucket, Plugins, Server}
  alias OneCore.Plugins.{Deploy, Store, Version}

  @doc """
  Installs the plugin `artifact` for `owner`, who find it by `name`. The plugins it uses are there for them first, as
  they are when it's deployed.
  """
  def install(owner, name, artifact) do
    installed = fn
      %{^name => ^artifact} = installed ->
        {:ok, installed}

      %{^name => other} ->
        {:error, "#{other} is installed for #{owner} as #{name}. Remove it first, or add this by another name."}

      installed ->
        {:ok, Map.put(installed, name, artifact)}
    end

    with :ok <- named(name),
         {number, code, _tag} <- Store.latest(artifact) || {:error, "There's no plugin #{artifact}"},
         trusted = Store.about(artifact)["trusted"] != false,
         {:ok, version} <- latest_here(artifact, code) || Store.version(artifact, code, true, trusted),
         :ok <- Deploy.fits(owner, name, version),
         :ok <- note(owner, installed) do
      Bucket.write("#{artifact}/installed/#{owner}", "")
      Plugins.reload(owner, [{artifact, number, {:ok, version}}])
      Server.notify_servers([owner])
    else
      :error -> {:error, "#{artifact}'s latest version doesn't compile here, so it isn't installed."}
      {:error, why} -> {:error, why}
    end
  end

  # The version of the plugin `artifact` running here, if it's the set of files named `code`.
  defp latest_here(artifact, code) do
    with %Version{code: ^code} = version <- Plugins.version(artifact), do: {:ok, version}, else: (_ -> nil)
  end

  @doc "Removes a plugin from what's installed for `owner`. It stays whoever's it is, and what it kept stays."
  def uninstall(owner, name) do
    # What uses it still finds it where it's installed for the network too.
    after_it = Map.new(Plugins.versions(Plugins.chain(owner) -- [owner]))

    used_by =
      for {other, version} <- Plugins.versions([owner]), name in version.uses, not is_map_key(after_it, name), do: other

    removed = fn
      %{^name => _artifact} = installed -> {:ok, Map.delete(installed, name)}
      _ -> {:error, "No plugin named #{name} is installed for #{owner}"}
    end

    with [] <- used_by,
         :ok <- last_launcher(owner, name),
         artifact = Plugins.plugin(owner, name),
         :ok <- note(owner, removed) do
      {installed, _tag} = read(owner)
      if artifact not in Map.values(installed), do: Bucket.remove("#{artifact}/installed/#{owner}")
      Plugins.reload(owner)
      Server.notify_servers([owner])
    else
      {:error, why} -> {:error, why}
      used_by -> {:error, "#{name} is used by #{Enum.join(used_by, ", ")}. Remove what uses it first."}
    end
  end

  # A network with no launcher has no way in, so the one it has stays until another version replaces it.
  defp last_launcher(owner, name) do
    with %{^name => version} <- Map.new(Plugins.versions([owner])),
         true <- Deploy.launcher?(version) do
      {:error,
       "#{name} is this network's launcher, which every client opens first, and a network with none has no way in. " <>
         "To replace it, deploy another as its new version: ONE_SUBJECT=network one deploy --name #{name} --files-from <folder>"}
    else
      _ -> :ok
    end
  end

  @doc "Who the plugin `artifact` is installed for, besides whoever's it is, as they installed it."
  def installed_for(artifact) do
    for blob <- Bucket.list("#{artifact}/installed/"), do: String.replace_prefix(blob, "#{artifact}/installed/", "")
  end

  @doc "Whether `name` is one a plugin is installed by: `:ok`, or `{:error, why}`."
  def named(name) do
    cond do
      not (is_binary(name) and name =~ ~r/^[a-z][a-z0-9-]*$/) ->
        {:error, "A plugin's name is lowercase letters, digits and hyphens, starting with a letter"}

      # A client finds the UI library where it finds a plugin's files, by this name (`OneCore.MCP`).
      name == "ui" ->
        {:error, "ui is the UI library's, which is One Core's own: name the plugin something else"}

      true ->
        :ok
    end
  end

  @doc """
  What's installed for an owner, as the bucket has it: each plugin's name there with the plugin, and the blob's tag,
  which is none if there's no blob.
  """
  def read(owner) do
    case Bucket.read("#{owner}/installed") do
      {:ok, installed, tag} -> {JSON.decode!(installed), tag}
      :absent -> {%{}, nil}
    end
  end

  @doc "Changes what's installed for an owner, as `change` says of what's there. Two changes at once both land."
  def note(owner, change) do
    {installed, tag} = read(owner)

    with {:ok, next} <- change.(installed) do
      blob = "#{owner}/installed"

      cond do
        next == installed -> :ok
        tag == nil and Bucket.write(blob, JSON.encode!(next)) == :ok -> :ok
        tag != nil and Bucket.replace(blob, JSON.encode!(next), tag) == :ok -> :ok
        true -> note(owner, change)
      end
    end
  end
end
