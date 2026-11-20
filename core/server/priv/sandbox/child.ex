# One Core inside an untrusted app's VM, in its sandbox (`OneCore.Sandbox`, which says the protocol). The app is
# compiled against these modules, which have the names and functions of the server's own, and each of their
# functions is a message to the server. This file is compiled by the VM it runs in, behind the wall.

defmodule OneCore.Call do
  @moduledoc false
  # What a tool is called with besides its inputs, as the server's own (`OneCore.Call`), whose functions here ask
  # the server. `id` is the call's, which the server knows it by: a call that has ended asks nothing more.
  alias OneCore.Sandbox.Child

  defstruct [:id, :own, :installed, :folder, :server, :subject, :actor, :caller, :app, :app_artifact, :loaded]

  def app(%__MODULE__{loaded: loaded, app: app}, name) do
    case loaded do
      %{^name => module} ->
        module

      _ ->
        raise "#{name}'s code doesn't run inside #{app}: it does where #{app} requires it (requires/0), it's " <>
                "installed for whoever #{app} runs for, its module provides functions, and its code can run in a " <>
                "sandbox. Otherwise call its tools."
    end
  end

  def tool(%__MODULE__{id: id}, app, tool, inputs, options \\ []) do
    request = %{"app" => app, "tool" => tool, "inputs" => inputs}

    request =
      if Keyword.has_key?(options, :on_behalf_of),
        do: Map.put(request, "on_behalf_of", options[:on_behalf_of]),
        else: request

    case Child.request("tool", request, id) do
      {:blob, kind, data} -> {:blob, kind, data}
      result -> result
    end
  end

  def make(%__MODULE__{id: call}, id), do: Child.artifact(Child.request("make", %{"id" => id}, call))

  def reach(%__MODULE__{id: id}, name, relation \\ "viewer"),
    do: Child.artifact(Child.request("reach", %{"name" => name, "relation" => relation}, id))

  def reaches?(%__MODULE__{id: id}, name, relation \\ "viewer"),
    do: Child.request("reaches", %{"name" => name, "relation" => relation}, id)

  def member?(%__MODULE__{id: id}, of), do: Child.request("member", %{"of" => of}, id)
  def owns?(%__MODULE__{id: id}), do: Child.request("owns", %{}, id)
  def apps(%__MODULE__{id: id}), do: Child.request("apps", %{}, id)
  def deployed(%__MODULE__{id: id}), do: Child.request("deployed", %{}, id)

  @doc "Tells every app watching that something changed, as the server's own does."
  def changed(%__MODULE__{id: id}), do: Child.request("changed", %{}, id)

  @doc "Says what the call is doing, as the server's own does."
  def stage(%__MODULE__{id: id}, text) when is_binary(text), do: Child.request("stage", %{"text" => text}, id)

  @doc "Asks the person the call is for `question`, and gives their answer, as the server's own does."
  def ask(%__MODULE__{id: id}, question), do: Child.request("ask", %{"question" => question}, id)

  @doc "Has the server holding what this call needs run it instead: it ends here if that server runs it."
  def forward_to(%__MODULE__{id: id}, server), do: Child.request("forward_to", %{"server" => server}, id)

  @doc """
  A process of this VM's that keeps running between calls: one for each `module` and `key`, a GenServer or a
  `:gen_statem`.
  """
  def process(%__MODULE__{}, module, key, argument) do
    name = {:via, Registry, {Child.Processes, {module, key}}}

    started =
      if :gen_statem in Child.behaviours(module),
        do: :gen_statem.start(name, module, argument, []),
        else: GenServer.start(module, argument, name: name)

    case started do
      {:ok, pid} -> pid
      {:error, {:already_started, pid}} -> pid
      {:error, why} -> raise "#{inspect(module)} didn't start: #{inspect(why)}"
    end
  end
end

