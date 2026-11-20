defmodule OneCore.Sandbox do
  @moduledoc """
  The sandbox an untrusted app runs in: its version's code runs in an Erlang VM of its own, a child of this
  server's, behind this machine's process sandbox, never in this server's VM. Any of the network's servers runs one
  for anyone, since what it reaches is only what this server hands it. The wall is the app `sandbox`, which One
  Core asks one thing of: start this program under this policy and give me its pipes (`Sandbox.start/3`). Without it,
  no untrusted app runs, and a call to one says why.

  There's one child for each version of an untrusted app on a server and each user, group or network whose own artifact
  it keeps (`OneCore.Apps.Calls.whose/3`): whoever it's installed for, or each person, where it keeps theirs apart,
  with the versions of the apps it requires whose code runs inside it, as it, where that code can run there
  (`database` among them, its SQLite too). So a child only ever runs one owner's calls, and what a stranger's app
  keeps from one person's call isn't there in another's: its handles are its own, and a request for an artifact or a
  call that isn't its own is refused. It's started by the first call, compiles the code itself, behind the wall, and
  stops after a minute idle. One that stops, or is stopped, fails the calls it was running and nothing else; the next
  call starts another. What the first child of a version compiled is kept on this machine, and every owner's next child
  loads it read-only instead.

  The protocol, over the child's standard input and output, each message a frame of a 4-byte length:

  - This server sends Erlang terms, which the child trusts: `{:load, sets, target}`, the apps' files to compile,
    those it requires first; `{:call, id, what, about}`, a tool, a hook heard or what `server-debug` asks (`debug/5`),
    with who it's for;
    `{:reply, request, result}`; and `{:cancel, id}`.
  - The child sends JSON, which never reaches `binary_to_term`: `{"answer": id, "ok" | "raised": ...}`,
    `{"request": n, "call": id, "op": ..., ...}`, one of the functions below, and `{"log": level, "text": ...}`.
    A frame over 64 MB, or that isn't one of these, stops it.

  What it's offered is what the app interface offers, as messages: `OneCore.App.Manifest.offered/0`, a process
  being its own VM's, and Logger. An
  artifact it's given is a handle this server looks up, never a prefix
  it could rewrite, and a call is one this server is running, never a subject, actor or caller it could name. It hooks
  only into what it observes: a mutate, validate or collect hook asks it nothing. It has no services, no other apps'
  functions, no network, no website it reaches, no programs, and no files but a folder of its own that goes when it
  stops.

  It calls the tools of only the apps it requires (`OneCore.App`): a call to any other's is refused. So does its
  page, which its client says is its (`OneCore.Mcp`), and which asks for whoever it was opened for.
  """
  @behaviour :gen_statem
  require Logger
  alias OneCore.{Artifact, Bucket, Builds, Call, Apps, Server, User}
  alias OneCore.Call.State
  alias OneCore.Apps.{Store, Version}

  @registry __MODULE__.Children
  @supervisor __MODULE__.Supervisor

  # How long a child that runs no call is kept, in milliseconds.
  @idle :timer.minutes(1)
  # How long a call waits on its child saying nothing, in milliseconds, before it's cancelled.
  @silent :timer.minutes(2)
  # The largest frame a child may send, and the most memory it may have.
  @largest 64 * 1024 * 1024
  @memory 1024 * 1024 * 1024
  # How many requests outside a call a child may have this server running at once, and how many lines it may log
  # a minute.
  @at_once 64
  @logs 120

  @doc "What runs the children: where they're registered, and what starts them."
  def children, do: [{Registry, keys: :unique, name: @registry}, {DynamicSupervisor, name: @supervisor}]

  @doc """
  What a set of an untrusted app's files says of itself, from a child that compiles only them: `{:ok, version}`
  of the app `artifact`, or `{:error, why}`. The files are as the server keeps them.
  """
  def describe(artifact, files) do
    name = Path.basename(artifact)
    sets = [{name, Map.reject(files, fn {path, _} -> Version.client?(path) end)}]

    with {:ok, description} <- once({:describe, name, Version.code(files)}, name, sets),
         do: Version.Description.untrusted(artifact, files, description)
  end

  @doc """
  Checks an untrusted app's version, from its `files` as the server keeps them, as it will run, with the apps it
  requires whose code runs inside it as `owner` finds them: `{:ok, warnings}`, or `{:error, why}` with each thing it
  uses that the sandbox doesn't offer, a sentence each with its file and line.
  """
  def check(owner, %Version{} = version, files) do
    with {:ok, %{"problems" => problems, "warnings" => warnings}} <-
           loaded(owner, version, Apps.required(owner, version), files) do
      case problems do
        [] ->
          {:ok, warnings}

        _ ->
          {:error, "Untrusted, #{Path.basename(version.artifact)} can't run as it is:\n" <> Enum.join(problems, "\n")}
      end
    end
  end

  @doc """
  Runs `program` with `arguments` to its end behind this machine's sandbox, under `policy` as the app `sandbox`
  takes it: what an untrusted app's entrypoint is built with, say. Gives `{output, status}`, or `{:error, why}` where it
  couldn't start. One that runs longer than five minutes is stopped.
  """
  def run(program, arguments, policy) do
    with {:ok, port} <- walled(program, arguments, policy), do: ran(port, [])
  end

  defp ran(port, output) do
    receive do
      {^port, {:data, data}} -> ran(port, [output | data])
      {^port, {:exit_status, status}} -> {IO.iodata_to_binary(output), status}
    after
      :timer.minutes(5) ->
        kill(port)
        {IO.iodata_to_binary(output) <> "\nIt was stopped after five minutes.", 1}
    end
  end

  # Kills what a port runs, the wall, and with it the program behind it, which goes with the wall everywhere.
  defp kill(port) do
    with {:os_pid, pid} <- Port.info(port, :os_pid) do
      case :os.type() do
        {:win32, _} -> System.cmd("taskkill", ["/F", "/PID", to_string(pid)], stderr_to_stdout: true)
        _ -> System.cmd("kill", ["-KILL", to_string(pid)])
      end
    end
  end

  # Starts a program behind the wall, which the app `sandbox` is, with its pipes as a port this process owns.
  defp walled(program, arguments, policy) do
    with {:error, why} <-
           Apps.run(["network"], "sandbox", fn _at, versions ->
             versions["sandbox"].app.start(program, arguments, policy)
           end),
         do:
           {:error,
            "An untrusted app runs behind the app sandbox, installed for the network, which didn't start it: #{why}"}
  end

  @doc """
  Runs `what` of the app `name` among `versions` in its child: `{:tool, tool, inputs}` or `{:hook, hook, about}`,
  for `call`. Gives what the app answered, or raises with why it didn't.
  """
  def call(versions, name, what, %Call{} = call) do
    version = versions[name]

    id = System.unique_integer([:positive])

    required = Map.delete(versions, name)

    # Its child is whoever's own artifact the call has, so what it keeps from one person's call isn't there in another's.
    whose = User.owner(Artifact.name(call.own))

    if Registry.lookup(@registry, {whose, key(version, required)}) == [],
      do: State.working(call.state, :waiting, "Starting its sandbox")

    {child, port, context} =
      case child(whose, version, required, nil, {:begin, id}) do
        {:ok, child, port, context} -> {child, port, context}
        {:error, why} -> raise why
      end

    State.working(call.state, :running, nil)

    try do
      about = %{
        "own" => [handle(context, call.own), Artifact.name(call.own)],
        "installed" => call.installed,
        "subject" => call.subject,
        "actor" => call.actor,
        "caller" => call.caller,
        "app" => call.app,
        "app_artifact" => call.app_artifact,
        "server" => call.server,
        "loaded" => Map.keys(versions) -- [name]
      }

      command(port, {:call, id, what, about})
      waited(child, port, id, call, context)
    after
      :gen_statem.cast(child, {:ended, id})
    end
  end

  @doc """
  Asks the VM of the app `name` among `versions`, the one whose own artifact is `owner`'s, what the app `server-debug` asks of it:
  `request`, a map JSON holds, which its side of One Core answers (`priv/sandbox/child.ex`). Gives what it
  answered, or raises with why it didn't. Only a VM that's running is asked, unless `start` says to start one.
  """
  def debug(owner, versions, name, request, start \\ false) do
    version = versions[name]

    if not start and Registry.lookup(@registry, {owner, key(version, Map.delete(versions, name))}) == [],
      do: raise("#{name} has no VM running for #{owner} on this server: its first call starts one")

    call = %Call{
      own: %Artifact{prefix: "#{owner}/own/#{name}/"},
      installed: owner,
      folder: nil,
      server: Server.key(),
      subject: owner,
      actor: nil,
      app: name,
      app_artifact: version.artifact,
      request: nil,
      loaded: %{},
      forwarded: false
    }

    call(versions, name, {:debug, request}, call)
  end

  defp waited(child, port, id, call, context) do
    receive do
      {__MODULE__, :request, ^id, request} ->
        command(port, {:reply, request["request"], handled(request, call, context)})
        waited(child, port, id, call, context)

      {__MODULE__, :reply, ^id, reply} ->
        decoded(reply, context)

      {__MODULE__, :stopped, ^child, why} ->
        raise "#{context.name} stopped while it ran this call: #{why}"
    after
      @silent ->
        :gen_statem.cast(child, {:cancel, id})
        raise "#{context.name} said nothing for #{div(@silent, 1000)} seconds, so its call was stopped"
    end
  end

  defp decoded(%{"ok" => %{"text" => text}}, _context) when is_binary(text), do: text
  defp decoded(%{"ok" => %{"json" => json}}, _context), do: json

  defp decoded(%{"ok" => %{"blob" => [kind, data]}}, _context) when is_binary(kind) and is_binary(data),
    do: {:blob, kind, Base.decode64!(data)}

  defp decoded(%{"ok" => %{"file" => [kind, handle, name]}}, context) when is_binary(kind) and is_binary(name),
    do: {:file, kind, artifact!(context, handle), name}

  defp decoded(%{"ok" => nil}, _context), do: nil
  defp decoded(%{"raised" => why}, _context) when is_binary(why), do: raise(String.slice(why, 0, 8000))
  defp decoded(_reply, context), do: raise("#{context.name} answered what isn't an answer")

  # What a request is answered with: `{:ok, result}`, or `{:raise, why}`. Moving the call to another server ends it
  # here (`OneCore.Call.forward_to/2`), which isn't caught.
  defp handled(request, call, context) do
    {:ok, op(request["op"], request, call, context)}
  rescue
    error -> {:raise, Exception.message(error)}
  end

  # What needs only an artifact runs anywhere: a process of the child's that isn't a call, such as a database's,
  # asks it too.
  defp op("read", %{"handle" => handle, "name" => name}, _call, context),
    do: Artifact.read(artifact!(context, handle), blob!(name))

  defp op("read_all", %{"handle" => handle, "names" => names}, _call, context) when is_list(names),
    do: Artifact.read_all(artifact!(context, handle), Enum.map(names, &blob!/1))

  defp op("write", %{"handle" => handle, "name" => name, "data" => data}, _call, context),
    do: Artifact.write(artifact!(context, handle), blob!(name), Base.decode64!(data))

  defp op("replace", %{"handle" => handle, "name" => name, "data" => data, "tag" => tag}, _call, context),
    do: Artifact.replace(artifact!(context, handle), blob!(name), Base.decode64!(data), tag)

  defp op("remove", %{"handle" => handle, "name" => name}, _call, context),
    do: Artifact.remove(artifact!(context, handle), blob!(name))

  defp op("size", %{"handle" => handle, "name" => name}, _call, context),
    do: Artifact.size(artifact!(context, handle), blob!(name))

  defp op("list", %{"handle" => handle, "starting" => starting}, _call, context),
    do: Artifact.list(artifact!(context, handle), listed!(starting))

  defp op(op, _request, nil, _context), do: raise("#{op} is asked within a call, and that call has ended")

  defp op("tool", %{"app" => app, "tool" => tool, "inputs" => inputs} = request, call, context)
       when is_binary(app) and is_binary(tool) and is_map(inputs) do
    if not is_map_key(context.requires, app),
      do:
        raise(
          "#{context.name} may not call #{app}_#{tool}: its requires/0 doesn't name #{app}. An untrusted " <>
            "app calls the tools of only the apps it requires."
        )

    options = if is_map_key(request, "on_behalf_of"), do: [on_behalf_of: request["on_behalf_of"]], else: []
    Call.tool(call, app, tool, inputs, options)
  end

  defp op("make", %{"id" => id}, call, context), do: named(context, Call.make(call, id))

  defp op("reach", %{"name" => name, "relation" => relation}, call, context),
    do: named(context, Call.reach(call, artifact_name!(name), relation!(relation)))

  defp op("reaches", %{"name" => name, "relation" => relation}, call, _context),
    do: Call.reaches?(call, artifact_name!(name), relation!(relation))

  defp op("member", %{"of" => of}, call, _context) when is_binary(of), do: Call.member?(call, of)
  defp op("owns", _request, call, _context), do: Call.owns?(call)
  defp op("apps", _request, call, _context), do: Call.apps(call)
  defp op("deployed", _request, call, _context), do: Call.deployed(call)
  defp op("forward_to", %{"server" => server}, call, _context) when is_binary(server), do: Call.forward_to(call, server)
  defp op("changed", _request, call, _context), do: Call.changed(call)
  defp op("stage", %{"text" => text}, call, _context) when is_binary(text), do: Call.stage(call, text)
  defp op("ask", %{"question" => question}, call, _context), do: Call.ask(call, question)
  defp op(op, _request, _call, _context), do: raise("#{inspect(op)} isn't offered to an untrusted app")

  defp named(context, artifact), do: [handle(context, artifact), Artifact.name(artifact)]

  # A blob's name in an artifact, which can't step outside it (`OneCore.Bucket.check_key/1`).
  defp blob!(name), do: Bucket.key!(name)

  # What a list starts with: nothing, a blob's name, or one's first parts and the / after them.
  defp listed!(""), do: ""

  defp listed!(starting) when is_binary(starting) do
    Bucket.key!(String.replace_suffix(starting, "/", ""))
    starting
  end

  defp artifact_name!(name) when is_binary(name), do: name
  defp relation!(relation) when relation in ["viewer", "editor", "owner"], do: relation
  defp relation!(_), do: raise(ArgumentError, "a relation is viewer, editor or owner")

  # An artifact the child holds a handle to, kept here for as long as the child runs. The same artifact has the
  # same handle, and no two children's handles are the same, so one passed on from another owner's child is refused.
  defp handle(context, %Artifact{} = artifact) do
    case :ets.lookup(context.handles, artifact) do
      [{^artifact, handle}] ->
        handle

      [] ->
        handle = System.unique_integer([:positive])
        :ets.insert(context.handles, [{handle, artifact}, {artifact, handle}])
        handle
    end
  end

  defp artifact!(context, handle) when is_integer(handle) do
    case :ets.lookup(context.handles, handle) do
      [{^handle, artifact}] -> artifact
      [] -> raise "That isn't an artifact this app was given"
    end
  end

  defp artifact!(_context, _handle), do: raise("That isn't an artifact this app was given")

  defp command(port, term) do
    frame = :erlang.term_to_binary(term)
    Port.command(port, [<<byte_size(frame)::32>>, frame])
  end

  # The child of a version with the versions whose code runs inside it, for whoever it's installed for, `owner`, started
  # if there's none, asked `ask` once it has loaded: `:loaded`, or `{:begin, id}`, a call it runs from now on. Gives
  # `{:ok, child, port, context}`, or `{:error, why}` with why it couldn't start or load.
  defp child(owner, version, required, files, ask) do
    key = key(version, required)

    case Registry.lookup(@registry, {owner, key}) do
      [{child, _}] ->
        try do
          :gen_statem.call(child, ask)
        catch
          # It stopped, idle, between being found and being asked, so it's another that's asked.
          :exit, {stopped, _} when stopped in [:noproc, :normal] -> child(owner, version, required, files, ask)
        end

      [] ->
        sets = fn -> sets(version, required, files) end
        spec = {__MODULE__, {owner, key, Path.basename(version.artifact), sets, version}}

        case DynamicSupervisor.start_child(@supervisor, spec) do
          {:ok, child} -> :gen_statem.call(child, ask)
          {:error, {:already_started, _child}} -> child(owner, version, required, files, ask)
        end
    end
  catch
    :exit, _ -> {:error, "#{Path.basename(version.artifact)} stopped as it started"}
  end

  # What tells a version's child from another's: its name and code, and those of the versions whose code runs inside
  # it. Two apps of the same files are two apps, each with its VM and what it compiled.
  defp key(version, required),
    do:
      {Path.basename(version.artifact), version.code, Enum.sort(for {name, other} <- required, do: {name, other.code})}

  # A child that runs once, to say what a set of files is, and stops.
  defp once(key, name, sets) do
    {:ok, child} = DynamicSupervisor.start_child(@supervisor, {__MODULE__, {nil, key, name, fn -> sets end, nil}})

    try do
      with {:ok, _child, _port, context} <- :gen_statem.call(child, :loaded), do: {:ok, context.description}
    after
      DynamicSupervisor.terminate_child(@supervisor, child)
    end
  catch
    :exit, _ -> {:error, "#{name} stopped as it started"}
  end

  defp loaded(owner, version, required, files) do
    case child(owner, version, required, files, :loaded) do
      {:ok, _child, _port, context} -> {:ok, context.description}
      {:error, why} -> {:error, why}
    end
  end

  # The files of each app, those it requires first, from the bucket but the app's own where they're given: what
  # isn't in client/, and of its NIFs' libraries only this machine's.
  defp sets(version, required, files) do
    own = Path.basename(version.artifact)

    for {name, %Version{artifact: artifact, code: code}} <- Enum.to_list(required) ++ [{own, version}] do
      found = if name == own and files, do: Map.keys(files), else: Store.paths(artifact, code)

      paths =
        for path <- found,
            not Version.client?(path),
            Path.extname(path) != ".so" or Path.basename(path) == "#{Builds.target()}.so",
            do: path

      read =
        if name == own and files,
          do: Map.take(files, paths),
          else:
            for(
              {path, {:ok, data}} <- Enum.zip(paths, Store.read_all(for path <- paths, do: {artifact, code, path})),
              into: %{},
              do: {path, data}
            )

      {name, read}
    end
  end

  def child_spec(started),
    do: %{id: __MODULE__, start: {__MODULE__, :start_link, [started]}, restart: :temporary}

  def start_link({owner, key, _name, _sets, _version} = started),
    do: :gen_statem.start_link({:via, Registry, {@registry, {owner, key}}}, __MODULE__, started, [])

  # A child is `:loading` while its VM starts and compiles the code, until it says what it is. Then it's `:ready`,
  # running no call, or `:busy`, running some. One that couldn't start or load is `{:failed, why}`: it tells
  # whatever asked meanwhile why, and stops. A ready one stops after a minute, which a call cancels by making it busy.
  @impl true
  def callback_mode, do: [:handle_event_function, :state_enter]

  @impl true
  def init({owner, key, name, sets, version}) do
    handles = :ets.new(__MODULE__, [:public, read_concurrency: true])

    # What a version's VM compiled is kept on this machine for its next VM, whoever's it is, which loads it instead:
    # its own code and the child's side of One Core, as this server has it.
    runtime =
      for file <- ~w(child.ex manifest.ex redact.ex nif.ex),
          do: File.read!(Application.app_dir(:one_core, "priv/sandbox/#{file}"))

    cache = if version, do: Path.join([Server.home(), "sandbox", Version.code({key, runtime})])

    context = %{
      cache: cache,
      owner: owner,
      name: name,
      handles: handles,
      artifact: version && version.artifact,
      requires: (version && version.requires) || %{},
      description: nil
    }

    data = %{context: context, port: nil, buffer: "", calls: %{}, requests: 0, logged: {0, 0}}
    {:ok, :loading, data, [{:next_event, :internal, {:start, sets}}]}
  end

  @impl true
  def handle_event(:enter, _was, :loading, _data), do: {:keep_state_and_data, [{:state_timeout, @silent, :loading}]}
  def handle_event(:enter, _was, :ready, _data), do: {:keep_state_and_data, [{:state_timeout, @idle, :idle}]}
  def handle_event(:enter, _was, :busy, _data), do: :keep_state_and_data
  # What was asked while it loaded is answered first.
  def handle_event(:enter, _was, {:failed, _why}, _data), do: {:keep_state_and_data, [{:state_timeout, 0, :failed}]}

  def handle_event(:internal, {:start, sets}, :loading, data) do
    # A VM started from what's kept marks it as in use.
    cached =
      with path when is_binary(path) <- data.context.cache,
           {:ok, description} <- File.read(Path.join(path, "modules")) do
        File.touch(path)
        JSON.decode!(description)
      else
        _ -> nil
      end

    case started(cached && data.context.cache) do
      {:ok, port} ->
        cache = cached && %{"ebin" => data.context.cache, "modules" => cached}
        command(port, {:load, sets.(), Builds.target(), cache})
        # Only a VM that compiled the code itself, before any call, has what it compiled kept.
        context = if cached, do: %{data.context | cache: nil}, else: data.context
        {:keep_state, %{data | port: port, context: context}}

      {:error, why} ->
        {:next_state, {:failed, why}, data}
    end
  end

  def handle_event(:state_timeout, :loading, :loading, data),
    do: stop(:loading, data, "it didn't load within #{div(@silent, 1000)} seconds")

  def handle_event(:state_timeout, :idle, :ready, _data), do: {:stop, :normal}
  def handle_event(:state_timeout, :failed, {:failed, _why}, _data), do: {:stop, :normal}

  def handle_event({:call, _from}, _ask, :loading, _data), do: {:keep_state_and_data, [:postpone]}

  def handle_event({:call, from}, _ask, {:failed, why}, _data),
    do: {:keep_state_and_data, [{:reply, from, {:error, why}}]}

  def handle_event({:call, from}, :loaded, _state, data), do: {:keep_state_and_data, [{:reply, from, loaded(data)}]}

  def handle_event({:call, {caller, _} = from}, {:begin, id}, _state, data) do
    calls = Map.put(data.calls, id, {caller, Process.monitor(caller)})
    {:next_state, :busy, %{data | calls: calls}, [{:reply, from, loaded(data)}]}
  end

  def handle_event(:cast, {:ended, id}, state, data), do: ended(state, data, id, false)
  def handle_event(:cast, {:cancel, id}, state, data), do: ended(state, data, id, true)

  def handle_event(:info, {port, {:data, received}}, state, %{port: port} = data),
    do: frames(state, %{data | buffer: data.buffer <> received})

  def handle_event(:info, {port, {:exit_status, status}}, state, %{port: port} = data),
    do: stop(state, data, "its VM ended with status #{status}")

  def handle_event(:info, {:DOWN, monitor, :process, _caller, _why}, state, data) do
    case Enum.find(data.calls, fn {_id, {_caller, watched}} -> watched == monitor end) do
      {id, _} -> ended(state, data, id, true)
      nil -> :keep_state_and_data
    end
  end

  def handle_event(:info, :replied, _state, data), do: {:keep_state, %{data | requests: data.requests - 1}}

  @impl true
  # Its port closes as this process ends, but that doesn't end the program behind it, which is killed.
  def terminate(_why, _state, %{port: port}) when is_port(port), do: kill(port)

  def terminate(_why, _state, _data), do: :ok

  # The program the child is: this server's own Erlang VM and Elixir, read-only, with the child's side of One Core.
  defp started(cache) do
    root = :code.root_dir() |> to_string()
    erts = Path.join(root, "erts-#{:erlang.system_info(:version)}")

    boot =
      hd(
        Path.wildcard(Path.join(root, "bin/start_clean.boot")) ++
          Path.wildcard(Path.join(root, "releases/*/start_clean.boot"))
      )

    boot = Path.rootname(boot)
    elixir = :code.lib_dir(:elixir) |> to_string() |> Path.dirname()
    child = Application.app_dir(:one_core, "priv/sandbox")

    # Erlang's and Elixir's applications, and the server's libraries (OpenTelemetry's API, which an app records with,
    # and Recon, which it's debugged with, `debug/5`), but not the server's own code, which is kept beside them: in an
    # installed server's release, all in its `lib`, and in Mix's `_build` otherwise.
    libraries = Application.app_dir(:one_core) |> Path.dirname()

    apps =
      for ebin <- Enum.flat_map([root <> "/lib", elixir, libraries], &Path.wildcard(Path.join(&1, "*/ebin"))),
          app = Path.dirname(ebin),
          hd(String.split(Path.basename(app), "-")) != "one_core",
          uniq: true,
          do: app

    ebins = for app <- apps, do: Path.join(app, "ebin")

    # Its own side of One Core is compiled as it starts, unless it was kept.
    compile =
      if cache,
        do: "",
        else:
          "{ok, _, _} = 'Elixir.Kernel.ParallelCompiler':compile_to_path([<<\"#{child}/child.ex\"/utf8>>, " <>
            "<<\"#{child}/manifest.ex\"/utf8>>, <<\"#{child}/redact.ex\"/utf8>>, <<\"#{child}/nif.ex\"/utf8>>], " <>
            "unicode:characters_to_binary(filename:join(os:getenv(\"TMPDIR\"), \"runtime\")), " <>
            "[{return_diagnostics, true}]), "

    arguments =
      ~w(-noshell -noinput +fnu +S 1:1 +SDcpu 1:1 +SDio 1 +A 1 +MIscs 128 -boot) ++
        [boot, "-boot_var", "RELEASE_LIB", Path.join(root, "lib")] ++
        Enum.flat_map(List.wrap(cache) ++ ebins, &["-pa", &1]) ++
        ["-eval", "application:ensure_all_started([elixir, logger]), #{compile}'Elixir.OneCore.Sandbox.Child':main()."]

    env = %{
      "ROOTDIR" => root,
      "BINDIR" => Path.join(erts, "bin"),
      "EMU" => emulator(erts),
      "PROGNAME" => "erl",
      "HOME" => "/tmp",
      "LANG" => "C.UTF-8",
      "ERL_CRASH_DUMP_SECONDS" => "0"
    }

    policy = %{read: Enum.uniq([erts, boot <> ".boot", child | apps] ++ List.wrap(cache)), env: env, memory: @memory}

    # The VM's own program, which on Windows is `one-server.exe` (the release's `erl.exe`, `tooling/release`), with the VM
    # in it rather than started by it.
    vm = if match?({:win32, _}, :os.type()), do: "bin/one-server.exe", else: "bin/erlexec"
    walled(Path.join(erts, vm), arguments, policy)
  end

  # The emulator erlexec starts is named by the release (`tooling/release`): `<EMU>.smp` in the VM's bin folder. Windows'
  # has the VM in `one-server.exe`, with its emulator beside it as a DLL.
  defp emulator(erts) do
    case Path.wildcard(Path.join(erts, "bin/*.smp")) do
      [emulator] -> Path.basename(emulator, ".smp")
      [] -> "beam"
    end
  end

  defp loaded(data), do: {:ok, self(), data.port, data.context}

  # A call ended, here or in the child: one the child may still be running is cancelled there. A child running none
  # is ready again.
  defp ended(state, data, id, cancel) do
    case Map.pop(data.calls, id) do
      {{_caller, monitor}, calls} ->
        Process.demonitor(monitor, [:flush])
        if cancel, do: command(data.port, {:cancel, id})
        {:next_state, if(calls == %{}, do: :ready, else: state), %{data | calls: calls}}

      {nil, _calls} ->
        :keep_state_and_data
    end
  end

  # Its VM stopped, or is stopped: the calls it was running fail, and one still loading fails whatever waits on it.
  defp stop(state, data, why) do
    Logger.warning("#{data.context.name}'s sandbox stopped: #{why}")
    for {_id, {caller, _monitor}} <- data.calls, do: send(caller, {__MODULE__, :stopped, self(), why})

    if state == :loading,
      do: {:next_state, {:failed, "#{data.context.name} stopped as it loaded: #{why}"}, data},
      else: {:stop, :normal, data}
  end

  # Its VM's own words, such as that it ran out of memory, aren't a frame either.
  defp frames(state, %{buffer: <<size::32, _::binary>>} = data) when size > @largest,
    do: stop(state, data, "it wrote what isn't the protocol: #{inspect(String.slice(data.buffer, 0, 200))}")

  defp frames(state, %{buffer: <<size::32, frame::binary-size(size), rest::binary>>} = data) do
    case JSON.decode(frame) do
      {:ok, message} ->
        case received(message, state, %{data | buffer: rest}) do
          {:ok, state, data} -> frames(state, data)
          {:stop, why, data} -> stop(state, data, why)
        end

      {:error, _} ->
        stop(state, data, "it sent what isn't the protocol: #{inspect(String.slice(frame, 0, 200))}")
    end
  end

  defp frames(state, data), do: {:next_state, state, data}

  # It answers what it is once, as it loads, before any call, so what it compiled holds nothing of anyone's.
  defp received(%{"answer" => "load"} = reply, :loading, data) do
    case reply do
      %{"ok" => description} when is_map(description) ->
        {beams, description} = Map.pop(description, "beams")
        if beams && data.context.cache, do: keep(data.context.cache, beams)
        {:ok, :ready, %{data | context: %{data.context | description: description}}}

      _ ->
        Logger.warning("#{data.context.name}'s sandbox stopped: it didn't load")

        why =
          if is_binary(reply["raised"]), do: reply["raised"], else: "#{data.context.name} said what isn't what it is"

        {:ok, {:failed, why}, data}
    end
  end

  defp received(%{"answer" => "load"}, _state, data), do: {:stop, "it said what it is again", data}

  defp received(%{"answer" => id} = reply, state, data) when is_integer(id) do
    with %{^id => {caller, _monitor}} <- data.calls, do: send(caller, {__MODULE__, :reply, id, reply})
    {:ok, state, data}
  end

  # A request within a call is run by the process running the call, and any other alongside, a few at once.
  defp received(%{"request" => request, "call" => call, "op" => op} = message, state, data)
       when is_integer(request) and is_binary(op) do
    cond do
      is_map_key(data.calls, call) ->
        {caller, _monitor} = data.calls[call]
        send(caller, {__MODULE__, :request, call, message})
        {:ok, state, data}

      data.requests >= @at_once ->
        {:stop, "it asked for more than #{@at_once} things at once outside a call", data}

      true ->
        {port, context, child} = {data.port, data.context, self()}

        Task.start(fn ->
          command(port, {:reply, request, handled(message, nil, context)})
          send(child, :replied)
        end)

        {:ok, state, %{data | requests: data.requests + 1}}
    end
  end

  # What it logs past its share of a minute is dropped.
  defp received(%{"log" => level, "text" => text}, state, data)
       when level in ["warning", "error", "critical", "alert", "emergency"] and is_binary(text) do
    minute = div(System.monotonic_time(:second), 60)
    {at, logged} = data.logged
    logged = if at == minute, do: logged + 1, else: 1

    if logged <= @logs,
      do: Logger.log(String.to_existing_atom(level), "#{data.context.name}: #{String.slice(text, 0, 4000)}")

    {:ok, state, %{data | logged: {minute, logged}}}
  end

  defp received(message, _state, data),
    do: {:stop, "it sent what isn't the protocol: #{inspect(message, limit: 5)}", data}

  # Keeps what a VM compiled, each module by its name, for the next VM of the same version. It's written aside and
  # moved into place, so a VM never loads half of it, and what another VM kept first stays.
  defp keep(cache, beams) do
    writing = "#{cache}.#{System.unique_integer([:positive])}"
    File.mkdir_p!(writing)

    modules =
      for {name, mods} <- beams, is_map(mods), into: %{} do
        written =
          for {module, data} <- mods, is_binary(module) and module =~ ~r/^[A-Za-z0-9_.]+\z/ and is_binary(data) do
            File.write!(Path.join(writing, module <> ".beam"), Base.decode64!(data))
            module
          end

        {name, written}
      end

    File.write!(Path.join(writing, "modules"), JSON.encode!(Map.delete(modules, "runtime")))
    with {:error, _} <- File.rename(writing, cache), do: File.rm_rf!(writing)

    # What no VM has been started from for a month goes: its version, or this server's build, is long gone.
    month_ago = System.os_time(:second) - 30 * 24 * 3600

    for folder <- Path.wildcard(Path.join(Path.dirname(cache), "*")),
        {:ok, %{mtime: used}} <- [File.stat(folder, time: :posix)],
        used < month_ago,
        do: File.rm_rf!(folder)
  rescue
    error -> Logger.warning("What an app's VM compiled wasn't kept: #{Exception.message(error)}")
  end
end
