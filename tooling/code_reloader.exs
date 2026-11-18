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
  - One Core's code is compiled here and loaded into every server while it runs (`OneCore.Release.swap/3`), its NIFs
    too, each beside the build before, unless it's what a running server can't load (`OneCore.Release.modules/2`,
    the rule a hot load follows): then the request is refused, saying why and that the network starts again for it.
  - The UI library (`core/ui`), where a file of it changed, is built again and is the one every server serves, as a
    release's is (`OneCore.Release.ui/1`).
  - An app of this repo whose files changed is deployed for the network again, as `one deploy --files-from` deploys
    it. A request is refused with why it didn't deploy.

  The deps and the config are built as the network starts again. Its compilers' output is its log.
  """
  use GenServer
  alias OneCore.{Folder, Release}
  alias OneCore.Apps.Deploy

  @root Path.expand("..", __DIR__)
  @build Path.expand("../core/server/_build/dev/lib/one_core", __DIR__)

  def start_link(folder), do: GenServer.start_link(__MODULE__, Path.join(folder, "reloader"), name: __MODULE__)

  @doc "Has the server whose node is `server` reloaded from now on: what it runs is what this has compiled."
  def serving(server), do: GenServer.call(__MODULE__, {:serving, server}, :infinity)

  @doc "Loads what changed into every server, as `server` asks before a request: `:ok`, or `{:error, why}`."
  def reload(server), do: GenServer.call(__MODULE__, {:reload, server}, :infinity)

  @impl GenServer
  def init(copy), do: {:ok, %{copy: copy, servers: [], rust: nil, core: nil, ui: nil, apps: nil}}

  # What the servers run is what's compiled when the first starts, so that's what a change is told from.
  @impl GenServer
  def handle_call({:serving, server}, _from, %{core: nil} = state) do
    copy(state)
    handle_call({:serving, server}, nil, %{state | rust: rust(), core: core(), ui: ui(), apps: apps()})
  end

  def handle_call({:serving, server}, _from, state), do: {:reply, :ok, %{state | servers: [server | state.servers]}}

  # What failed is the request's answer, and the next request tries again.
  def handle_call({:reload, server}, _from, state) do
    with {:ok, state} <- built(rust(), state),
         {:ok, state} <- reloaded(core(), state),
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
    new = code(@build)

    with {:compiled, {compiled, _diagnostics}} when compiled in [:ok, :noop] <- {:compiled, compiled()},
         {changes, nil} when changes != {[], [], []} <- Release.modules(code(state.copy), new),
         [] <- for(server <- state.servers, (why = swap(server, changes, new)) != :ok, do: "#{server}: #{why}") do
      copy(state)
      {removed, added, changed} = changes
      took = System.monotonic_time(:millisecond) - began
      Mix.shell().info("Loaded #{Enum.map_join(removed ++ added ++ changed, ", ", &inspect/1)} in #{took} ms")
      {:ok, %{state | core: core}}
    else
      {:compiled, {:error, diagnostics}} ->
        {:error, "One Core didn't compile:\n" <> Enum.map_join(diagnostics, "\n", &diagnostic/1), state}

      {{[], [], []}, _why} ->
        {:ok, %{state | core: core}}

      {_changes, why} ->
        {:error,
         "One Core changed in a way a running server can't load (#{why}): stop tooling/network and start it again " <>
           "in the same folder.", state}

      failed ->
        {:error, "One Core's change didn't load: #{Enum.join(failed, "; ")}", state}
    end
  end

  # Each compiler runs once in a VM unless it's run again, as Phoenix's code reloader runs them.
  defp compiled do
    Mix.Task.Compiler.reenable()
    Mix.Task.rerun("compile.all", ["--no-deps-check", "--return-errors"])
  end

  # A copy of what the servers run, which what's compiled next is told from.
  defp copy(state) do
    File.rm_rf!(state.copy)

    for dir <- ["ebin", "consolidated", "priv/native"], copy = Path.join(state.copy, dir) do
      File.mkdir_p!(Path.dirname(copy))
      File.cp_r!(Path.join(@build, dir), copy)
    end
  end

  # As `bun build.js` builds it as the network starts, into the folder every server serves it from.
  defp ui_built(ui, %{ui: ui} = state), do: {:ok, state}

  defp ui_built(ui, state) do
    case System.cmd("bun", ["build.js"], cd: Path.join(@root, "core/ui"), stderr_to_stdout: true) do
      {_output, 0} ->
        for server <- state.servers, do: :ok = :erpc.call(server, Release, :ui, [Path.join(@root, "core/ui/dist")])
        Mix.shell().info("Built the UI library")
        {:ok, %{state | ui: ui}}

      {output, _status} ->
        {:error, "The UI library didn't build:\n" <> output, state}
    end
  end

  defp swap(server, changes, new) do
    case :erpc.call(server, Release, :swap, [changes, new], :infinity) do
      :ok -> :ok
      {:error, why} -> why
    end
  end

  defp deployed(apps, server, state) do
    changed = for {name, files} <- apps, state.apps[name] != files, do: name

    case for(name <- changed, (why = deploy(server, name)) != :ok, do: "#{name}: #{why}") do
      [] ->
        if changed != [], do: Mix.shell().info("Deployed #{Enum.join(changed, ", ")}")
        {:ok, %{state | apps: apps}}

      failed ->
        {:error, "An app didn't deploy: #{Enum.join(failed, "; ")}", state}
    end
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

  defp code(dir),
    do: %{
      ebin: Path.join(dir, "ebin"),
      consolidated: Path.join(dir, "consolidated"),
      native: Path.join(dir, "priv/native")
    }

  defp diagnostic(%{file: file, position: position, message: message}),
    do: "#{Path.relative_to(to_string(file), @root)}:#{line(position)}: #{message}"

  defp line({line, _column}), do: line
  defp line(line), do: line

  # What the Rust workspace, One Core's code and the UI library are built from, and what each app is deployed from, by
  # each file's contents: a file's time is only to the second, and hashing them all takes no longer than reading their
  # times. One Core's is its NIFs too, as they're built.
  defp rust, do: hashes(tracked(["*.rs", "*Cargo.toml", "Cargo.lock", ".cargo"]))

  defp core,
    do:
      hashes(
        Path.wildcard(Path.join(@root, "core/server/lib/**/*.ex")) ++ Path.wildcard(Path.join(@build, "priv/native/*"))
      )

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