defmodule OneCore.Artifact do
  @moduledoc false
  # An artifact as an untrusted app holds it: a handle the server gave, which it looks up, and its name.
  alias OneCore.Sandbox.Child

  defstruct [:handle, :prefix, :codec]

  def name(%__MODULE__{prefix: prefix}), do: String.trim_trailing(prefix, "/")
  def read(%__MODULE__{handle: handle}, name), do: Child.request("read", %{"handle" => handle, "name" => name})

  def read_all(%__MODULE__{handle: handle}, names),
    do: Child.request("read_all", %{"handle" => handle, "names" => names})

  def write(%__MODULE__{handle: handle}, name, data),
    do: Child.request("write", %{"handle" => handle, "name" => name, "data" => Base.encode64(data)})

  def replace(%__MODULE__{handle: handle}, name, data, tag),
    do: Child.request("replace", %{"handle" => handle, "name" => name, "data" => Base.encode64(data), "tag" => tag})

  def remove(%__MODULE__{handle: handle}, name), do: Child.request("remove", %{"handle" => handle, "name" => name})
  def size(%__MODULE__{handle: handle}, name), do: Child.request("size", %{"handle" => handle, "name" => name})

  def list(%__MODULE__{handle: handle}, starting \\ ""),
    do: Child.request("list", %{"handle" => handle, "starting" => starting})
end

defmodule OneCore.App do
  @moduledoc false
  # The interface an app is written against (the server's `OneCore.App` says it all).

  def file(module, path) do
    {name, _out, files} =
      :persistent_term.get({OneCore.Sandbox.Child, :files, module}, nil) || raise "#{inspect(module)} is no app's"

    if String.ends_with?(path, ".ex") or path == "SKILL.md" or String.starts_with?(path, ["client/", "skills/"]),
      do: raise("#{name}'s app has no file #{path} outside client/"),
      else: files[path] || raise("#{name}'s app has no file #{path} outside client/")
  end

  require OneCore.App.Manifest
  OneCore.App.Manifest.callbacks()
end

defmodule OneCore.Service do
  @moduledoc false
  require OneCore.App.Manifest
  OneCore.App.Manifest.service_callbacks()
end

