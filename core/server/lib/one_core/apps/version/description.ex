defmodule OneCore.Apps.Version.Description do
  @moduledoc """
  What a version says of itself, without its code. A server that doesn't run an app for whoever it's installed for
  never loads its code, which runs with all of its machine's access: it knows the version by what the version said
  of itself where it was loaded (`description/1`), which that server kept in the bucket, and it's `described/2` from
  that. An untrusted version's code never loads on a server, so it's described from what its code said of itself in
  its sandbox (`untrusted/4`). A described version has no modules, so it's only listed here, and every call to it is
  run where it's loaded, or in its sandbox (`OneCore.Apps.Calls`).

  Each is held as `OneCore.Apps.LoadedVersion` holds a version, by its app and its code.
  """
  alias OneCore.{Entrypoint, Skills}
  alias OneCore.Apps.{LoadedVersion, Version}
  alias OneCore.Apps.Version.Checks

  @doc """
  What a loaded version says of itself, as the bucket keeps it: what it provides, and its files and skills, but
  none of its code.
  """
  def description(%Version{under: under} = version) do
    kept = LoadedVersion.kept(under)

    description =
      Map.take(version, [
        :tools,
        :entrypoints,
        :widgets,
        :exports,
        :services,
        :hooks,
        :offers,
        :jobs,
        :code,
        :ui,
        :title,
        :icon,
        :requires,
        :core
      ])

    # It holds only names any server knows: a tool's inputs as JSON has them, not the atoms its app wrote them
    # with, which a server that never loaded the app doesn't have, and won't take from the bucket.
    tools =
      for tool <- description.tools do
        tool
        |> Map.take([:name, :description, :inputs, :outputs, :read_only, :by_link, :ui, :task_support])
        |> Map.update!(:inputs, &JSON.decode!(JSON.encode!(&1)))
        |> Map.replace_lazy(:outputs, &JSON.decode!(JSON.encode!(&1)))
      end

    :erlang.term_to_binary(%{description | tools: tools} |> Map.merge(%{files: kept.files, skills: kept.skills}))
  end

  @doc """
  A version of the app `artifact` from what it said of itself (`description/1`), and what it keeps as one held
  (`OneCore.Apps.LoadedVersion.described/2`): `{:ok, version, kept}`, or `:error` where that isn't one. It was written by a server that runs the app, whoever's that is, so only
  what's listed of it is taken, and what each tool takes and gives is read from its inputs and outputs again.
  """
  def described(artifact, description) do
    # Its skills are as `OneCore.Skills` says them, whose names a server that has just started may not know yet.
    Code.ensure_loaded!(Skills)
    description = :erlang.binary_to_term(description, [:safe])

    fields =
      Map.take(description, [
        :entrypoints,
        :widgets,
        :exports,
        :services,
        :hooks,
        :offers,
        :jobs,
        :code,
        :ui,
        :title,
        :icon,
        :requires,
        :core
      ])

    under = {:described, artifact, description.code}
    tools = Enum.map(description.tools, &Version.schemas/1)
    version = struct!(Version, Map.merge(fields, %{under: under, app: nil, tools: tools, artifact: artifact}))
    {:ok, version, %{files: description.files, hashes: %{}, skills: description.skills, own: %{}}}
  rescue
    _ -> :error
  end

  @doc """
  A version of the untrusted app `artifact`, from its files as a server keeps them and what its code said of
  itself in its sandbox (`OneCore.Sandbox`), held by the caller: `{:ok, version}`, or `{:error, why}`. It's described as a
  version a server doesn't run is, and checked as one that loads is.
  """
  def untrusted(artifact, files, description) do
    name = Path.basename(artifact)
    code = Version.code(files)
    under = {:described, artifact, code}
    rest = Map.reject(files, fn {path, _} -> Version.server?(path) and Path.extname(path) == ".ex" end)

    {skills, rest} =
      Map.split_with(rest, fn {path, _} -> path == "SKILL.md" or String.starts_with?(path, "skills/") end)

    taken = fn list, keys ->
      for item <- List.wrap(list),
          is_map(item),
          do: Map.new(Map.take(item, keys), &{String.to_existing_atom(elem(&1, 0)), elem(&1, 1)})
    end

    offers =
      for offer <- taken.(description["offers"], ~w(name kind description)),
          do: Map.update(offer, :kind, nil, &hook_kind/1)

    description = %{
      tools:
        for(
          tool <- taken.(description["tools"], ~w(name description inputs outputs read_only by_link ui task_support)),
          do: Map.replace_lazy(tool, :task_support, &task_support/1)
        ),
      entrypoints: taken.(description["entrypoints"], ~w(name page icon category subject)),
      title: description["title"],
      icon: description["icon"],
      widgets: taken.(description["widgets"], ~w(name tool sizes)),
      exports: description["exports"],
      hooks: description["hooks"],
      offers: offers,
      jobs: taken.(description["jobs"], ~w(tool every inputs)),
      requires: description["requires"]
    }

    with {:ok, skills} <- Skills.found(name, skills),
         {:ok, version} <- Checks.checked(under, nil, description, [], rest) do
      version = %{
        version
        | code: code,
          ui: Entrypoint.ui(files),
          title: version.title || name,
          artifact: artifact,
          trusted: false
      }

      served = Enum.sort(for path <- Map.keys(rest), Version.loaded?(path), do: path)
      :ok = LoadedVersion.described(version, %{files: served, hashes: %{}, skills: skills, own: %{}})
      {:ok, version}
    end
  end

  defp hook_kind(kind) when kind in ["observe", "mutate", "validate", "collect"], do: String.to_existing_atom(kind)
  defp hook_kind(_kind), do: nil

  # An untrusted app's code said its tools in JSON, where an atom is text.
  defp task_support("optional"), do: :optional
  defp task_support("required"), do: :required
  defp task_support(other), do: other
end
