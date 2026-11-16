# One Core inside a plugin's own VM, in sandbox mode (`OneCore.Sandbox`, which says the protocol). The plugin is
# compiled against these modules, which have the names and functions of the server's own, and each of their
# functions is a message to the server. This file is compiled by the VM it runs in, behind the wall.

defmodule OneCore.Call do
  @moduledoc false
  # What a tool is called with besides its inputs, as the server's own (`OneCore.Call`), whose functions here ask
  # the server. `id` is the call's, which the server knows it by: a call that has ended asks nothing more.
  alias OneCore.Sandbox.Child

  defstruct [:id, :own, :installed, :folder, :server, :subject, :actor, :caller, :plugin, :who, :uses]

  def plugin(%__MODULE__{uses: uses, plugin: plugin}, name) do
    case uses do
      %{^name => module} -> module
      _ -> raise "#{plugin} doesn't say it uses #{name}. Its uses/0 names the plugins it uses."
    end
  end

  def tool(%__MODULE__{id: id}, plugin, tool, inputs, options \\ []) do
    asked = %{"plugin" => plugin, "tool" => tool, "inputs" => inputs}

    asked =
      if Keyword.has_key?(options, :on_behalf_of),
        do: Map.put(asked, "on_behalf_of", options[:on_behalf_of]),
        else: asked

    case Child.request("tool", asked, id) do
      {:blob, kind, data} -> {:blob, kind, data}
      answer -> answer
    end
  end

  def make(%__MODULE__{id: id}, made), do: Child.artifact(Child.request("make", %{"id" => made}, id))

  def reach(%__MODULE__{id: id}, name, relation \\ "viewer"),
    do: Child.artifact(Child.request("reach", %{"name" => name, "relation" => relation}, id))

  def reaches?(%__MODULE__{id: id}, name, relation \\ "viewer"),
    do: Child.request("reaches", %{"name" => name, "relation" => relation}, id)

  def member?(%__MODULE__{id: id}, of), do: Child.request("member", %{"of" => of}, id)
  def owns?(%__MODULE__{id: id}), do: Child.request("owns", %{}, id)
  def plugins(%__MODULE__{id: id}), do: Child.request("plugins", %{}, id)
  def deployed(%__MODULE__{id: id}), do: Child.request("deployed", %{}, id)

  @doc "Has the server holding what this call needs run it instead: it ends here if that server runs it."
  def forward_to(%__MODULE__{id: id}, server), do: Child.request("forward_to", %{"server" => server}, id)

  @doc """
  Makes a web request, from the server, to a host the plugin's `permissions/0` names: `url`, and `method`,
  `headers` and `body` if it has them. Gives the answer's `status`, `headers` and `body`.
  """
  def fetch(%__MODULE__{id: id}, request) do
    request = Map.new(request, fn {key, value} -> {to_string(key), value} end)
    body = request["body"]
    request = if is_binary(body), do: Map.put(request, "body", Base.encode64(body)), else: request
    Child.request("fetch", %{"web" => request}, id)
  end

  @doc "A process of this VM's that keeps running between calls: one for each `module` and `key`."
  def process(%__MODULE__{}, module, key, argument) do
    name = {:via, Registry, {Child.Processes, {module, key}}}

    case GenServer.start(module, argument, name: name) do
      {:ok, pid} -> pid
      {:error, {:already_started, pid}} -> pid
      {:error, why} -> raise "#{inspect(module)} didn't start: #{inspect(why)}"
    end
  end
end

defmodule OneCore.Artifact do
  @moduledoc false
  # An artifact as a plugin in sandbox mode holds it: a handle the server gave, which it looks up, and its name.
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

defmodule OneCore.Plugin do
  @moduledoc false
  # The interface a plugin is written against (the server's `OneCore.Plugin` says it all).

  def file(module, path) do
    {name, _out, files} =
      :persistent_term.get({OneCore.Sandbox.Child, :files, module}, nil) || raise "#{inspect(module)} is no plugin's"

    if String.ends_with?(path, ".ex") or path == "SKILL.md" or String.starts_with?(path, ["client/", "skills/"]),
      do: raise("#{name}'s plugin has no file #{path} outside client/"),
      else: files[path] || raise("#{name}'s plugin has no file #{path} outside client/")
  end

  def nif(module, load) do
    library =
      :persistent_term.get({OneCore.Sandbox.Child, :nif, module}, nil) ||
        raise "#{inspect(module)} has no NIF built for this machine"

    load.(String.to_charlist(library))
  end

  @callback tools() :: [map]
  @callback call(String.t(), map, struct) :: term
  @callback apps() :: [map]
  @callback widgets() :: [map]
  @callback uses() :: [String.t()]
  @callback hooks() :: [String.t()]
  @callback hook(String.t(), map, struct) :: term
  @callback offers() :: [map]
  @callback jobs() :: [map]
  @callback schedule(struct, [map], (map -> term)) :: term
  @callback essential() :: boolean
  @callback permissions() :: [String.t()]
  @optional_callbacks tools: 0,
                      call: 3,
                      apps: 0,
                      widgets: 0,
                      uses: 0,
                      hooks: 0,
                      hook: 3,
                      offers: 0,
                      jobs: 0,
                      schedule: 3,
                      essential: 0,
                      permissions: 0
