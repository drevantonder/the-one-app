defmodule OneCore.Apps.Requires do
  @moduledoc """
  What an app `requires` (`requires/0`, `OneCore.App`): the other apps it needs, by name, each with why, which
  are its requirements; and the other way, the apps it's `required_by`. A requirement is there for an owner where it
  runs for them: installed for them, or for a scope they're in, the network.

  These only say what's what; `OneCore.Apps.Lifecycle` makes the moves.
  """
  alias OneCore.Apps
  alias OneCore.Apps.{Installed, Store}

  @network "network"

  @doc """
  What the app `artifact` requires, as its version described itself: each requirement's name and why,
  `%{"google-account" => "to reach your mail"}`. None where no server has described it yet.
  """
  def requires(artifact) do
    with {:ok, version} <- described(artifact), do: version.requires, else: (_ -> %{})
  end

  @doc """
  The requirements of the app `artifact`, as `requires` says them, that aren't there for `owner`, and theirs in
  turn, deepest first, so each comes before what requires it. Each is

      %{name: "google-account", title: "Google", by: "gmail", why: "to reach your mail",
        app: "network/apps/google-account", deleted: false}

  `by` is the app that requires it, and `why` its reason. `app` is where to get it from: the owner's Recently
  Deleted (`deleted`, restored there first), their own app of that name, that of whoever's app requires it, or the
  network's; `nil` where it's none of these, so it can't be installed.
  """
  def missing(owner, artifact, requires), do: found(owner, artifact, requires, [Path.basename(artifact)], [])

  defp found(owner, artifact, requires, seen, found) do
    there = Apps.running(owner)
    by = Path.basename(artifact)

    Enum.reduce(Enum.sort(requires), found, fn {name, why}, found ->
      if is_map_key(there, name) or name in seen or Enum.any?(found, &(&1.name == name)) do
        found
      else
        requirement = Map.merge(source(owner, name, Store.whose(artifact)), %{by: by, why: why})

        found =
          if requirement.app,
            do: found(owner, requirement.app, requires(requirement.app), [name | seen], found),
            else: found

        found ++ [requirement]
      end
    end)
  end

  defp source(owner, name, whose) do
    record = Installed.read(owner)[name]

    {artifact, deleted} =
      if match?(%{"state" => "recently_deleted"}, record) do
        {record["app"], true}
      else
        Enum.find_value(Enum.uniq([owner, whose, @network]), {nil, false}, fn from ->
          if Store.latest("#{from}/apps/#{name}"), do: {"#{from}/apps/#{name}", false}
        end)
      end

    title = with {:ok, version} <- described(artifact), do: version.title, else: (_ -> nil)
    %{name: name, title: title || name, app: artifact, deleted: deleted}
  end

  defp described(nil), do: :error

  defp described(artifact) do
    with {_number, code, _tag} <- Store.latest(artifact), do: Store.described(artifact, code), else: (_ -> :error)
  end

  @doc """
  The apps installed for `owner` that `name` is required by, directly or through another, by the names they're
  installed by, nearest first: what can't work for them without it, whether or not it's there for them from the
  network too.
  """
  def required_by(owner, name), do: required_by(Apps.versions([owner]), [name], [name], [])

  defp required_by(versions, names, seen, found) do
    case for({other, version} <- versions, other not in seen, Enum.any?(names, &is_map_key(version.requires, &1)), do: other) do
      [] -> found
      more -> required_by(versions, more, seen ++ more, found ++ more)
    end
  end
end
