defmodule OneCore.App do
  @moduledoc """
  Builds a plugin's apps from their source, which is what's deployed: the folder `client/` of the plugin's files, apps
  written in Solid with the UI library. The UI library is One Core's own (`core/ui`), and a server ships with its build,
  its `:ui` setting, as an app ships with the framework it's written in: a server's build is what changes it. An app is
  built with the build the UI library holds, its folder `build/` (`core/ui/app.js`), which Bun runs, so the UI library
  an app is built against is the one its server serves it with. What a client is served at `client/` is what that
  built. The code of the UI library it was built against is kept there too, as `client/ui`, which no page loads.

  The files deployed are kept beside the plugin's code as they were (`OneCore.Plugins`), to change and deploy again.
  An app that names packages of its own in `client/package.json` has them installed as it's built; one that names none
  installs nothing.
  """
  alias OneCore.{Server, Skills}
  alias OneCore.Plugins.Version

  @client "client/"
  @ui "client/ui"
  @build "build/"

  @doc """
  The UI library this server ships with, as it's built: its `files` by their paths, its `code`, which names them, and
  its `skills`. A client is served its `client/` (`OneCore.MCP`), and agents its skills (`OneCore.Skills`).
  """
  def library do
    with nil <- :persistent_term.get({__MODULE__, :library}, nil) do
      folder = Application.fetch_env!(:one_core, :ui)

      files =
        for path <- Path.wildcard(Path.join(folder, "**"), match_dot: true), File.regular?(path), into: %{} do
          {Path.relative_to(path, folder), File.read!(path)}
        end

      skills = Map.filter(files, fn {path, _} -> path == "SKILL.md" or String.starts_with?(path, "skills/") end)
      {:ok, skills} = Skills.found("ui", skills)
      library = %{files: files, code: Version.code(files), skills: skills}
      :persistent_term.put({__MODULE__, :library}, library)
      library
    end
  end

  @doc """
  Whether a plugin's files hold an app's source to build: a page in `client/`. One whose `client/` has none was built
  already.
  """
  def app?(files), do: Enum.any?(Map.keys(files), &(String.starts_with?(&1, @client) and Path.extname(&1) == ".html"))

  @doc """
  The files of a plugin with an app as a server keeps them, from the files deployed, built against this server's UI
  library. `{:error, why}` says why it didn't build: the compiler's own words, a problem to a line, with its file and
  line.
  """
  def build(files) do
    {app, rest} = Map.split_with(files, fn {path, _} -> String.starts_with?(path, @client) end)
    folder = Path.join([Server.home(), "building", Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)])
    source = Path.join(folder, "source")
    out = Path.join(folder, "out")

    try do
      write(source, for({path, text} <- app, do: {String.replace_prefix(path, @client, ""), text}))

      with {:ok, bun} <- bun(),
           {:ok, builder} <- builder(bun),
           :ok <- install(bun, source),
           :ok <- run(bun, [Path.join(builder, "app.js"), source, out], "The app didn't build", builder) do
        built =
          for path <- Path.wildcard(Path.join(out, "**"), match_dot: true), File.regular?(path), into: %{} do
            {@client <> Path.relative_to(path, out), File.read!(path)}
          end

        # What the build makes nothing of, as an app's icon, is served as it was deployed: its pictures, sounds,
        # videos and fonts.
        as_is =
          Map.filter(app, fn {path, _} -> String.starts_with?(MIME.from_path(path), ~w(image/ audio/ video/ font/)) end)

        {:ok, rest |> Map.merge(as_is) |> Map.merge(built) |> Map.put(@ui, library().code)}
      end
    after
      File.rm_rf!(folder)
    end
  end

  @doc "The code of the UI library an app was built against, from the files a server keeps of its plugin, or `nil` for one with no app."
  def ui(kept), do: kept[@ui]

  # The UI library's build, with what it builds with installed: its folder, made whole beside it and moved in, so a
  # folder that's there is ready. Those an earlier server build made go as this one's is made.
  defp builder(bun) do
    files = for {path, text} <- library().files, String.starts_with?(path, @build), into: %{}, do: {path, text}
    builds = Path.join(Server.home(), "ui-builds")
    folder = Path.join(builds, binary_part(Version.code(files), 0, 16))

    if File.dir?(folder) do
      {:ok, folder}
    else
      writing = folder <> "." <> Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)

      try do
        write(writing, for({path, text} <- files, do: {String.replace_prefix(path, @build, ""), text}))

        with :ok <- run(bun, ~w(install --frozen-lockfile --production), "The UI library's build didn't install", writing) do
          # Another build may have made it meanwhile, which is as good.
          with {:error, _} <- File.rename(writing, folder), do: :ok
          for name <- File.ls!(builds), not String.contains?(name, "."), name != Path.basename(folder),
              do: File.rm_rf!(Path.join(builds, name))

          {:ok, folder}
        end
      after
        File.rm_rf!(writing)
      end
    end
  end

  defp write(folder, files) do
    for {path, text} <- files do
      file = Path.join(folder, path)
      File.mkdir_p!(Path.dirname(file))
      File.write!(file, text)
    end
  end

  # The Bun an app is built with: an installed server's is the one its release names, `{url, sha256}`, which it
  # downloads the first time and keeps in its home, so an update downloads no Bun of its own. One started with
  # `mix network` uses the Bun on its PATH, as the rest of the repo does.
  defp bun do
    case Application.get_env(:one_core, :bun) do
      {url, sha256} -> fetched(url, sha256)
      nil -> path()
    end
  end

  defp path do
    case System.find_executable("bun") do
      nil ->
        {:error,
         "Building an app needs Bun, which this server can't find: run `mise install` in The One App's folder, or put bun on its PATH."}

      bun ->
        {:ok, bun}
    end
  end

  defp fetched(url, sha256) do
    folder = Path.join([Server.home(), "bun", sha256])
    bun = Path.join(folder, "bun")

    with false <- File.exists?(bun),
         {:ok, %{status: 200, body: zip}} <- Req.get(url, raw: true),
         ^sha256 <- Base.encode16(:crypto.hash(:sha256, zip), case: :lower),
         {:ok, files} <- :zip.extract(zip, [:memory]),
         {_name, program} <- Enum.find(files, fn {name, _} -> Path.basename(to_string(name)) == "bun" end) do
      writing = folder <> "." <> Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)
      File.mkdir_p!(writing)
      File.write!(Path.join(writing, "bun"), program)
      File.chmod!(Path.join(writing, "bun"), 0o755)
      # Another build may have fetched it meanwhile, which is as good.
      with {:error, _} <- File.rename(writing, folder), do: File.rm_rf!(writing)
      {:ok, bun}
    else
      true -> {:ok, bun}
      _ -> {:error, "Building an app needs Bun, which this server couldn't download whole from #{url}."}
    end
  end

  # An app's own packages, which it names in its package.json, and none for an app that names none.
  defp install(bun, source) do
    with {:ok, text} <- File.read(Path.join(source, "package.json")),
         {:ok, about} <- JSON.decode(text),
         named when map_size(named) > 0 <- Map.merge(about["dependencies"] || %{}, about["devDependencies"] || %{}) do
      run(bun, ["install"], "The app's packages didn't install", source)
    else
      {:error, %JSON.DecodeError{}} -> {:error, "client/package.json isn't JSON."}
      _ -> :ok
    end
  end

  defp run(bun, args, failed, folder) do
    case System.cmd(bun, args, stderr_to_stdout: true, cd: folder) do
      {_, 0} -> :ok
      {said, _} -> {:error, "#{failed}:\n#{String.trim(said)}"}
    end
  end
end