end

defmodule OneCore.Service do
  @moduledoc false
  @callback called() :: String.t()
  @callback needs() :: String.t()
end

defmodule OneCore.Sandbox.Child do
  @moduledoc false

  # Speaks to the server over standard input and output: what the server sends is Erlang's external term format,
  # and what this sends is JSON, each in frames of a 4-byte length.
  def main do
    Process.register(self(), __MODULE__)
    sink = spawn(&sink/0)
    # Whatever the plugin writes as output or errors would land in the protocol: it's dropped.
    Process.group_leader(self(), sink)
    with pid when is_pid(pid) <- Process.whereis(:standard_error), do: Process.unregister(:standard_error)
    Process.register(sink, :standard_error)
    :logger.remove_handler(:default)
    :logger.add_handler(:server, __MODULE__.Logged, %{level: :warning})
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
      {^ref, {:ok, answer}} -> answer
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
            if why != :normal, do: answer(state.port, id, {:raised, "It stopped: #{Exception.format_exit(why)}"})
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
    answer(state.port, :load, loaded(sets, target, cache))
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

  defp answer(port, id, {:answered, answer}), do: Port.command(port, JSON.encode!(%{"answer" => id, "ok" => answer}))
  defp answer(port, id, {:raised, why}), do: Port.command(port, JSON.encode!(%{"answer" => id, "raised" => why}))

  # A call runs in a process of its own, which answers it.
  defp called(id, what, about) do
    Process.put(:"$one_call", id)

    result =
      try do
        call = call(id, about)
        {:answered, encoded(run(what, call))}
      rescue
        error -> {:raised, Exception.message(error)}
      catch
        kind, reason -> {:raised, Exception.format_banner(kind, reason)}
      end

    send(__MODULE__, {:send, JSON.encode!(Map.merge(%{"answer" => id}, answered(result)))})
  end

  defp answered({:answered, answer}), do: %{"ok" => answer}
  defp answered({:raised, why}), do: %{"raised" => why}

  defp run({:tool, tool, inputs}, call), do: plugin().call(tool, inputs, call)
  defp run({:hook, hook, about}, call), do: plugin().hook(hook, about, call)

  # A job is called by the server, as the clock: whichever plugin it uses keeps its schedule.
  defp run({:schedule, jobs}, call) do
    fire = fn job ->
      request(
        "fire",
        %{"installed" => call.installed, "plugin" => call.plugin, "tool" => job.tool, "inputs" => job.inputs},
        nil
      )
    end

    for {_name, module} <- call.uses, function_exported?(module, :schedule, 3), do: module.schedule(call, jobs, fire)
    nil
  end

  defp call(id, about) do
    %OneCore.Call{
      id: id,
      own: artifact(about["own"]),
      installed: about["installed"],
      folder: Path.join([System.tmp_dir!(), "own", about["installed"], about["plugin"]]),
      server: about["server"],
      subject: about["subject"],
      actor: about["actor"],
      caller: about["caller"],
      plugin: about["plugin"],
      who: about["who"],
      uses: Map.new(about["uses"], &{&1, :persistent_term.get({__MODULE__, :plugin, &1})})
    }
  end

  @doc false
  def artifact([handle, name]), do: %OneCore.Artifact{handle: handle, prefix: name <> "/"}

  # What a tool answered, as JSON holds it: a file, text, or anything JSON holds.
  defp encoded({:blob, kind, data}), do: %{"blob" => [kind, Base.encode64(data)]}
  defp encoded({:file, kind, %OneCore.Artifact{handle: handle}, name}), do: %{"file" => [kind, handle, name]}
  defp encoded(text) when is_binary(text), do: %{"text" => text}
  defp encoded(answer), do: %{"json" => JSON.decode!(JSON.encode!(answer))}

  defp plugin, do: :persistent_term.get({__MODULE__, :main})

  # Compiles each set of files in turn, the plugins it uses before the plugin, and says what the plugin is. What it
  # uses runs here as it, as it would run as it on a server. A VM of the same version that ran before compiled them
  # already, and the server kept what it compiled in `cache`, where they're loaded from instead. Otherwise what was
  # compiled is answered with, its own side of One Core among it, for the server to keep.
  defp loaded(sets, target, cache) do
    {main, _files} = List.last(sets)

    modules =
      for {name, files} <- sets, reduce: %{} do
        modules -> Map.put(modules, name, compiled(name, files, target, cache))
      end

    plugins = for {name, mods} <- modules, module <- mods, OneCore.Plugin in behaviours(module), do: {name, module}

    case for {^main, module} <- plugins, do: module do
      [plugin] ->
        for {name, module} <- plugins, do: :persistent_term.put({__MODULE__, :plugin, name}, module)
        :persistent_term.put({__MODULE__, :main}, plugin)
        described = described(plugin, main, modules[main], Map.keys(modules) -- [main])
        {:answered, if(cache, do: described, else: Map.put(described, "beams", beams(modules)))}

      found ->
        raise "One module of a plugin says `@behaviour OneCore.Plugin`, and #{length(found)} do"
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
    out = if name == "runtime", do: "runtime", else: Path.join(["plugins", name, "ebin"])
    File.read!(Path.join([System.tmp_dir!(), out, "#{module}.beam"]))
  end

  defp compiled(name, files, target, cache) do
    folder = Path.join([System.tmp_dir!(), "plugins", name])
    out = if cache, do: cache["ebin"], else: Path.join(folder, "ebin")
    File.mkdir_p!(folder)

    sources =
      for {path, text} <- files, String.starts_with?(path, "server/"), Path.extname(path) == ".ex" do
        file = Path.join(folder, path)
        File.mkdir_p!(Path.dirname(file))
        File.write!(file, text)
        file
      end

    # A NIF's module loads its library for this kind of machine as it's loaded.
    for {path, text} <- files,
        Path.dirname(path) == "server/nif",
        Path.extname(path) == ".ex",
        library = files[Path.rootname(path) <> "/#{target}.so"],
        {:ok, {:defmodule, _, [{:__aliases__, _, parts} | _]}} <- [Code.string_to_quoted(text)] do
      # Erlang adds the library's extension itself.
      file = Path.join([System.tmp_dir!(), "nif", name, Path.basename(path, ".ex")])
      File.mkdir_p!(Path.dirname(file))
      File.write!(file <> ".so", library)
      :persistent_term.put({__MODULE__, :nif, Module.concat(parts)}, file)
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
        Process.put({__MODULE__, :warnings, name}, said(warnings.compile_warnings ++ warnings.runtime_warnings, folder))

        for module <- modules, do: :persistent_term.put({__MODULE__, :files, module}, {name, out, files})
        Enum.each(modules, &Code.ensure_loaded!/1)
        modules

      {:error, errors, _warnings} ->
        raise said(errors, folder) |> Enum.join("\n")
    end
  end

  defp said(diagnostics, folder) do
    for %{file: file, position: position, message: message} <- diagnostics do
      line = if is_tuple(position), do: elem(position, 0), else: position
      "#{Path.relative_to(to_string(file), folder)}:#{line}: #{String.trim(message)}"
    end
  end

  defp behaviours(module), do: List.flatten(Keyword.get_values(module.module_info(:attributes), :behaviour))

  # What the plugin says of itself, as `OneCore.Plugins.Version` takes it, with what sandbox mode doesn't offer it.
  defp described(plugin, main, modules, uses) do
    said = fn function, none ->
      if function_exported?(plugin, function, 0), do: apply(plugin, function, []), else: none
    end

    hooks = said.(:hooks, [])
    offers = said.(:offers, [])
    services = for module <- modules, OneCore.Service in behaviours(module), do: inspect(module)

    %{
      "tools" => said.(:tools, []),
      "apps" => said.(:apps, []),
      "title" => said.(:title, nil),
      "icon" => said.(:icon, nil),
      "offers" => offers,
      "widgets" => said.(:widgets, []),
      "uses" => said.(:uses, []),
      "hooks" => hooks,
      "jobs" => said.(:jobs, []),
      "permissions" => said.(:permissions, []),
      "warnings" => Process.get({__MODULE__, :warnings, main}, []) |> Enum.join("\n"),
      "problems" => OneCore.Sandbox.Check.problems(plugin, modules, uses, hooks, offers, services)
    }
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
    # A warning or an error said to Logger goes to the server's log, as the plugin's.
    def log(%{level: level} = event, _config) do
      text =
        case event.msg do
          {:string, text} -> IO.chardata_to_string(text)
          {:report, report} -> inspect(report)
          {format, args} -> IO.chardata_to_string(:io_lib.format(format, args))
        end

      OneCore.Sandbox.Child.send_json(%{"log" => to_string(level), "text" => String.slice(text, 0, 8000)})
    end
  end
