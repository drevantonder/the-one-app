defmodule OneCore.Plugins.Requires do
  @moduledoc """
  What a plugin `requires` (`requires/0`, `OneCore.Plugin`): the other plugins it needs, by name, each with why, which
  are its requirements; and the other way, the plugins it's `required_by`. A requirement is there for an owner where it
  runs for them: installed for them, or for a scope they're in, the network.

  These only say what's what; `OneCore.Plugins.Lifecycle` makes the moves.
  """
  alias OneCore.Plugins
  alias OneCore.Plugins.{Installed, Lifecycle, Store}

  @network "network"

  @doc """
  What the plugin `artifact`'s version `number` (its latest where `nil`) requires, as it described itself: each
  requirement's name and why, `%{"google-account" => "to reach your mail"}`. None where no server has described it yet.
  """
  def requires(artifact, number \\ nil) do
    with {:ok, version} <- described(artifact, number), do: version.requires, else: (_ -> %{})
  end

  @doc """
  The requirements of the plugin `artifact`, as `requires` says them, that aren't there for `owner`, and theirs in
  turn, deepest first, so each comes before what requires it. Each is

      %{name: "google-account", title: "Google", by: "gmail", why: "to reach your mail",
        plugin: "network/plugins/google-account", number: 7, trusted: true, deleted: false}

  `by` is the plugin that requires it, and `why` its reason. `plugin` and `number` are where to get it from: the
  owner's Recently Deleted (`deleted`, to restore), their own plugin of that name, that of whoever's plugin requires
  it, or the network's; `nil` where it's none of these, so it can't be installed. `trusted` is whether it would be
  where nobody says (`OneCore.Plugins.Lifecycle.trusted?/2`).
  """
  def missing(owner, artifact, requires), do: found(owner, artifact, requires, [Path.basename(artifact)], [])

  defp found(owner, artifact, requires, seen, found) do
    there = Plugins.running(owner)
    by = Path.basename(artifact)

    Enum.reduce(Enum.sort(requires), found, fn {name, why}, found ->
      if is_map_key(there, name) or name in seen or Enum.any?(found, &(&1.name == name)) do
        found
      else
        requirement = Map.merge(source(owner, name, Store.whose(artifact)), %{by: by, why: why})

        found =
          if requirement.plugin,
            do: found(owner, requirement.plugin, requires(requirement.plugin, requirement.number), [name | seen], found),
            else: found

        found ++ [requirement]
      end
    end)
  end

  defp source(owner, name, whose) do
    record = elem(Installed.read(owner), 0)[name]

    {artifact, number, deleted} =
      if match?(%{"state" => "recently_deleted"}, record) do
        {record["plugin"], record["version"], true}
      else
        Enum.find_value(Enum.uniq([owner, whose, @network]), {nil, nil, false}, fn from ->
          with {number, _code, _tag} <- Store.latest("#{from}/plugins/#{name}"),
               do: {"#{from}/plugins/#{name}", number, false}
        end)
      end

    title = with {:ok, version} <- described(artifact, number), do: version.title, else: (_ -> nil)

    trusted =
      cond do
        deleted -> record["trusted"] != false
        artifact -> Lifecycle.trusted?(owner, artifact)
        true -> false
      end

    %{name: name, title: title || name, plugin: artifact, number: number, trusted: trusted, deleted: deleted}
  end

  defp described(nil, _number), do: :error

  defp described(artifact, nil) do
    with {number, _code, _tag} <- Store.latest(artifact), do: described(artifact, number)
  end

  defp described(artifact, number) do
    with code when is_binary(code) <- Store.code(artifact, number), do: Store.described(artifact, code)
  end

  @doc """
  The plugins installed for `owner` that `name` is required by, directly or through another, by the names they're
  installed by, nearest first: what can't work for them without it, whether or not it's there for them from the
  network too.
  """
  def required_by(owner, name), do: required_by(Plugins.versions([owner]), [name], [name], [])

  defp required_by(versions, names, seen, found) do
    case for({other, version} <- versions, other not in seen, Enum.any?(names, &is_map_key(version.requires, &1)), do: other) do
      [] -> found
      more -> required_by(versions, more, seen ++ more, found ++ more)
    end
  end
end
