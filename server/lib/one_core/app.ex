defmodule OneCore.App do
  @moduledoc """
  Builds a plugin's app from its source, which is what's deployed: the folder `app/` of the plugin's files, an app
  written in Solid with the kit. A server builds it with the kit's own build (`ui/app.js`), which Bun runs, and
  what a client is served at `app/` is what that built. The code of the kit it was built against is kept there too,
  as `app/kit`, which no page loads.
  The files deployed are kept beside the plugin's code as they were (`OneCore.Plugins`), to change and deploy again.

  The kit's folder on this server, with what it builds with installed, is its `:kit` setting. An app that names
  packages of its own in `app/package.json` has them installed as it's built; one that names none installs
  nothing.
  """
  alias OneCore.Server

  @app "app/"
  @kit "app/kit"

  @doc """
  Whether a plugin's files hold an app's source to build: a page in `app/`. One whose `app/` has none was built
  already, as the kit's is.
  """
  def app?(files), do: Enum.any?(Map.keys(files), &(String.starts_with?(&1, @app) and Path.extname(&1) == ".html"))

  @doc """
  The files of a plugin with an app as a server keeps them, from the files deployed and the code of the kit that's
  running for whoever it's deployed for, `kit`. `{:error, why}` says why it didn't build: the compiler's own words,
  a problem to a line, with its file and line.
  """
  def build(files, kit) do
    {app, rest} = Map.split_with(files, fn {path, _} -> String.starts_with?(path, @app) end)
    folder = Path.join([Server.home(), "building", Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)])
    source = Path.join(folder, "source")
    out = Path.join(folder, "out")

    try do
      for {path, text} <- app do
        file = Path.join(source, String.replace_prefix(path, @app, ""))
        File.mkdir_p!(Path.dirname(file))
        File.write!(file, text)
      end

      with {:ok, bun} <- bun(),
           :ok <- install(bun, source),
           :ok <- run(bun, [Path.join(kit_folder(), "app.js"), source, out], "The app didn't build") do
        built =
          for path <- Path.wildcard(Path.join(out, "**"), match_dot: true), File.regular?(path), into: %{} do
            {@app <> Path.relative_to(path, out), File.read!(path)}
          end

        {:ok, rest |> Map.merge(built) |> Map.put(@kit, kit)}
      end
    after
      File.rm_rf!(folder)
    end
  end

  @doc "The code of the kit an app was built against, from the files a server keeps of its plugin, or `nil` for one with no app."
  def kit(kept), do: kept[@kit]

  defp kit_folder, do: Application.fetch_env!(:one_core, :kit)

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

  defp run(bun, args, failed, folder \\ nil) do
    case System.cmd(bun, args, stderr_to_stdout: true, cd: folder || kit_folder()) do
      {_, 0} -> :ok
      {said, _} -> {:error, "#{failed}:\n#{String.trim(said)}"}
    end
  end
end