end

defmodule OneCore.Sandbox.Check do
  @moduledoc false
  # What a plugin's modules use that sandbox mode doesn't offer, a sentence each with its file and line and what to
  # do instead. It reads the compiled code, after every macro, so it sees what will run. It's a check for the
  # author, not the wall: the wall is the sandbox this VM runs in.

  @offered %{
    OneCore.Call =>
      ~w(plugin tool make reach reaches? member? owns? plugins deployed process forward_to fetch __struct__)a,
    OneCore.Artifact => ~w(name read read_all write replace remove size list __struct__)a,
    OneCore.Plugin => ~w(file nif behaviour_info)a
  }

  @programs [{System, :cmd}, {System, :shell}, {Port, :open}, {:os, :cmd}, {:erlang, :open_port}]
  @network [:gen_tcp, :gen_udp, :gen_sctp, :ssl, :httpc, :socket, Req, Finch, Mint.HTTP, HTTPoison, Tesla]

  # Core's hooks that change or decide what happens, which only a plugin on a server of its owner's may.
  @deciding ~w(inputs call reach device owner)

  # The plugins it uses run in its VM as it, so they're checked too: one that can't run there, as most of the
  # network's own can't, is used by its tools instead.
  def problems(plugin, modules, uses, hooks, offers, services) do
    loaded = MapSet.new(modules ++ Enum.flat_map(uses, &modules_of/1))
    calls = calls(modules, loaded)

    using =
      for name <- uses, said = calls(modules_of(name), loaded), said != [] do
        "#{where(plugin)}: it uses #{name}, whose code would run in its VM in sandbox mode, and can't there " <>
          "(#{hd(said)}#{if length(said) > 1, do: " And #{length(said) - 1} more."}) Call its tools instead, which " <>
          "permissions/0 names, and leave what can't run out of uses/0."
      end

    hooking =
      for hook <- hooks,
          hook in @deciding,
          do:
            "#{where(plugin)}: hooks/0 names `#{hook}`, which changes or decides what happens. In sandbox mode a " <>
              "plugin only hears: hook `called` or `changed` instead, or run it on a server of its owner's."

    offering =
      if offers == [],
        do: [],
        else: [
          "#{where(plugin)}: offers/0 offers hooks, which sandbox mode doesn't: run it on a server of its owner's."
        ]

    serving =
      for service <- services,
          do:
            "#{service} is a service, which runs on particular machines: sandbox mode has none. A plugin with one " <>
              "runs on a server of its owner's."

    calls ++ using ++ hooking ++ offering ++ serving
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
    do: source |> to_string() |> String.split("/") |> Enum.drop_while(&(&1 != "server")) |> Enum.join("/")

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

      Map.has_key?(@offered, m) ->
        if f not in @offered[m], do: "#{name} isn't offered in sandbox mode. #{instead(m, f)}"

      m == OneCore.Service ->
        "#{name} reaches a service, which sandbox mode has none of: a plugin with one runs on a server of its owner's."

      String.starts_with?(inspect(m), "OneCore.") ->
        "#{name} is One Core's own, which a plugin doesn't call."

      {m, f} in @programs ->
        "#{name} starts a program, which sandbox mode doesn't: it has none to start."

      m in @network ->
        "#{name} reaches the network, which sandbox mode doesn't. Fetch from the web with " <>
          "`OneCore.Call.fetch(call, %{url: ...})`, from a host the plugin's permissions/0 names."

      m == Jason ->
        "#{name} is a package that isn't here. Elixir's own `JSON` encodes and decodes."

      not match?({:module, _}, Code.ensure_loaded(m)) ->
        plugin = m |> inspect() |> String.split(".") |> hd() |> Macro.underscore() |> String.replace("_", "-")

        "#{name} is another plugin's function, or a package that isn't here. In sandbox mode another plugin is " <>
          "called by its tool, as whoever the call runs for: `OneCore.Call.tool(call, \"#{plugin}\", \"#{f}\", " <>
          "%{...})`, which permissions/0 names."

      true ->
        nil
    end
  end

  defp instead(OneCore.Call, :run_hook), do: "A plugin in sandbox mode offers no hooks."
  defp instead(OneCore.Call, f) when f in [:forward, :run_elsewhere, :refused], do: "One Core does that itself."
  defp instead(OneCore.Artifact, :link), do: "Answer a blob as `{:file, kind, artifact, name}` instead."
  defp instead(_m, _f), do: ""
end
