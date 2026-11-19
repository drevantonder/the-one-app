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
  alias OneCore.App.Manifest
  alias OneCore.Apps.{LoadedVersion, Version}
  alias OneCore.Apps.Version.Checks

  # What a server keeps of a version besides what its app said: the version's own fields, and of what it keeps as
  # it's held, its files a page loads and its skills.
  @kept [:exports, :services, :code, :ui, :title, :icon, :core]

  @doc """
  What a loaded version says of itself, as the bucket keeps it: its manifest's JSON form (`OneCore.App.Manifest`)
  as checked, what it provides besides, and its files and skills, but none of its code.
  """
  def description(%Version{under: under} = version) do
    kept = LoadedVersion.kept(under)

    tools =
      for tool <- version.tools,
          do: Map.take(tool, [:name, :description, :inputs, :outputs, :read_only, :by_link, :ui, :runtime_version, :task_support])

    version
    |> Map.take(Manifest.said() ++ @kept)
    |> Map.merge(%{tools: tools, files: kept.files, skills: kept.skills})
    |> JSON.encode!()
  end

  @doc """
  A version of the app `artifact` from what it said of itself (`description/1`), and what it keeps as one held
  (`OneCore.Apps.LoadedVersion.described/2`): `{:ok, version, kept}`, or `:error` where that isn't one. It was written
  by a server that runs the app, whoever's that is, so only what's listed of it is taken, and what each tool takes
  and gives is read from its inputs and outputs again.
  """
  def described(artifact, description) do
    with {:ok, %{} = json} <- JSON.decode(description) do
      said = Manifest.decode(json)
      fields = for field <- @kept, into: %{}, do: {field, json["#{field}"]}

      services =
        for service <- json["services"] || [],
            do: %{name: service["name"], called: service["called"], needs: service["needs"]}

      under = {:described, artifact, json["code"]}
      tools = Enum.map(said.tools, &Version.schemas/1)

      version =
        struct!(
          Version,
          said
          |> Map.merge(fields)
          |> Map.merge(%{under: under, app: nil, tools: tools, services: services, artifact: artifact})
        )

      {:ok, version, %{files: json["files"], hashes: %{}, skills: Enum.map(json["skills"], &skill/1), own: %{}}}
    else
      _ -> :error
    end
  end

  # A skill as `OneCore.Skills.found/2` gives it, from its JSON form.
  defp skill(skill) do
    files = for file <- skill["files"], do: %{path: file["path"], digest: file["digest"], size: file["size"]}
    %{at: skill["at"], frontmatter: skill["frontmatter"], digest: skill["digest"], size: skill["size"], files: files}
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

    description = Manifest.decode(description)

    with {:ok, skills} <- Skills.found(name, skills),
         {:ok, version} <- Checks.checked(under, nil, description, [], rest, skills) do
      version = %{
        version
        | code: code,
          ui: Entrypoint.ui(files),
          artifact: artifact,
          trusted: false
      }

      served = Enum.sort(for path <- Map.keys(rest), Version.loaded?(path), do: path)
      :ok = LoadedVersion.described(version, %{files: served, hashes: %{}, skills: skills, own: %{}})
      {:ok, version}
    end
  end
end
