# The code reloader of a network on this machine, as Phoenix's code reloader is in development, which
# `tooling/network` starts beside its servers, from `core/server/`:
#
#     mix run --no-start --no-halt ../../tooling/code_reloader.exs <folder>
#
# It's a script rather than part of One Core's code, which it compiles and loads: compiling itself would end it.
defmodule CodeReloader do
  @moduledoc """
  A server started with `--reloader <its node>` (`mix network`) asks it before it answers a client's request
  (`OneCore.Mcp`), so the request sees what changed in this checkout since the last:

  - The Rust workspace, where a file of it changed, is built again as `mix compile` builds it (`nifs/1` in
    `core/server/mix.exs`): `one`, and the NIFs, of One Core and of this repo's apps, each put where it's loaded from.
  - One Core's code is compiled here and its modules that changed loaded into every server, as Phoenix's code
    reloader loads what Mix compiled: a process carries on in the new code from its next call into its module, and
    what only starting builds again, as a supervisor's children, is the code before until the network starts again.
    One Core's NIFs aren't loaded: where one changed, the request is refused, saying the network starts again for it.
  - The UI library (`core/ui`), where a file of it changed, is built again and is the one every server serves, as a
    release's is (`OneCore.Apps.Deploy.ui/1`).
  - An app of this repo whose files changed is deployed for the network again, as `one deploy --files-from` deploys
    it. A request is refused with why it didn't deploy.

  The deps and the config are built as the network starts again. Its compilers' output is its log.
  """
  use GenServer
  alias OneCore.Folder
  alias OneCore.Apps.Deploy

  @root Path.expand("..", __DIR__)
  @build Path.expand("../core/server/_build/dev/lib/one_core", __DIR__)

  def start_link(folder), do: GenServer.start_link(__MODULE__, Path.join(folder, "reloader"), name: __MODULE__)

  @doc "Has the server whose node is `server` reloaded from now on: what it runs is what this has compiled."
  def serving(server), do: GenServer.call(__MODULE__, {:serving, server}, :infinity)

  @doc "Loads what changed into every server, as `server` asks before a request: `:ok`, or `{:error, why}`."
  def reload(server), do: GenServer.call(__MODULE__, {:reload, server}, :infinity)

  @impl GenServer
  def init(copy), do: {:ok, %{copy: copy, servers: [], rust: nil, core: nil, natives: nil, ui: nil, apps: nil}}

  # What the servers run is what's compiled when the first starts, so that's what a change is told from. The apps are
  # what was last deployed, which the network's folder keeps (`kept/1`): a network started again in the same folder,
  # after the checkout moved on, deploys each that changed meanwhile at its first request, rather than running what
  # it was first deployed with (19 Nov: a Vault from before a fix, which a delegate took for the bug still there).
  @impl GenServer
  def handle_call({:serving, server}, _from, %{core: nil} = state) do
    copy(state)
    rust = rust()
    apps = kept(state) || keep(state, apps())
    state = %{state | rust: rust, core: core(rust), natives: natives(), ui: ui(), apps: apps}
    handle_call({:serving, server}, nil, state)
  end

  def handle_call({:serving, server}, _from, state), do: {:reply, :ok, %{state | servers: [server | state.servers]}}

  # What failed is the request's answer, and the next request tries again.
  def handle_call({:reload, server}, _from, state) do
    rust = rust()

    with {:ok, state} <- built(rust, state),
         {:ok, state} <- reloaded(core(rust), state),
         {:ok, state} <- ui_built(ui(), state),
         {:ok, state} <- deployed(apps(), server, state) do
      {:reply, :ok, state}
    else
      {:error, why, state} -> {:reply, {:error, why}, state}
    end
  catch
    kind, why -> {:reply, {:error, "The code reloader failed: " <> Exception.format(kind, why, __STACKTRACE__)}, state}
  end

  defp built(rust, %{rust: rust} = state), do: {:ok, state}

  defp built(rust, state) do
    Mix.Project.get!().nifs([])
    {:ok, %{state | rust: rust}}
  rescue
    error in Mix.Error ->
      {:error, "The Rust workspace didn't build: #{Exception.message(error)}, in #{state.copy}.log", state}
  end

  defp reloaded(core, %{core: core} = state), do: {:ok, state}

  defp reloaded(core, state) do
    began = System.monotonic_time(:millisecond)

    with {:compiled, {compiled, _diagnostics}} when compiled in [:ok, :noop] <- {:compiled, compiled()},
         {:natives, natives} when natives == state.natives <- {:natives, natives()},
         {removed, changed} when removed != [] or changed != [] <- changes(state.copy),
         [] <- for(server <- state.servers, (why = load(server, removed, changed)) != :ok, do: "#{server}: #{why}") do
      copy(state)
      took = System.monotonic_time(:millisecond) - began
      loaded = removed ++ Enum.map(changed, &elem(&1, 0))
      Mix.shell().info("Loaded #{Enum.map_join(loaded, ", ", &inspect/1)} in #{took} ms")
      {:ok, %{state | core: core}}
    else
      {:compiled, {:error, diagnostics}} ->
        {:error, "One Core didn't compile:\n" <> Enum.map_join(diagnostics, "\n", &diagnostic/1), state}

      {:natives, _natives} ->
        {:error, "One Core's NIFs changed: stop tooling/network and start it again in the same folder.", state}

      {[], []} ->
        {:ok, %{state | core: core}}

      failed ->
        {:error, "One Core's change didn't load: #{Enum.join(failed, "; ")}", state}
    end
  end

  # One Core's modules that differ from the copy of what the servers run, with its consolidated protocols, by each
  # module's MD5 (`:beam_lib.md5/1`): `{removed, changed}`, each changed one as `:code.atomic_load/1` takes it.
  defp changes(copy) do
    [was, now] = for dir <- [copy, @build], do: beams(dir)

    changed = for {module, {md5, file}} <- now, not match?({^md5, _}, was[module]), do: {module, file, File.read!(file)}
    {Map.keys(Map.drop(was, Map.keys(now))), changed}
  end

  defp beams(dir) do
    for file <- Path.wildcard(Path.join(dir, "{ebin,consolidated}/*.beam")), into: %{} do
      {:ok, {module, md5}} = :beam_lib.md5(to_charlist(file))
      {module, {md5, to_charlist(file)}}
    end
  end

  # Loads `changed` into the server whose node is `server`, and deletes `removed`, as IEx's `r/1` reloads a module:
  # what runs the code before the one before is ended as it's purged.
  defp load(server, removed, changed) do
    for module <- removed ++ Enum.map(changed, &elem(&1, 0)), do: :erpc.call(server, :code, :purge, [module])

    case :erpc.call(server, :code, :atomic_load, [changed]) do
      :ok -> Enum.each(removed, &:erpc.call(server, :code, :delete, [&1]))
      {:error, why} -> inspect(why)
    end
  catch
    kind, why -> Exception.format(kind, why)
  end

  # One Core's NIFs as built, by their contents: a server goes on with the build of each it loaded.
  defp natives, do: hashes(Path.wildcard(Path.join(@build, "priv/native/*")))

  # Each compiler runs once in a VM unless it's run again, as Phoenix's code reloader runs them.
  defp compiled do
    Mix.Task.Compiler.reenable()
    Mix.Task.rerun("compile.all", ["--no-deps-check", "--return-errors"])
  end

  # A copy of what the servers run, which what's compiled next is told from.
  defp copy(state) do
    File.rm_rf!(state.copy)

    for dir <- ["ebin", "consolidated"], copy = Path.join(state.copy, dir) do
      File.mkdir_p!(Path.dirname(copy))
      File.cp_r!(Path.join(@build, dir), copy)
    end
  end

  # As `bun build.js` builds it as the network starts, into the folder every server serves it from.
  defp ui_built(ui, %{ui: ui} = state), do: {:ok, state}

  defp ui_built(ui, state) do
    case System.cmd("bun", ["build.js"], cd: Path.join(@root, "core/ui"), stderr_to_stdout: true) do
      {_output, 0} ->
        for server <- state.servers, do: :ok = :erpc.call(server, Deploy, :ui, [Path.join(@root, "core/ui/dist")])
        Mix.shell().info("Built the UI library")
        {:ok, %{state | ui: ui}}

      {output, _status} ->
        {:error, "The UI library didn't build:\n" <> output, state}
    end
  end

  defp deployed(apps, server, state) do
    changed = for {name, files} <- apps, state.apps[name] != files, do: name
    if changed != [], do: versioned(state.servers)

    case for(name <- changed, (why = deploy(server, name)) != :ok, do: "#{name}: #{why}") do
      [] ->
        if changed != [], do: Mix.shell().info("Deployed #{Enum.join(changed, ", ")}")
        {:ok, %{state | apps: keep(state, apps)}}

      failed ->
        {:error, "An app didn't deploy: #{Enum.join(failed, "; ")}", state}
    end
  end

  # Has each server's build be this checkout's version (`tooling/version`), as a release's would be: an app that
  # requires a newer platform than the one the servers started on is refused as waiting for their update, and what's
  # loaded into them is this checkout's.
  defp versioned(servers) do
    {version, 0} = System.cmd(Path.join(@root, "tooling/version"), [], cd: @root)

    for server <- servers,
        do: :ok = :erpc.call(server, Application, :put_env, [:one_core, :build, String.trim(version)])
  end

  # What each app was last deployed from, as the network's folder keeps it, or nil for a network that hasn't kept it.
  defp kept(state) do
    with {:ok, kept} <- File.read(state.copy <> "-apps"), do: :erlang.binary_to_term(kept), else: (_ -> nil)
  end

  defp keep(state, apps) do
    File.write!(state.copy <> "-apps", :erlang.term_to_binary(apps))
    apps
  end

  # As `one deploy --name <name> --files-from apps/<name>` deploys it for the network.
  defp deploy(server, name) do
    folder = Path.join([@root, "apps", name])
    {:ok, paths} = Folder.files(folder)
    files = Map.new(paths, &{&1, File.read!(Path.join(folder, &1))})

    case :erpc.call(server, Deploy, :deploy, ["network", name, files], :infinity) do
      {:ok, _number, _warnings} -> :ok
      {:error, why} -> why
    end
  end

  defp diagnostic(%{file: file, position: position, message: message}),
    do: "#{Path.relative_to(to_string(file), @root)}:#{line(position)}: #{message}"

  defp line({line, _column}), do: line
  defp line(line), do: line

  # What the Rust workspace, One Core's code and the UI library are built from, and what each app is deployed from, by
  # each file's contents: a file's time is only to the second, and hashing them all takes no longer than reading their
  # times. One Core's is its NIFs too, by the Rust they're built from: the NIFs themselves are over 100 MB in a debug
  # build, more with each build loaded beside the one before, and reading them before every request made each take a
  # second once a page asked a few at once.
  defp rust, do: hashes(tracked(["*.rs", "*Cargo.toml", "Cargo.lock", ".cargo"]))

  defp core(rust), do: {hashes(Path.wildcard(Path.join(@root, "core/server/lib/**/*.ex"))), rust}

  defp ui, do: hashes(tracked(["core/ui"]))

  defp apps do
    for folder <- Path.wildcard(Path.join(@root, "apps/*")), {:ok, paths} <- [Folder.files(folder)], into: %{} do
      {Path.basename(folder), hashes(Enum.map(paths, &Path.join(folder, &1)))}
    end
  end

  defp tracked(pathspecs) do
    {files, 0} =
      System.cmd("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--" | pathspecs], cd: @root)

    for file <- String.split(files, <<0>>, trim: true), do: Path.join(@root, file)
  end

  defp hashes(paths), do: for(path <- paths, {:ok, text} <- [File.read(path)], into: %{}, do: {path, :erlang.md5(text)})
end

[folder] = System.argv()
{:ok, _pid} = CodeReloader.start_link(Path.expand(folder))
IO.puts("The code reloader is running")
