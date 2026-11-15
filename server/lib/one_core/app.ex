defmodule OneCore.App do
  @moduledoc """
  Builds a plugin's app from its source, which is what's deployed: the folder `app/` of the plugin's files, an app
  written in Solid with the kit. The kit is a plugin, `ui`, and an app is built with the build the kit itself holds,
  its folder `build/` (`ui/app.js` as the kit was built), which Bun runs: so the kit an app is built against is the
  kit it loads, whichever version of it runs for whoever the app is for. What a client is served at `app/` is what
  that built. The code of the kit it was built against is kept there too, as `app/kit`, which no page loads.

  The files deployed are kept beside the plugin's code as they were (`OneCore.Plugins`), to change and deploy again.
  An app that names packages of its own in `app/package.json` has them installed as it's built; one that names none
  installs nothing.
  """
  alias OneCore.{Plugins, Server}
  alias OneCore.Plugins.Version

  @app "app/"
  @kit "app/kit"
  @build "build/"

  @doc """
  Whether a plugin's files hold an app's source to build: a page in `app/`. One whose `app/` has none was built
  already, as the kit's is.
  """
  def app?(files), do: Enum.any?(Map.keys(files), &(String.starts_with?(&1, @app) and Path.extname(&1) == ".html"))

  @doc """
  The files of a plugin with an app as a server keeps them, from the files deployed and `kit`, the version of the
  kit that runs for whoever it's deployed for. `{:error, why}` says why it didn't build: the compiler's own words, a
  problem to a line, with its file and line.
  """
  def build(files, %Version{} = kit) do
    {app, rest} = Map.split_with(files, fn {path, _} -> String.starts_with?(path, @app) end)
    folder = Path.join([Server.home(), "building", Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)])
    source = Path.join(folder, "source")
    out = Path.join(folder, "out")

    try do
      write(source, for({path, text} <- app, do: {String.replace_prefix(path, @app, ""), text}))

      with {:ok, bun} <- bun(),
           {:ok, builder} <- builder(bun, kit),
           :ok <- install(bun, source),
           :ok <- run(bun, [Path.join(builder, "app.js"), source, out], "The app didn't build", builder) do
        built =
          for path <- Path.wildcard(Path.join(out, "**"), match_dot: true), File.regular?(path), into: %{} do
            {@app <> Path.relative_to(path, out), File.read!(path)}
          end

        {:ok, rest |> Map.merge(built) |> Map.put(@kit, kit.code)}
      end
    after
      File.rm_rf!(folder)
    end
  end

  @doc "The code of the kit an app was built against, from the files a server keeps of its plugin, or `nil` for one with no app."
  def kit(kept), do: kept[@kit]

  # The kit's build, with what it builds with installed: a folder on this server for each set of those files,
  # made whole beside it and moved in, so a folder that's there is ready.
  defp builder(bun, kit) do
    files = for {path, hash} <- Enum.sort(kit.hashes), String.starts_with?(path, @build), do: {path, hash}
    folder = Path.join([Server.home(), "kits", binary_part(Version.code(files), 0, 16)])

    cond do
      files == [] ->
        {:error, "The kit, #{kit.artifact}, has no build/ to build an app with. Deploy a kit that has one."}

      File.dir?(folder) ->
        {:ok, folder}

      true ->
        writing = folder <> "." <> Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)

        try do
          write(writing, for({path, _hash} <- files, do: {String.replace_prefix(path, @build, ""), read(kit, path)}))

          with :ok <- run(bun, ~w(install --frozen-lockfile --production), "The kit's build didn't install", writing) do
            # Another build may have made it meanwhile, which is as good.
            with {:error, _} <- File.rename(writing, folder), do: :ok
            {:ok, folder}
          end
        after
          File.rm_rf!(writing)
        end
    end
  end

  defp read(kit, path) do
    {:ok, text} = Plugins.file(kit.artifact, kit.code, path)
    text
  end

  defp write(folder, files) do
    for {path, text} <- files do
      file = Path.join(folder, path)
      File.mkdir_p!(Path.dirname(file))
      File.write!(file, text)
    end
  end

  defp bun do
    case System.find_executable("bun") do
      nil ->
        {:error,
         "Building an app needs Bun, which this server can't find: run `mise install` in The One App's folder, or put bun on its PATH."}

      bun ->
        {:ok, bun}
    end
  end

  # An app's own packages, which it names in its package.json, and none for an app that names none.
  defp install(bun, source) do
    with {:ok, text} <- File.read(Path.join(source, "package.json")),
         {:ok, about} <- JSON.decode(text),
         named when map_size(named) > 0 <- Map.merge(about["dependencies"] || %{}, about["devDependencies"] || %{}) do
      run(bun, ["install"], "The app's packages didn't install", source)
    else
      {:error, %JSON.DecodeError{}} -> {:error, "app/package.json isn't JSON."}
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
