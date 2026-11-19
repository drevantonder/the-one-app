defmodule OneCore.Entrypoint do
  @moduledoc """
  Builds an app's entrypoints from their source, which is what's deployed: the folder `client/` of the app's files, apps
  written in Solid with the UI library. The UI library is One Core's own (`core/ui`), and a server ships with its build,
  its `:ui` setting, as an app ships with the framework it's written in: a server's build is what changes it. An app is
  built with the build the UI library holds, its folder `build/` (`core/ui/app.js`), which Bun runs, so the UI library
  an app is built against is the one its server serves it with. What a client is served at `client/` is what that
  built. The code of the UI library it was built against is kept there too, as `client/ui`, which no page loads.

  The files deployed are kept beside the app's code as they were (`OneCore.Apps`), to change and deploy again.
  An app that names packages of its own in `client/package.json` has them installed as it's built; one that names none
  installs nothing.
  """
  alias OneCore.{Server, Skills}
  alias OneCore.Apps.Version

  @client "client/"
  @ui "client/ui"
  @build "build/"

  @doc """
  The UI library this server ships with, as it's built: its `files` by their paths, its `code`, which names them, and
  its `skills`. A client is served its `client/` (`OneCore.Mcp`), and agents its skills (`OneCore.Skills`).
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
  Whether an app's files hold a client's source to build: a page in `client/`, or a module its `client/package.json`
  exports for another app's page. One whose `client/` has neither was built already.
  """
  def client?(files) do
    Enum.any?(Map.keys(files), &(String.starts_with?(&1, @client) and Path.extname(&1) == ".html")) or
      match?({:ok, %{"exports" => %{}}}, JSON.decode(files["client/package.json"] || ""))
  end

  # What Bun reads to install from somewhere else than the registry's own, named by a package.json.
  @own_registry [".npmrc", "bunfig.toml", "bun.lock", "bun.lockb"]

  @doc """
  The files of an app with an app as a server keeps them, from the files deployed, built against this server's UI
  library. `{:error, why}` says why it didn't build: the compiler's own words, a problem to a line, with its file and
  line.

  An untrusted app is a stranger's, whose app's build could otherwise read the server's files (an import of any
  path) or run its own code. So unless it's `trusted`, its packages install without their scripts, and it's built in a
  sandbox (`OneCore.Sandbox.run/3`), which reads only Bun, the UI library's build and the app.
  """
  def build(files, trusted) do
    {app, rest} = Map.split_with(files, fn {path, _} -> String.starts_with?(path, @client) end)
    folder = Path.join([Server.home(), "building", Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)])
    source = Path.join(folder, "source")
    out = Path.join(folder, "out")

    try do
      write(source, client(app, trusted))

      with {:ok, bun} <- bun(),
           {:ok, builder} <- builder(bun),
           :ok <- install(bun, source, trusted),
           :ok <- built(bun, builder, source, out, trusted) do
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

  @doc """
  Type checks an app's page against the UI library alone where one of the `version`'s entrypoints or tools' pages says its `runtime_version`, since a native
  host draws it itself and an element of the DOM's or a Web API would break it only there (`core/ui/lint-app.js`,
  which the UI library's build holds). `{:ok, warnings}`, its warnings a line each (a raw colour or length in a style,
  which should be a token), or `{:error, why}` with the checker's own errors, a line each: file, line, column and
  message, as a compiler's. It runs on the files as they were deployed, as `build/2` does, and in the sandbox where the
  app isn't `trusted`.
  """
  def lint(files, version, trusted) do
    if client?(files) and Enum.any?(version.entrypoints ++ version.tools, & &1[:runtime_version]),
      do: type_check(files, trusted),
      else: {:ok, ""}
  end

  # The app's client files, by their paths in `client/`, as Bun installs and builds them. An untrusted app's packages
  # come from the registry as its package.json names them: a registry, a lockfile or a URL of its own would have this
  # server fetch whatever the app says, as an internal address.
  defp client(files, trusted) do
    for {path, text} <- files,
        String.starts_with?(path, @client),
        path = String.replace_prefix(path, @client, ""),
        trusted or path not in @own_registry,
        do: {path, text}
  end

  defp type_check(files, trusted) do
    folder = Path.join([Server.home(), "building", Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)])
    source = Path.join(folder, "source")

    try do
      write(source, client(files, trusted))

      with {:ok, bun} <- bun(),
           {:ok, builder} <- builder(bun),
           :ok <- install(bun, source, trusted) do
        # Against the UI library's types, which are beside the checker where a server holds it, and not where the
        # app's own tsconfig.json says (its editor's, which isn't beside it here).
        config = %{extends: Path.join(builder, "tsconfig.app.json"), include: ["**/*"]}
        File.write!(Path.join(source, "tsconfig.json"), JSON.encode!(config))
        checked(bun, builder, source, trusted)
      end
    after
      File.rm_rf!(folder)
    end
  end

  defp checked(bun, builder, source, true) do
    case System.cmd(bun, [Path.join(builder, "lint-app.js"), source], stderr_to_stdout: true, env: runtime()) do
      {said, 0} -> {:ok, String.trim(said)}
      {said, _} -> {:error, not_checked(said)}
    end
  end

  defp checked(bun, builder, source, false) do
    bun = real(bun)
    policy = %{read: [Path.dirname(bun), builder, source], write: [], env: %{"HOME" => "/tmp", "NO_COLOR" => "1"}, memory: nil}

    case OneCore.Sandbox.run(bun, [Path.join(builder, "lint-app.js"), source], policy) do
      {:error, why} -> {:error, why}
      {said, 0} -> {:ok, String.trim(said)}
      {said, _} -> {:error, not_checked(said)}
    end
  end

  defp not_checked(said), do: "The app's page uses what the UI library doesn't have, where its entrypoint's runtime_version says it has only that:\n#{String.trim(said)}"

  @doc "The code of the UI library an app was built against, from the files a server keeps of its app, or `nil` for one with no app."
  def ui(settings), do: settings[@ui]

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

  @doc "The Bun an app is built with: `OneCore.Programs`."
  def bun, do: OneCore.Programs.path(:bun)

  # An app's own packages, which it names in its package.json, and none for an app that names none.
  defp install(bun, source, trusted) do
    with {:ok, text} <- File.read(Path.join(source, "package.json")),
         {:ok, about} <- JSON.decode(text),
         named when map_size(named) > 0 <- Map.merge(about["dependencies"] || %{}, about["devDependencies"] || %{}),
         :ok <- if(trusted, do: :ok, else: from_registry(about)) do
      scripts = if trusted, do: [], else: ["--ignore-scripts"]
      run(bun, ["install" | scripts], "The app's packages didn't install", source)
    else
      {:error, %JSON.DecodeError{}} -> {:error, "client/package.json isn't JSON."}
      {:error, why} when is_binary(why) -> {:error, why}
      _ -> :ok
    end
  end

  # What an untrusted app asks of the registry: a version, a range, a tag or `npm:` another package, and nothing that
  # names a URL, a git repo or a file (`https:`, `github:`, `file:`), which this server would fetch or read.
  defp from_registry(about) do
    wanted =
      for field <- ~w(dependencies devDependencies optionalDependencies peerDependencies overrides resolutions),
          {name, spec} <- if(is_map(about[field]), do: about[field], else: %{}),
          not (is_binary(spec) and spec =~ ~r/^(?:npm:[@\w.\/^~<>=*| +-]*|[\w.^~<>=*| +-]*)$/),
          do: "#{name}: #{inspect(spec)}"

    case wanted do
      [] -> :ok
      _ -> {:error, "An untrusted app's packages come from the registry only, as a version, a range or a tag. These don't: #{Enum.join(wanted, ", ")}."}
    end
  end

  defp built(bun, builder, source, out, true),
    do: run(bun, [Path.join(builder, "app.js"), source, out], "The app didn't build", builder)

  defp built(bun, builder, source, out, false) do
    File.mkdir_p!(out)
    bun = real(bun)
    policy = %{read: [Path.dirname(bun), builder], write: [source, out], env: %{"HOME" => "/tmp", "NO_COLOR" => "1"}, memory: nil}

    case OneCore.Sandbox.run(bun, [Path.join(builder, "app.js"), source, out], policy) do
      {:error, why} -> {:error, why}
      {_, 0} -> :ok
      {said, _} -> {:error, "The app didn't build:\n#{String.trim(said)}"}
    end
  end

  # A path with each symlink in it followed, as a folder bound into the sandbox has to be.
  defp real(path) do
    path
    |> Path.split()
    |> Enum.reduce(fn part, at ->
      next = Path.join(at, part)

      case :file.read_link_all(next) do
        {:ok, target} -> real(Path.expand(to_string(target), at))
        _ -> next
      end
    end)
  end

  defp run(bun, args, failed, folder) do
    case System.cmd(bun, args, stderr_to_stdout: true, cd: folder, env: runtime()) do
      {_, 0} -> :ok
      {said, _} -> {:error, "#{failed}:\n#{String.trim(said)}"}
    end
  end

  # The Solid compiler is a native Windows DLL Bun loads, which needs the Visual C++ runtime, and a PC may not have it:
  # its DLLs are the Erlang VM's, beside it (`tooling/release`), so Bun looks in the VM's folder too.
  defp runtime do
    case :os.type() do
      {:win32, _} ->
        erts = Path.join([:code.root_dir(), "erts-#{:erlang.system_info(:version)}", "bin"])
        [{"PATH", String.replace(erts, "/", "\\") <> ";" <> System.get_env("PATH", "")}]

      _ ->
        []
    end
  end
end