defmodule OneCore.Sandbox.Child do
  @moduledoc false

  # Speaks to the server over standard input and output: what the server sends is Erlang's external term format,
  # and what this sends is JSON, each in frames of a 4-byte length.
  def main do
    Process.register(self(), __MODULE__)
    sink = spawn(&sink/0)
    # Whatever the app writes as output or errors would land in the protocol: it's dropped.
    Process.group_leader(self(), sink)
    with pid when is_pid(pid) <- Process.whereis(:standard_error), do: Process.unregister(:standard_error)
    Process.register(sink, :standard_error)
    :logger.remove_handler(:default)
    # Every level is said, as on a server, and only warnings and errors reach the server's log.
    :logger.set_primary_config(:level, :all)
    :logger.add_handler(:server, __MODULE__.Logged, %{level: :warning})
    OneCore.Redact.install()
    {:ok, _} = Registry.start_link(keys: :unique, name: __MODULE__.Processes)
    port = Port.open({:fd, 0, 1}, [:binary, :eof, packet: 4])
    loop(%{port: port, waiting: %{}, calls: %{}, next: 0, loaded: %{}})
  end

  @doc false
  def send_json(message), do: send(__MODULE__, {:send, JSON.encode!(message)})

  @doc "Asks the server `op` with `args`, for the call `call` (its id, or nil), and gives its answer, or raises why."
  def request(op, args, call \\ Process.get(:"$one_call")) do
    ref = make_ref()
    send(__MODULE__, {:request, self(), ref, Map.merge(args, %{"op" => op, "call" => call})})

    receive do
      {^ref, {:ok, result}} -> result
      {^ref, {:raise, why}} -> raise why
    end
  end

  defp loop(state) do
    receive do
      {port, {:data, frame}} when port == state.port ->
        loop(received(:erlang.binary_to_term(frame), state))

      {:request, from, ref, request} ->
        rid = state.next
        Port.command(state.port, JSON.encode!(Map.put(request, "request", rid)))
        loop(%{state | next: rid + 1, waiting: Map.put(state.waiting, rid, {from, ref})})

      {:send, json} ->
        Port.command(state.port, json)
        loop(state)

      {:DOWN, _monitor, :process, pid, why} ->
        case Enum.find(state.calls, fn {_id, called} -> called == pid end) do
          {id, _} ->
            if why != :normal,
              do: reply(state.port, id, {:raised, "It stopped: #{Exception.format_exit(OneCore.Redact.redact(why))}"})

            loop(%{state | calls: Map.delete(state.calls, id)})

          nil ->
            loop(state)
        end

      {port, _closed} when port == state.port ->
        System.halt(0)
    end
  end

  defp received({:reply, rid, result}, state) do
    {{from, ref}, waiting} = Map.pop(state.waiting, rid)
    send(from, {ref, result})
    %{state | waiting: waiting}
  end

  defp received({:load, sets, target, cache}, state) do
    reply(state.port, :load, loaded(sets, target, cache))
    state
  end

  defp received({:call, id, what, about}, state) do
    pid = spawn(fn -> called(id, what, about) end)
    Process.monitor(pid)
    %{state | calls: Map.put(state.calls, id, pid)}
  end

  defp received({:cancel, id}, state) do
    with %{^id => pid} <- state.calls, do: Process.exit(pid, :kill)
    %{state | calls: Map.delete(state.calls, id)}
  end

  defp reply(port, id, {:returned, value}), do: Port.command(port, JSON.encode!(%{"answer" => id, "ok" => value}))
  defp reply(port, id, {:raised, why}), do: Port.command(port, JSON.encode!(%{"answer" => id, "raised" => why}))

  # A call runs in a process of its own, which answers it. `server-debug` traces this function to hear one begin.
  defp called(id, what, about) do
    Process.put(:"$one_call", id)
    # Whose call it is, as a server's process running one says (`OneCore.Apps.Calls`).
    Logger.metadata(installed: about["installed"], subject: about["subject"])

    result =
      try do
        call = call(id, about)
        {:returned, encoded(run(what, call))}
      rescue
        # What it raised is redacted (`OneCore.Redact`), as a call on the server is (`OneCore.Apps.held/2`).
        error -> {:raised, Exception.message(OneCore.Redact.redact(error))}
      catch
        kind, reason -> {:raised, Exception.format_banner(kind, OneCore.Redact.redact(reason))}
      end

    send(__MODULE__, {:send, JSON.encode!(Map.merge(%{"answer" => id}, reply_body(result)))})
  end

  defp reply_body({:returned, result}), do: %{"ok" => result}
  defp reply_body({:raised, why}), do: %{"raised" => why}

  defp run({:tool, tool, inputs}, call), do: app().call(tool, inputs, call)
  defp run({:hook, hook, about}, call), do: app().hook(hook, about, call)

  # What `server-debug` asks: its own code to run here, and what to ask it, as it would run on a server for an app
  # there. A call here runs for whoever it says, and the app's processes kept between calls are its own. An app
  # that says it's sensitive is never looked into.
  defp run({:debug, %{"source" => source, "request" => request, "scope" => scope}}, _call) do
    if function_exported?(app(), :sensitive, 0) and app().sensitive(),
      do: raise("It's sensitive: what it's given and keeps are secrets, so nothing looks into it as it runs")

    probe =
      with nil <- :persistent_term.get({__MODULE__, :probe, source}, nil) do
        probe =
          Enum.find_value(Code.compile_string(source), fn {module, _} ->
            function_exported?(module, :run, 2) && module
          end)

        :persistent_term.put({__MODULE__, :probe, source}, probe)
        probe
      end

    label = fn pid ->
      case Registry.keys(__MODULE__.Processes, pid) do
        [{module, _key}] -> inspect(module)
        [] -> nil
      end
    end

    module = fn name ->
      module = Module.concat([name])
      if :persistent_term.get({__MODULE__, :files, module}, nil), do: module, else: raise("It has no module #{name}")
    end

    probe.run(request, %{
      subject: scope["subject"],
      app_artifact: nil,
      own: if(scope["own"], do: &(label.(&1) != nil)),
      label: label,
      module: module,
      name: & &1,
      calls: {__MODULE__, :called, [:_, :_, %{"subject" => scope["subject"]}]}
    })
  end

  defp call(id, about) do
    %OneCore.Call{
      id: id,
      own: artifact(about["own"]),
      installed: about["installed"],
      folder: Path.join(System.tmp_dir!(), Enum.at(about["own"], 1)),
      server: about["server"],
      subject: about["subject"],
      actor: about["actor"],
      caller: about["caller"],
      app: about["app"],
      app_artifact: about["app_artifact"],
      loaded:
        for(
          name <- about["loaded"],
          module = :persistent_term.get({__MODULE__, :app, name}, nil),
          into: %{},
          do: {name, module}
        )
    }
  end

  @doc false
  def artifact([handle, name]), do: %OneCore.Artifact{handle: handle, prefix: name <> "/"}

  # What a tool answered, as JSON holds it: a file, text, or anything JSON holds.
  defp encoded({:blob, kind, data}), do: %{"blob" => [kind, Base.encode64(data)]}
  defp encoded({:file, kind, %OneCore.Artifact{handle: handle}, name}), do: %{"file" => [kind, handle, name]}
  defp encoded(text) when is_binary(text), do: %{"text" => text}
  defp encoded(result), do: %{"json" => JSON.decode!(JSON.encode!(result))}

  defp app, do: :persistent_term.get({__MODULE__, :main})

  # Compiles each set of files in turn, the apps it requires before the app, and says what the app is. What
  # it requires runs here as it, as it would run as it on a server, where its code can run here: what can't, the
  # app calls by its tools. A VM of the same version that ran before compiled them
  # already, and the server kept what it compiled in `cache`, where they're loaded from instead. Otherwise what was
  # compiled is answered with, its own side of One Core among it, for the server to keep.
  defp loaded(sets, target, cache) do
    {main, _files} = List.last(sets)

    modules =
      for {name, files} <- sets, reduce: %{} do
        modules -> Map.put(modules, name, compiled(name, files, target, cache))
      end

    apps = for {name, mods} <- modules, module <- mods, OneCore.App in behaviours(module), do: {name, module}

    case for {^main, module} <- apps, do: module do
      [app] ->
        runs = OneCore.Sandbox.Check.runs(Map.keys(modules) -- [main])

        for {name, module} <- apps,
            name == main or name in runs,
            do: :persistent_term.put({__MODULE__, :app, name}, module)

        :persistent_term.put({__MODULE__, :main}, app)
        described = described(app, main, modules[main], runs)
        {:returned, if(cache, do: described, else: Map.put(described, "beams", beams(modules)))}

      found ->
        raise "One module of an app says `@behaviour OneCore.App`, and #{length(found)} do"
    end
  rescue
    error -> {:raised, Exception.message(error)}
  end

  defp beams(modules) do
    runtime = Path.join(System.tmp_dir!(), "runtime")
    modules = Map.put(modules, "runtime", for(file <- File.ls!(runtime), do: Module.concat([Path.rootname(file)])))

    for {name, mods} <- modules, into: %{} do
      {name, for(module <- mods, into: %{}, do: {Atom.to_string(module), Base.encode64(beam(name, module))})}
    end
  end

  defp beam(name, module) do
    out = if name == "runtime", do: "runtime", else: Path.join(["apps", name, "ebin"])
    File.read!(Path.join([System.tmp_dir!(), out, "#{module}.beam"]))
  end

  defp compiled(name, files, target, cache) do
    folder = Path.join([System.tmp_dir!(), "apps", name])
    out = if cache, do: cache["ebin"], else: Path.join(folder, "ebin")
    File.mkdir_p!(folder)

    # A NIF loads as its build, which the app's code reaches under its name (`OneCore.Nif`).
    for {path, text} <- files, Path.dirname(path) == "server/nif", Path.extname(path) == ".ex" do
      library = files[Path.rootname(path) <> "/#{target}.so"] || raise "#{path} has no build for this kind of machine"

      with {:error, why} <- OneCore.Nif.load(text, library, name, Path.join([System.tmp_dir!(), "nif", name])),
           do: raise("#{path} didn't load on this machine, #{target}: #{why}")
    end

    sources =
      for {path, text} <- files,
          String.starts_with?(path, "server/"),
          Path.extname(path) == ".ex",
          Path.dirname(path) != "server/nif" do
        file = Path.join(folder, path)
        File.mkdir_p!(Path.dirname(file))
        File.write!(file, text)
        file
      end

    if cache do
      modules = Enum.map(cache["modules"][name], &String.to_atom/1)
      for module <- modules, do: :persistent_term.put({__MODULE__, :files, module}, {name, out, files})
      Enum.each(modules, &Code.ensure_loaded!/1)
      modules
    else
      File.mkdir_p!(out)
      Code.prepend_path(out)
      compiled(sources, name, out, files, folder)
    end
  end

  defp compiled(sources, name, out, files, folder) do
    case Kernel.ParallelCompiler.compile_to_path(sources, out, return_diagnostics: true) do
      {:ok, modules, warnings} ->
        Process.put(
          {__MODULE__, :warnings, name},
          messages(warnings.compile_warnings ++ warnings.runtime_warnings, folder)
        )

        for module <- modules, do: :persistent_term.put({__MODULE__, :files, module}, {name, out, files})
        Enum.each(modules, &Code.ensure_loaded!/1)
        modules

      {:error, errors, _warnings} ->
        raise messages(errors, folder) |> Enum.join("\n")
    end
  end

  defp messages(diagnostics, folder) do
    for %{file: file, position: position, message: message} <- diagnostics do
      line = if is_tuple(position), do: elem(position, 0), else: position
      "#{Path.relative_to(to_string(file), folder)}:#{line}: #{String.trim(message)}"
    end
  end

  @doc "The behaviours a module says it follows, one `behaviour` attribute each."
  def behaviours(module), do: List.flatten(Keyword.get_values(module.module_info(:attributes), :behaviour))

  # What the app says of itself, its manifest's JSON form (`OneCore.App.Manifest`), with what an untrusted app isn't
  # offered.
  defp described(app, main, modules, runs) do
    said = OneCore.App.Manifest.said(app)
    services = for module <- modules, OneCore.Service in behaviours(module), do: inspect(module)

    said
    |> Map.merge(%{
      warnings: Process.get({__MODULE__, :warnings, main}, []) |> Enum.join("\n"),
      problems: OneCore.Sandbox.Check.problems(app, modules, runs, said.hooks, said.offers, services)
    })
    |> JSON.encode!()
    |> JSON.decode!()
  end

  # An IO server that takes whatever it's sent and keeps none of it.
  defp sink do
    receive do
      {:io_request, from, ref, request} ->
        send(from, {:io_reply, ref, if(elem(request, 0) in [:put_chars, :requests], do: :ok, else: {:error, :enotsup})})
    end

    sink()
  end

  defmodule Logged do
    @moduledoc false
    # A warning or an error said to Logger goes to the server's log, as the app's: a report as Elixir translated it,
    # redacted (`OneCore.Redact`).
    def log(%{level: level} = event, _config) do
      text = IO.chardata_to_string(:logger_formatter.format(event, %{template: [:msg], single_line: false}))

      OneCore.Sandbox.Child.send_json(%{"log" => to_string(level), "text" => String.slice(text, 0, 8000)})
    end
  end
