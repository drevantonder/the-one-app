defmodule OneCore.Sandbox do
  @moduledoc """
  Sandbox mode: a plugin version whose code runs in an Erlang VM of its own, a child of this server's, behind this
  machine's process sandbox, never in this server's VM. Any of the network's servers runs one for anyone, since
  what it reaches is only what this server hands it. The wall is the plugin `sandbox`, which One Core asks one
  thing of: start this program under this policy and give me its pipes (`Sandbox.start/3`). Without it, no plugin
  runs in sandbox mode, and a call to one says why.

  There's one child for each version of a sandboxed plugin on a server and each user, group or network it's
  installed for, with the versions of the plugins it uses, which run inside it as it (`database` among them, its
  SQLite too). So a child only ever runs one owner's calls, and what a stranger's plugin keeps from one person's call
  isn't there in another's: its handles are its own, and a request for an artifact or a call that isn't its own is
  refused. It's started by the first call, compiles the code itself, behind the wall, and stops after a minute
  idle. One that stops, or is stopped, fails the calls it was running and nothing else; the next call starts
  another. What the first child of a version compiled is kept on this machine, and every owner's next child loads it
  read-only instead.

  The protocol, over the child's standard input and output, each message a frame of a 4-byte length:

  - This server sends Erlang terms, which the child trusts: `{:load, sets, target}`, the plugins' files to compile,
    those it uses first; `{:call, id, what, about}`, a tool, a hook heard, a schedule to run or what `server-debug`
    asks (`debug/5`), with who it's for;
    `{:reply, request, result}`; and `{:cancel, id}`.
  - The child sends JSON, which never reaches `binary_to_term`: `{"answer": id, "ok" | "raised": ...}`,
    `{"request": n, "call": id, "op": ..., ...}`, one of the functions below, and `{"log": level, "text": ...}`.
    A frame over 64 MB, or that isn't one of these, stops it.

  What it's offered is what the plugin interface offers, as messages: `OneCore.Artifact`'s read, read_all, write,
  replace, remove, list, size and name; `OneCore.Call`'s tool, make, reach, reaches?, member?, owns?, plugins,
  deployed, process (its own VM's) and forward_to; `OneCore.Plugin.file/2` and `nif/2`; Logger; and `fetch`, a
  web request this server makes for it. An artifact it's given is a handle this server looks up, never a prefix
  it could rewrite, and a call is one this server is running, never a subject, actor or caller it could name. It hooks
  only into what it observes: a mutate, validate or collect hook asks it nothing. It has no services, no other plugins'
  functions, no network, no programs, and no files but a folder of its own that goes when it stops.

  What it may call of other plugins, and where it may send data, are its permissions (`OneCore.Plugin`): a tool
  call or a fetch outside them is refused.
  """
  use GenServer, restart: :temporary
  require Logger
  alias OneCore.{Artifact, Bucket, Builds, Call, Plugins, Server}
  alias OneCore.Plugins.{Calls, Version}

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
  What a set of a sandboxed plugin's files says of itself, from a child that compiles only them: `{:ok, version}`
  of the plugin `artifact`, or `{:error, why}`. The files are as the server keeps them.
  """
  def describe(artifact, files) do
    name = Path.basename(artifact)
    sets = [{name, Map.reject(files, fn {path, _} -> Version.client?(path) end)}]

    with {:ok, said} <- once({:describe, name, Version.code(files)}, name, sets),
         do: Version.sandboxed(artifact, files, said)
  end

  @doc """
  Checks a sandboxed version, from its `files` as the server keeps them, as it will run, with the plugins it uses as
  `owner` finds them: `{:ok, warnings}`, or `{:error, why}` with each thing it uses that sandbox mode doesn't offer,
  a sentence each with its file and line.
  """
  def check(owner, %Version{} = version, files) do
    with {:ok, used} <- used(owner, version),
         {:ok, %{"problems" => problems, "warnings" => warnings}} <- loaded(owner, version, used, files) do
      case problems do
        [] ->
          {:ok, warnings}

        _ ->
          {:error,
           "In sandbox mode, #{Path.basename(version.artifact)} can't run as it is:\n" <> Enum.join(problems, "\n")}
      end
    end
  end

  defp used(owner, version) do
    case Plugins.used(owner, version.uses) do
      {:ok, used} ->
        {:ok, used}

      {:absent, missing} ->
        {:error, "#{Path.basename(version.artifact)} uses #{missing}, which isn't installed for #{owner}"}
    end
  end

  @doc """
  Runs `program` with `arguments` to its end behind this machine's sandbox, under `policy` as the plugin `sandbox`
  takes it: what a sandboxed plugin's app is built with, say. Gives `{output, status}`, or `{:error, why}` where it
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

  # Starts a program behind the wall, which the plugin `sandbox` is, with its pipes as a port this process owns.
  defp walled(program, arguments, policy) do
    case Plugins.run(["network"], "sandbox", fn _at, versions ->
           versions["sandbox"].plugin.start(program, arguments, policy)
         end) do
      {:ok, port} ->
        {:ok, port}

      {:error, "There's no plugin named sandbox"} ->
        {:error, "Sandbox mode needs the plugin sandbox, which isn't installed for the network"}

      {:error, why} ->
        {:error, why}
    end
  end

  @doc """
  Runs `what` of the plugin `name` among `versions` in its child: `{:tool, tool, inputs}`, `{:hook, hook, about}` or
  `{:schedule, jobs}`, for `call`. Gives what the plugin answered, or raises with why it didn't.
  """
  def call(versions, name, what, %Call{} = call) do
    version = versions[name]

    {child, port, context} =
      case child(call.installed, version, Map.delete(versions, name)) do
        {:ok, child, port, context} -> {child, port, context}
        {:error, why} -> raise why
      end

    id = System.unique_integer([:positive])
    :ok = GenServer.call(child, {:begin, id})

    try do
      about = %{
        "own" => [handle(context, call.own), Artifact.name(call.own)],
        "installed" => call.installed,
        "subject" => call.subject,
        "actor" => call.actor,
        "caller" => call.caller,
        "plugin" => call.plugin,
        "who" => call.who,
        "server" => call.server,
        "uses" => Map.keys(versions) -- [name]
      }

      command(port, {:call, id, what, about})
      waited(child, port, id, call, context)
    after
      GenServer.cast(child, {:ended, id})
    end
  end

  @doc """
  Asks the VM of the plugin `name` among `versions`, the one for `owner`, what the plugin `server-debug` asks of it:
  `request`, a map JSON holds, which its side of One Core answers (`OneCore.Sandbox.Child.Debug`). Gives what it
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
      plugin: name,
      who: version.artifact,
      request: nil,
      uses: %{},
      forwarded: false
    }

    call(versions, name, {:debug, request}, call)
  end

  defp waited(child, port, id, call, context) do
    receive do
      {__MODULE__, :request, ^id, request} ->
        command(port, {:reply, request["request"], handled(request, call, context)})
        waited(child, port, id, call, context)

      {__MODULE__, :answer, ^id, answer} ->
        answered(answer, context)

      {__MODULE__, :stopped, ^child, why} ->
        raise "#{context.name} stopped while it ran this call: #{why}"
    after
      @silent ->
        GenServer.cast(child, {:cancel, id})
        raise "#{context.name} said nothing for #{div(@silent, 1000)} seconds, so its call was stopped"
    end
  end

  defp answered(%{"ok" => %{"text" => text}}, _context) when is_binary(text), do: text
  defp answered(%{"ok" => %{"json" => json}}, _context), do: json

  defp answered(%{"ok" => %{"blob" => [kind, data]}}, _context) when is_binary(kind) and is_binary(data),
    do: {:blob, kind, Base.decode64!(data)}

  defp answered(%{"ok" => %{"file" => [kind, handle, name]}}, context) when is_binary(kind) and is_binary(name),
    do: {:file, kind, artifact!(context, handle), name}

  defp answered(%{"ok" => nil}, _context), do: nil
  defp answered(%{"raised" => why}, _context) when is_binary(why), do: raise(String.slice(why, 0, 8000))
  defp answered(_answer, context), do: raise("#{context.name} answered what isn't an answer")

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

  defp op("fetch", %{"web" => request}, _call, context), do: fetch(request, context)
  defp op("fire", request, _call, context), do: fire(request, context)
  defp op(op, _request, nil, _context), do: raise("#{op} is asked within a call, and that call has ended")

  defp op("tool", %{"plugin" => plugin, "tool" => tool, "inputs" => inputs} = request, call, context)
       when is_binary(plugin) and is_binary(tool) and is_map(inputs) do
    if not permitted?(context.permissions, plugin, tool),
      do:
        raise(
          "#{context.name} may not call #{plugin}_#{tool}: its permissions/0 doesn't name it. A plugin in sandbox " <>
            "mode calls only the tools it says it uses."
        )

    options = if is_map_key(request, "on_behalf_of"), do: [on_behalf_of: request["on_behalf_of"]], else: []
    Call.tool(call, plugin, tool, inputs, options)
  end

  defp op("make", %{"id" => made}, call, context), do: named(context, Call.make(call, made))

  defp op("reach", %{"name" => name, "relation" => relation}, call, context),
    do: named(context, Call.reach(call, artifact_name!(name), relation!(relation)))

  defp op("reaches", %{"name" => name, "relation" => relation}, call, _context),
    do: Call.reaches?(call, artifact_name!(name), relation!(relation))

  defp op("member", %{"of" => of}, call, _context) when is_binary(of), do: Call.member?(call, of)
  defp op("owns", _request, call, _context), do: Call.owns?(call)
  defp op("plugins", _request, call, _context), do: Call.plugins(call)
  defp op("deployed", _request, call, _context), do: Call.deployed(call)
  defp op("forward_to", %{"server" => server}, call, _context) when is_binary(server), do: Call.forward_to(call, server)
  defp op(op, _request, _call, _context), do: raise("#{inspect(op)} isn't offered in sandbox mode")

  defp named(context, artifact), do: [handle(context, artifact), Artifact.name(artifact)]

  # A blob's name in an artifact, which can't step outside it: parts joined by /, none starting with a dot.
  defp blob!(name) do
    if is_binary(name) and name != "" and Enum.all?(String.split(name, "/"), &(&1 =~ ~r/^[^.]/)),
      do: name,
      else: raise(ArgumentError, "a blob's name is parts joined by /, none empty or starting with a dot")
  end

  defp listed!(""), do: ""

  defp listed!(starting) when is_binary(starting) do
    parts = String.split(starting, "/")

    if Enum.all?(Enum.drop(parts, -1), &(&1 =~ ~r/^[^.]/)) and not String.starts_with?(List.last(parts), "."),
      do: starting,
      else: raise(ArgumentError, "what's listed starts with parts joined by /, none starting with a dot")
  end

  defp artifact_name!(name) when is_binary(name), do: name
  defp relation!(relation) when relation in ["viewer", "editor", "owner"], do: relation
  defp relation!(_), do: raise(ArgumentError, "a relation is viewer, editor or owner")

  @doc """
  Whether a plugin's `permissions` let it call `plugin`'s `tool`: they name the tool (`gmail_search`) or the
  plugin, all its tools.
  """
  def permitted?(permissions, plugin, tool), do: plugin in permissions or "#{plugin}_#{tool}" in permissions

  @doc "Whether a plugin's `permissions` let it send data to `url`: they name its host (`https://api.example.com`)."
  def sends_to?(permissions, url) do
    %URI{scheme: scheme, host: host} = URI.parse(url)

    scheme in ["https", "http"] and is_binary(host) and
      Enum.any?(permissions, fn permission ->
        case URI.parse(permission) do
          %URI{scheme: ^scheme, host: "*." <> domain, path: nil} -> String.ends_with?(host, "." <> domain)
          %URI{scheme: ^scheme, host: ^host, path: nil} -> true
          _ -> false
        end
      end)
  end

  # A redirect isn't followed, so one host can't send it on to another.
  defp fetch(%{"url" => url} = request, context) when is_binary(url) do
    if not sends_to?(context.permissions, url),
      do:
        raise(
          "#{context.name} may not send data to #{url}: its permissions/0 doesn't name its host. A plugin in " <>
            "sandbox mode fetches only from the hosts it says it sends data to."
        )

    body = with body when is_binary(body) <- request["body"], do: Base.decode64!(body)
    Call.fetch(nil, Map.put(request, "body", body))
  end

  # A scheduled job's call, which the plugin's schedule keeper in the child makes as the clock, for whoever it's
  # installed for, as One Core would: only one of the plugin's own jobs, for the owner whose child it is.
  defp fire(%{"installed" => at, "plugin" => name, "tool" => tool, "inputs" => inputs}, context) do
    with true <- at == context.owner,
         %{^name => %Version{artifact: artifact}} <- Plugins.versions([at]),
         true <- artifact == context.artifact,
         %{} <- Enum.find(context.jobs, &(&1.tool == tool and &1.inputs == inputs)) do
      with {:error, why} <- Calls.call(name, tool, inputs, {nil, at}, true),
           do: Logger.error("#{name}'s scheduled #{tool} for #{at} failed: #{why}")

      OneCore.Server.changed()
      nil
    else
      _ -> raise "That isn't one of #{context.name}'s scheduled jobs for #{at}"
    end
  end

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
      [] -> raise "That isn't an artifact this plugin was given"
    end
  end

  defp artifact!(_context, _handle), do: raise("That isn't an artifact this plugin was given")

  defp command(port, term) do
    frame = :erlang.term_to_binary(term)
    Port.command(port, [<<byte_size(frame)::32>>, frame])
  end

  # The child of a version with the versions it uses, for whoever it's installed for, `owner`, started if there's
  # none: `{:ok, child, port, context}`, or `{:error, why}` with why it couldn't start or load.
  defp child(owner, version, used, files \\ nil) do
    key = key(version, used)

    child =
      case Registry.lookup(@registry, {owner, key}) do
        [{child, _}] ->
          child

        [] ->
          sets = fn -> sets(version, used, files) end
          spec = {__MODULE__, {owner, key, Path.basename(version.artifact), sets, version}}

          case DynamicSupervisor.start_child(@supervisor, spec) do
            {:ok, child} -> child
            {:error, {:already_started, child}} -> child
          end
      end

    GenServer.call(child, :loaded, :infinity)
  catch
    :exit, _ -> {:error, "#{Path.basename(version.artifact)} stopped as it started"}
  end

  # What tells a version's child from another's: its name and code, and those of the versions it uses. Two plugins
  # of the same files are two plugins, each with its VM and what it compiled.
  defp key(version, used),
    do: {Path.basename(version.artifact), version.code, Enum.sort(for {name, used} <- used, do: {name, used.code})}

  # A child that runs once, to say what a set of files is, and stops.
  defp once(key, name, sets) do
    {:ok, child} = DynamicSupervisor.start_child(@supervisor, {__MODULE__, {nil, key, name, fn -> sets end, nil}})

    try do
      with {:ok, _child, _port, context} <- GenServer.call(child, :loaded, :infinity), do: {:ok, context.said}
    after
      DynamicSupervisor.terminate_child(@supervisor, child)
    end
  catch
    :exit, _ -> {:error, "#{name} stopped as it started"}
  end

  defp loaded(owner, version, used, files) do
    case child(owner, version, used, files) do
      {:ok, _child, _port, context} -> {:ok, context.said}
      {:error, why} -> {:error, why}
    end
  end

  # The files of each plugin, those it uses first, from the bucket but the plugin's own where they're given: what
  # isn't in client/, and of its NIFs' libraries only this machine's.
  defp sets(version, used, files) do
    own = Path.basename(version.artifact)

    for {name, %Version{artifact: artifact, code: code}} <- Enum.to_list(used) ++ [{own, version}] do
      prefix = "#{artifact}/code/#{code}/"

      found =
        if name == own and files,
          do: files,
          else: Map.new(Bucket.list(prefix), &{String.replace_prefix(&1, prefix, ""), &1})

      paths =
        for {path, _} <- found,
            not Version.client?(path),
            Path.extname(path) != ".so" or Path.basename(path) == "#{Builds.target()}.so",
            do: path

      read =
        if name == own and files,
          do: Map.take(files, paths),
          else:
            for(
              {path, {:ok, data, _tag}} <- Enum.zip(paths, Bucket.read_all(Enum.map(paths, &found[&1]))),
              into: %{},
              do: {path, data}
            )

      {name, read}
    end
  end

  def start_link({owner, key, _name, _sets, _version} = started),
    do: GenServer.start_link(__MODULE__, started, name: {:via, Registry, {@registry, {owner, key}}})

  @impl true
  def init({owner, key, name, sets, version}) do
    handles = :ets.new(__MODULE__, [:public, read_concurrency: true])

    # What a version's VM compiled is kept on this machine for its next VM, whoever's it is, which loads it instead:
    # its own code and the child's side of One Core, as this server has it.
    runtime = for file <- ~w(child.ex redact.ex), do: File.read!(Application.app_dir(:one_core, "priv/sandbox/#{file}"))
    cache = if version, do: Path.join([Server.home(), "sandbox", Version.code({key, runtime})])

    context = %{
      cache: cache,
      owner: owner,
      name: name,
      handles: handles,
      artifact: version && version.artifact,
      permissions: (version && version.permissions) || [],
      jobs: (version && version.jobs) || [],
      said: nil
    }

    state = %{context: context, port: nil, buffer: "", loaded: nil, waiting: [], calls: %{}, requests: 0, logged: {0, 0}}
    {:ok, state, {:continue, {:start, sets}}}
  end

  @impl true
  def handle_continue({:start, sets}, state) do
    # A VM started from what's kept marks it as in use.
    cached =
      with path when is_binary(path) <- state.context.cache,
           {:ok, said} <- File.read(Path.join(path, "modules")) do
        File.touch(path)
        JSON.decode!(said)
      else
        _ -> nil
      end

    case started(cached && state.context.cache) do
      {:ok, port} ->
        cache = cached && %{"ebin" => state.context.cache, "modules" => cached}
        command(port, {:load, sets.(), Builds.target(), cache})
        Process.send_after(self(), :loading, @silent)
        # Only a VM that compiled the code itself, before any call, has what it compiled kept.
        context = if cached, do: %{state.context | cache: nil}, else: state.context
        {:noreply, %{state | port: port, context: context}}

      {:error, why} ->
        {:stop, :normal, answer_loaded(state, {:error, why})}
    end
  end

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

    # Erlang's and Elixir's applications, and the server's libraries, but not the server's own code, which an
    # installed server's release keeps beside them.
    # Recon, which a plugin is debugged with (`debug/5`), is among them in a release, and is added where it isn't.
    apps =
      for ebin <- Enum.uniq(Path.wildcard(Path.join(root, "lib/*/ebin")) ++ Path.wildcard(Path.join(elixir, "*/ebin"))),
          not String.starts_with?(Path.basename(Path.dirname(ebin)), "one_core-"),
          do: Path.dirname(ebin)

    apps = Enum.uniq(apps ++ [to_string(:code.lib_dir(:recon))])

    ebins = for app <- apps, do: Path.join(app, "ebin")

    # Its own side of One Core is compiled as it starts, unless it was kept.
    compile =
      if cache,
        do: "",
        else:
          "{ok, _, _} = 'Elixir.Kernel.ParallelCompiler':compile_to_path([<<\"#{child}/child.ex\"/utf8>>, " <>
            "<<\"#{child}/redact.ex\"/utf8>>], " <>
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
      "EMU" => "beam",
      "PROGNAME" => "erl",
      "HOME" => "/tmp",
      "LANG" => "C.UTF-8",
      "ERL_CRASH_DUMP_SECONDS" => "0"
    }

    policy = %{read: Enum.uniq([erts, boot <> ".boot", child | apps] ++ List.wrap(cache)), env: env, memory: @memory}

    # The VM's own program, which on Windows is `erl.exe`, with the VM in it rather than started by it.
    vm = if match?({:win32, _}, :os.type()), do: "bin/erl.exe", else: "bin/erlexec"
    walled(Path.join(erts, vm), arguments, policy)
  end

  @impl true
  def handle_call(:loaded, from, %{loaded: nil} = state), do: {:noreply, %{state | waiting: [from | state.waiting]}}
  def handle_call(:loaded, _from, state), do: {:reply, state.loaded, state}

  def handle_call({:begin, id}, {caller, _}, state) do
    monitor = Process.monitor(caller)
    {:reply, :ok, %{state | calls: Map.put(state.calls, id, {caller, monitor})}}
  end

  @impl true
  def handle_cast({:ended, id}, state), do: {:noreply, ended(state, id, false)}
  def handle_cast({:cancel, id}, state), do: {:noreply, ended(state, id, true)}

  @impl true
  def handle_info({port, {:data, data}}, %{port: port} = state), do: frames(%{state | buffer: state.buffer <> data})

  def handle_info({port, {:exit_status, status}}, %{port: port} = state),
    do: {:stop, :normal, stopped(state, "its VM ended with status #{status}")}

  def handle_info({:DOWN, monitor, :process, _caller, _why}, state) do
    case Enum.find(state.calls, fn {_id, {_caller, watched}} -> watched == monitor end) do
      {id, _} -> {:noreply, ended(state, id, true)}
      nil -> {:noreply, state}
    end
  end

  def handle_info(:idle, %{calls: calls} = state) when calls == %{}, do: {:stop, :normal, state}
  def handle_info(:idle, state), do: {:noreply, state}

  def handle_info(:loading, %{loaded: nil} = state),
    do: {:stop, :normal, stopped(state, "it didn't load within #{div(@silent, 1000)} seconds")}

  def handle_info(:loading, state), do: {:noreply, state}
  def handle_info(:answered, state), do: {:noreply, %{state | requests: state.requests - 1}}

  @impl true
  def terminate(_why, %{port: port}) when is_port(port) do
    kill(port)
    Port.close(port)
  catch
    _, _ -> :ok
  end

  def terminate(_why, _state), do: :ok

  # A call ended, here or in the child: one the child may still be running is cancelled there. A child with none
  # left stops after a while.
  defp ended(state, id, cancel) do
    case Map.pop(state.calls, id) do
      {{_caller, monitor}, calls} ->
        Process.demonitor(monitor, [:flush])
        if cancel, do: command(state.port, {:cancel, id})
        if calls == %{}, do: Process.send_after(self(), :idle, @idle)
        %{state | calls: calls}

      {nil, _calls} ->
        state
    end
  end

  # Its VM's own words, such as that it ran out of memory, aren't a frame either.
  defp frames(%{buffer: <<size::32, _::binary>>} = state) when size > @largest,
    do:
      {:stop, :normal,
       stopped(state, "it wrote what isn't the protocol: #{inspect(String.slice(state.buffer, 0, 200))}")}

  defp frames(%{buffer: <<size::32, frame::binary-size(size), rest::binary>>} = state) do
    case JSON.decode(frame) do
      {:ok, message} ->
        case received(message, %{state | buffer: rest}) do
          {:ok, state} -> frames(state)
          {:stop, why, state} -> {:stop, :normal, stopped(state, why)}
        end

      {:error, _} ->
        {:stop, :normal, stopped(state, "it sent what isn't the protocol: #{inspect(String.slice(frame, 0, 200))}")}
    end
  end

  defp frames(state), do: {:noreply, state}

  # It answers what it is once, before any call, so what it compiled holds nothing of anyone's.
  defp received(%{"answer" => "load"}, %{loaded: loaded} = state) when loaded != nil,
    do: {:stop, "it said what it is again", state}

  defp received(%{"answer" => "load"} = answer, state) do
    loaded =
      case answer do
        %{"ok" => said} when is_map(said) ->
          {beams, said} = Map.pop(said, "beams")
          if beams && state.context.cache, do: keep(state.context.cache, beams)
          context = %{state.context | said: said}
          {:ok, self(), state.port, context}

        %{"raised" => why} when is_binary(why) ->
          {:error, why}

        _ ->
          {:error, "#{state.context.name} said what isn't what it is"}
      end

    state = answer_loaded(state, loaded)
    state = with {:ok, _, _, context} <- loaded, do: %{state | context: context}
    if match?({:error, _}, loaded), do: {:stop, "it didn't load", state}, else: {:ok, state}
  end

  defp received(%{"answer" => id} = answer, state) when is_integer(id) do
    with %{^id => {caller, _monitor}} <- state.calls, do: send(caller, {__MODULE__, :answer, id, answer})
    {:ok, state}
  end

  # A request within a call is run by the process running the call, and any other alongside, a few at once.
  defp received(%{"request" => request, "call" => call, "op" => op} = message, state)
       when is_integer(request) and is_binary(op) do
    cond do
      is_map_key(state.calls, call) ->
        {caller, _monitor} = state.calls[call]
        send(caller, {__MODULE__, :request, call, message})
        {:ok, state}

      state.requests >= @at_once ->
        {:stop, "it asked for more than #{@at_once} things at once outside a call", state}

      true ->
        {port, context, child} = {state.port, state.context, self()}

        Task.start(fn ->
          command(port, {:reply, request, handled(message, nil, context)})
          send(child, :answered)
        end)

        {:ok, %{state | requests: state.requests + 1}}
    end
  end

  # What it logs past its share of a minute is dropped.
  defp received(%{"log" => level, "text" => text}, state)
       when level in ["warning", "error", "critical", "alert", "emergency"] and is_binary(text) do
    minute = div(System.monotonic_time(:second), 60)
    {at, logged} = state.logged
    logged = if at == minute, do: logged + 1, else: 1

    if logged <= @logs,
      do: Logger.log(String.to_existing_atom(level), "#{state.context.name}: #{String.slice(text, 0, 4000)}")

    {:ok, %{state | logged: {minute, logged}}}
  end

  defp received(message, state), do: {:stop, "it sent what isn't the protocol: #{inspect(message, limit: 5)}", state}

  # Keeps what a VM compiled, each module by its name, for the next VM of the same version. It's written aside and
  # moved into place, so a VM never loads half of it, and what another VM kept first stays.
  defp keep(cache, beams) do
    writing = "#{cache}.#{System.unique_integer([:positive])}"
    File.mkdir_p!(writing)

    modules =
      for {name, mods} <- beams, is_map(mods), into: %{} do
        written =
          for {module, data} <- mods, is_binary(module) and module =~ ~r/^[A-Za-z0-9_.]+$/ and is_binary(data) do
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
    error -> Logger.warning("What a plugin's VM compiled wasn't kept: #{Exception.message(error)}")
  end

  defp answer_loaded(state, loaded) do
    for from <- state.waiting, do: GenServer.reply(from, loaded)
    %{state | loaded: loaded, waiting: []}
  end

  defp stopped(state, why) do
    Logger.warning("#{state.context.name}'s sandbox stopped: #{why}")
    for {_id, {caller, _monitor}} <- state.calls, do: send(caller, {__MODULE__, :stopped, self(), why})

    if state.loaded == nil,
      do: answer_loaded(state, {:error, "#{state.context.name} stopped as it loaded: #{why}"}),
      else: state
  end
end
