# The code reloader of a network on this machine, as Phoenix's code reloader is in development, which
# `tooling/network` starts beside its servers, from `core/server/`:
#
#     mix run --no-start --no-halt ../../tooling/code_reloader.exs <folder>
#
# It's a script rather than part of One Core's code, which it compiles and loads: compiling itself would end it.
defmodule CodeReloader do
  @moduledoc """
  A server started with `--reloader <its node>` (`mix network`) asks it before it answers a client's request
  (`OneCore.MCP`), so the request sees what changed in this checkout since the last:

  - One Core's code is compiled here and loaded into every server while it runs (`OneCore.Release.swap/3`), unless
    it's what a running server can't load (`OneCore.Release.modules/2`, the rule a hot load follows): then the request
    is refused, saying why and that the network starts again for it.
  - A plugin of this repo whose files changed is deployed for the network again, as `one deploy --files-from` deploys
    it. A request is refused with why it didn't deploy.

  Rust, the deps and the config are built as the network starts again. Its compiler's output is its log.
  """
  use GenServer
  alias OneCore.{Folder, Release}
  alias OneCore.Plugins.Deploy

  @root Path.expand("..", __DIR__)
  @build Path.expand("../core/server/_build/dev/lib/one_core", __DIR__)

  def start_link(folder), do: GenServer.start_link(__MODULE__, Path.join(folder, "reloader"), name: __MODULE__)

  @doc "Has the server whose node is `server` reloaded from now on: what it runs is what this has compiled."
  def serving(server), do: GenServer.call(__MODULE__, {:serving, server}, :infinity)

  @doc "Loads what changed into every server, as `server` asks before a request: `:ok`, or `{:error, why}`."
  def reload(server), do: GenServer.call(__MODULE__, {:reload, server}, :infinity)

  @impl GenServer
  def init(copy), do: {:ok, %{copy: copy, servers: [], core: nil, plugins: nil}}

  # What the servers run is what's compiled when the first starts, so that's what a change is told from.
  @impl GenServer
  def handle_call({:serving, server}, _from, %{core: nil} = state) do
    copy(state)
    handle_call({:serving, server}, nil, %{state | core: core(), plugins: plugins()})
  end

  def handle_call({:serving, server}, _from, state), do: {:reply, :ok, %{state | servers: [server | state.servers]}}

  # What failed is the request's answer, and the next request tries again.
  def handle_call({:reload, server}, _from, state) do
    with {:ok, state} <- reloaded(core(), state), {:ok, state} <- deployed(plugins(), server, state) do
      {:reply, :ok, state}
    else
      {:error, why, state} -> {:reply, {:error, why}, state}
    end
  catch
    kind, why -> {:reply, {:error, "The code reloader failed: " <> Exception.format(kind, why, __STACKTRACE__)}, state}
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
    for compiler <- Mix.Tasks.Compile.compilers(), do: Mix.Task.reenable("compile.#{compiler}")
    Mix.Task.rerun("compile.all", ["--no-deps-check", "--return-errors"])
  end

  # A copy of what the servers run, which what's compiled next is told from.
  defp copy(state) do
    File.rm_rf!(state.copy)
    File.mkdir_p!(state.copy)
    for dir <- ["ebin", "consolidated"], do: File.cp_r!(Path.join(@build, dir), Path.join(state.copy, dir))
  end

  defp swap(server, changes, new) do
    case :erpc.call(server, Release, :swap, [changes, new], :infinity) do
      :ok -> :ok
      {:error, why} -> why
    end
  end

  defp deployed(plugins, server, state) do
    changed = for {name, files} <- plugins, state.plugins[name] != files, do: name

    case for(name <- changed, (why = deploy(server, name)) != :ok, do: "#{name}: #{why}") do
      [] ->
        if changed != [], do: Mix.shell().info("Deployed #{Enum.join(changed, ", ")}")
        {:ok, %{state | plugins: plugins}}

      failed ->
        {:error, "A plugin didn't deploy: #{Enum.join(failed, "; ")}", state}
    end
  end

  # As `one deploy --name <name> --files-from plugins/<name>` deploys it for the network.
  defp deploy(server, name) do
    folder = Path.join([@root, "plugins", name])
    {:ok, paths} = Folder.files(folder)
    files = Map.new(paths, &{&1, File.read!(Path.join(folder, &1))})

    case :erpc.call(server, Deploy, :deploy, ["network", name, files], :infinity) do
      {:ok, _number, _warnings} -> :ok
      {:error, why} -> why
    end
  end

  defp code(dir), do: %{ebin: Path.join(dir, "ebin"), consolidated: Path.join(dir, "consolidated")}

  defp diagnostic(%{file: file, position: position, message: message}),
    do: "#{Path.relative_to(to_string(file), @root)}:#{line(position)}: #{message}"

  defp line({line, _column}), do: line
  defp line(line), do: line

  # What One Core's code is compiled from, and what each plugin is deployed from, by each file's contents: a file's
  # time is only to the second, and hashing them all takes no longer than reading their times.
  defp core, do: hashes(Path.wildcard(Path.join(@root, "core/server/lib/**/*.ex")))

  defp plugins do
    for folder <- Path.wildcard(Path.join(@root, "plugins/*")), {:ok, paths} <- [Folder.files(folder)], into: %{} do
      {Path.basename(folder), hashes(Enum.map(paths, &Path.join(folder, &1)))}
    end
  end

  defp hashes(paths), do: for(path <- paths, {:ok, text} <- [File.read(path)], into: %{}, do: {path, :erlang.md5(text)})
end

[folder] = System.argv()
{:ok, _pid} = CodeReloader.start_link(Path.expand(folder))
IO.puts("The code reloader is running")