end

defmodule OneCore.Sandbox.Check do
  @moduledoc false
  # What an app's modules use that an untrusted app isn't offered, a sentence each with its file and line and what
  # to do instead. It reads the compiled code, after every macro, so it sees what will run. It's a check for the
  # author, not the wall: the wall is the sandbox this VM runs in.

  @programs [{System, :cmd}, {System, :shell}, {Port, :open}, {:os, :cmd}, {:erlang, :open_port}]
  @network [:gen_tcp, :gen_udp, :gen_sctp, :ssl, :httpc, :socket, Req, Finch, Mint.HTTP, HTTPoison, Tesla]

  # Core's hooks that mutate or validate what happens, which only an app on a server of its owner's may.
  @deciding for {hook, kind} <- OneCore.App.Manifest.kinds(), kind in [:mutate, :validate], do: hook

  # Which of the apps it requires, by their names, run in its VM as it: those whose code can run there. One that
  # can't, as most of the network's own can't, is called by its tools instead.
  def runs(required) do
    loaded = MapSet.new(Enum.flat_map(required, &modules_of/1))
    for name <- required, calls(modules_of(name), loaded) == [], do: name
  end

  # `runs` are the apps it requires that run in its VM, whose modules it may call.
  def problems(app, modules, runs, hooks, offers, services) do
    loaded = MapSet.new(modules ++ Enum.flat_map(runs, &modules_of/1))
    calls = calls(modules, loaded)

    hooking =
      for hook <- hooks,
          hook in @deciding,
          do:
            "#{where(app)}: hooks/0 names `#{hook}`, which changes or decides what happens. An untrusted " <>
              "app only observes: hook `changed` instead, or run it trusted, on a server of its owner's."

    offering =
      if offers == [],
        do: [],
        else: [
          "#{where(app)}: offers/0 offers hooks, which an untrusted app doesn't: run it trusted, on a server " <>
            "of its owner's."
        ]

    serving =
      for service <- services,
          do:
            "#{service} is a service, which runs on particular machines: an untrusted app has none. An app " <>
              "with one runs trusted, on a server of its owner's."

    calls ++ hooking ++ offering ++ serving
  end

  defp calls(modules, loaded) do
    for module <- modules,
        {file, line, m, f, a} <- remote(module),
        problem = problem(m, f, a, loaded),
        uniq: true,
        do: "#{file}:#{line}: #{problem}"
  end

  defp modules_of(name) do
    for {{OneCore.Sandbox.Child, :files, module}, {^name, _out, _files}} <- :persistent_term.get(), do: module
  end

  defp where(module) do
    case :persistent_term.get({OneCore.Sandbox.Child, :files, module}, nil) do
      {_name, out, _files} -> source(module, out)
      nil -> inspect(module)
    end
  end

  defp source(module, out) do
    case :beam_lib.chunks(Path.join(out, "#{module}.beam") |> String.to_charlist(), [:compile_info]) do
      {:ok, {_, [compile_info: info]}} -> relative(info[:source])
      _ -> inspect(module)
    end
  end

  defp relative(nil), do: "?"

  defp relative(source),
    do: source |> to_string() |> Path.split() |> Enum.drop_while(&(&1 != "server")) |> Path.join()

  # Every call to a function of another module that the module's code makes, and every capture of one, with its
  # file and line.
  defp remote(module) do
    {_name, out, _files} = :persistent_term.get({OneCore.Sandbox.Child, :files, module})
    beam = String.to_charlist(Path.join(out, "#{module}.beam"))

    with {:ok, {_, [debug_info: {:debug_info_v1, backend, data}]}} <- :beam_lib.chunks(beam, [:debug_info]),
         {:ok, forms} <- backend.debug_info(:erlang_v1, module, data, []) do
      file = source(module, out)
      walk(forms, file, [])
    else
      _ -> []
    end
  end

  defp walk({:call, anno, {:remote, _, {:atom, _, m}, {:atom, _, f}}, args}, file, found),
    do: walk(args, file, [{file, :erl_anno.line(anno), m, f, length(args)} | found])

  defp walk({:fun, anno, {:function, {:atom, _, m}, {:atom, _, f}, {:integer, _, a}}}, file, found),
    do: [{file, :erl_anno.line(anno), m, f, a} | found]

  defp walk(form, file, found) when is_tuple(form), do: walk(Tuple.to_list(form), file, found)
  defp walk(forms, file, found) when is_list(forms), do: Enum.reduce(forms, found, &walk(&1, file, &2))
  defp walk(_other, _file, found), do: found

  defp problem(m, f, a, loaded) do
    name = "#{inspect(m)}.#{f}/#{a}"

    cond do
      m in loaded ->
        nil

      offered = OneCore.App.Manifest.offered()[inspect(m)] ->
        if f not in offered, do: "#{name} isn't offered to an untrusted app. #{instead(m, f)}"

      m == OneCore.Service ->
        "#{name} reaches a service, which an untrusted app has none of: an app with one runs trusted, on a " <>
          "server of its owner's."

      String.starts_with?(inspect(m), "OneCore.") ->
        "#{name} is One Core's own, which an app doesn't call."

      {m, f} in @programs ->
        "#{name} starts a program, which an untrusted app doesn't: it has none to start."

      m in @network ->
        "#{name} reaches the network, which an untrusted app doesn't, nor any website: run it trusted, on a " <>
          "server of its owner's."

      m == Jason ->
        "#{name} is a package that isn't here. Elixir's own `JSON` encodes and decodes."

      not match?({:module, _}, Code.ensure_loaded(m)) ->
        app = m |> inspect() |> String.split(".") |> hd() |> Macro.underscore() |> String.replace("_", "-")

        "#{name} is another app's function, or a package that isn't here. An untrusted app calls " <>
          "another by its tool, as whoever the call runs for: `OneCore.Call.tool(call, \"#{app}\", \"#{f}\", " <>
          "%{...})`, naming #{app} in requires/0."

      true ->
        nil
    end
  end

  defp instead(OneCore.Call, :run_hook), do: "An untrusted app offers no hooks."

  defp instead(OneCore.Call, :fetch),
    do: "An untrusted app reaches no website: run it trusted, on a server of its owner's."

  defp instead(OneCore.Call, f) when f in [:forward, :run_elsewhere, :refused], do: "One Core does that itself."
  defp instead(OneCore.Artifact, :link), do: "Answer a blob as `{:file, kind, artifact, name}` instead."
  defp instead(_m, _f), do: ""
end
