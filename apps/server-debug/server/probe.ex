defmodule ServerDebug.Probe do
  @moduledoc false
  # What looks into an app as it runs, in the VM its code runs in: the server's, or an untrusted app's sandbox's,
  # which compiles this file's source itself and runs it there (`OneCore.Sandbox.debug/5`). So it uses nothing of One
  # Core's. What it's told of the app is `scope`:
  #
  # - `subject`: whose calls it looks at, the only ones it shows.
  # - `who`: the app, as the Logger metadata of a process running a call of it says (`OneCore.Apps.Calls`), or
  #   `nil` in a VM that runs no other app's calls.
  # - `own`: whether a process is one of the app's own, kept between calls, where those may be shown; `nil` if not.
  # - `label`: what one of those is, as the app wrote its module. `module`: a module of the app by that name.
  #   `name`: a text with the app's module names as it wrote them.
  # - `calls`: the function each call of the app runs in, `{module, function, head}`, with the head of a match
  #   specification that matches it only where the call is `subject`'s.

  # Its source, as the server compiled it, to send to an app's VM. That VM compiles it from no file, so has none.
  @source (case File.read(__ENV__.file) do
             {:ok, source} -> source
             _ -> nil
           end)
  def source, do: @source

  def run(%{"op" => "processes"} = request, scope), do: processes(request, scope)
  def run(%{"op" => "trace"} = request, scope), do: trace(request, scope)
  def run(%{"op" => "logs"} = request, scope), do: logs(request, scope)

  ## Whose a process is

  # A process is the caller's to see if it runs a call of the app for `subject`, was started by one (a `Task`'s
  # callers), or is one of the app's own that they may see.
  defp mine?(pid, scope) do
    case Process.info(pid, :dictionary) do
      {:dictionary, dictionary} -> mine?(pid, metadata(dictionary), dictionary[:"$callers"], scope)
      nil -> false
    end
  end

  defp mine?(pid, metadata, callers, scope) do
    cond do
      Map.has_key?(metadata, :subject) -> called?(metadata, scope)
      scope.own && scope.own.(pid) -> true
      match?([caller | _] when is_pid(caller), callers) -> caller?(hd(callers), scope)
      true -> false
    end
  end

  defp caller?(pid, scope) do
    case Process.info(pid, :dictionary) do
      {:dictionary, dictionary} -> called?(metadata(dictionary), scope)
      nil -> false
    end
  end

  defp called?(metadata, scope),
    do: metadata[:subject] == scope.subject and (scope.who == nil or metadata[:who] == scope.who)

  defp metadata(dictionary) do
    case List.keyfind(dictionary, :"$logger_metadata$", 0) do
      {_, metadata} when is_map(metadata) -> metadata
      _ -> %{}
    end
  end

  ## processes

  # The app's processes as observer_cli and `:recon.proc_count/2` show them, the top by `sort`, and the ETS
  # tables they own.
  defp processes(request, scope) do
    sort = String.to_existing_atom(request["sort"] || "memory")
    pids = for pid <- Process.list(), pid != self(), mine?(pid, scope), do: pid

    rows =
      for pid <- pids,
          info = Process.info(pid, [:memory, :reductions, :message_queue_len, :current_function, :registered_name]),
          info != nil do
        info = Map.new(info)

        %{
          "pid" => inspect(pid),
          "name" => name(pid, info, scope),
          "memory" => info.memory,
          "reductions" => info.reductions,
          "message_queue_len" => info.message_queue_len,
          "current_function" => scope.name.(function(info.current_function))
        }
      end

    tables =
      for table <- :ets.all(),
          owner = :ets.info(table, :owner),
          owner in pids,
          size = :ets.info(table, :size),
          is_integer(size) do
        %{
          "table" => scope.name.(inspect(:ets.info(table, :name))),
          "owner" => inspect(owner),
          "size" => size,
          "memory" => :ets.info(table, :memory) * :erlang.system_info(:wordsize)
        }
      end

    %{
      "count" => length(rows),
      "processes" => rows |> Enum.sort_by(& &1[Atom.to_string(sort)], :desc) |> Enum.take(request["top"] || 20),
      "tables" => Enum.sort_by(tables, & &1["memory"], :desc)
    }
  end

  defp name(pid, info, scope) do
    cond do
      is_atom(info.registered_name) and info.registered_name != nil ->
        scope.name.(inspect(info.registered_name))

      label = scope.label.(pid) ->
        label

      true ->
        "call"
    end
  end

  defp function({m, f, a}), do: Exception.format_mfa(m, f, a)
  defp function(other), do: inspect(other)

  ## trace

  # `:recon_trace.calls/3` on one function of the app, or all of a module's, with what each was called with and
  # what it returned or raised. Only the caller's calls are shown. Whose a call is is heard as it begins: the function
  # every call of the app runs in, `scope.calls`, is traced too, matched only where it's the caller's, so its trace
  # messages say which processes are in one, in order with the rest. A process that was in one already as the trace
  # began, a task a call started and the app's own are told by looking. Recon's limit counts everyone's calls, so a
  # function others call often stops at it sooner. Only one trace runs in a VM at once, as recon has it.
  defp trace(request, scope) do
    module = scope.module.(request["module"])
    function = if request["function"], do: function!(module, request["function"], request["arity"]), else: :_
    calls = request["calls"] || 10
    seconds = request["seconds"] || 20
    # Recon's own limit, of trace messages: a call and its return are two.
    most = min(calls * 20, 1000)

    # Recon's trace is one per VM: starting one clears the last. A lock held while it runs, which goes with the
    # process holding it, keeps a second from starting.
    lock = {:recon_trace, self()}

    if not :global.set_lock(lock, [node()], 0),
      do: raise("Another trace is running on this server; each lasts a minute at most")

    head = if is_integer(request["arity"]), do: List.duplicate(:_, request["arity"]), else: :_
    {in_module, in_function, in_head} = scope.calls
    me = self()

    formatter = fn traced ->
      send(me, {__MODULE__, traced})
      ""
    end

    specs = [
      {module, function, [{head, [], [{:exception_trace}]}]},
      {in_module, in_function, [{in_head, [], [{:exception_trace}]}]}
    ]

    try do
      case :recon_trace.calls(specs, most, scope: :local, formatter: formatter) do
        0 ->
          raise "That matches no function of #{scope.name.(inspect(module))}"

        _ ->
          # The formatter passes on each message in turn, and ends after the last once recon's limit is reached.
          formatting = Process.monitor(Process.whereis(:recon_trace_formatter))
          until = System.monotonic_time(:millisecond) + seconds * 1000
          state = %{lines: [], calls: 0, open: %{}, in: %{}, ended: nil, until: until, formatting: formatting}
          state = traced(state, calls, most, {in_module, in_function}, scope)
          %{"lines" => Enum.reverse(state.lines), "calls" => state.calls, "ended" => state.ended}
      end
    after
      # Recon's tracer is linked to the process that started it, and clearing kills it.
      with tracer when is_pid(tracer) <- Process.whereis(:recon_trace_tracer), do: Process.unlink(tracer)
      :recon_trace.clear()
      :global.del_lock(lock, [node()])
    end
  end

  defp function!(module, name, arity) do
    functions = module.module_info(:functions)

    Enum.find_value(functions, fn {function, takes} ->
      (Atom.to_string(function) == name and (arity == nil or arity == takes)) && function
    end) || raise "#{module |> inspect() |> String.split(".") |> List.last()} has no function #{name}"
  end

  defp traced(%{calls: calls, open: open} = state, calls, _most, _in, _scope) when map_size(open) == 0,
    do: %{state | ended: "It showed the #{calls} calls asked for."}

  defp traced(state, calls, most, {in_module, in_function} = within, scope) do
    left = max(state.until - System.monotonic_time(:millisecond), 0)

    receive do
      # One of the caller's calls begins in this process, or ends.
      {__MODULE__, {:trace, pid, :call, {^in_module, ^in_function, _}}} ->
        traced(%{state | in: Map.update(state.in, pid, 1, &(&1 + 1))}, calls, most, within, scope)

      {__MODULE__, {:trace, pid, ended, {^in_module, ^in_function, _}, _}}
      when ended in [:return_from, :exception_from] ->
        traced(%{state | in: out(state.in, pid)}, calls, most, within, scope)

      {__MODULE__, {:trace, pid, :call, {m, f, arguments}}} ->
        if state.calls < calls and (Map.has_key?(state.in, pid) or mine?(pid, scope)) do
          line = "#{inspect(pid)} #{called(m, f, arguments)}"
          state = %{state | calls: state.calls + 1, open: Map.update(state.open, pid, 1, &(&1 + 1))}
          traced(line(state, line, scope), calls, most, within, scope)
        else
          traced(state, calls, most, within, scope)
        end

      {__MODULE__, {:trace, pid, :return_from, {m, f, a}, value}} ->
        line = "#{function({m, f, a})} returned #{shown(value)}"
        traced(returned(state, pid, line, scope), calls, most, within, scope)

      {__MODULE__, {:trace, pid, :exception_from, {m, f, a}, {kind, reason}}} ->
        line = "#{function({m, f, a})} raised #{shown(Exception.normalize(kind, reason))}"
        traced(returned(state, pid, line, scope), calls, most, within, scope)

      {__MODULE__, _other} ->
        traced(state, calls, most, within, scope)

      {:DOWN, formatting, :process, _, _} when formatting == state.formatting ->
        %{state | ended: "It stopped at recon's limit of #{most} trace messages, which counts everyone's calls."}
    after
      left -> %{state | ended: "Its time was up."}
    end
  end

  defp out(in_calls, pid) do
    case in_calls do
      %{^pid => 1} -> Map.delete(in_calls, pid)
      %{^pid => n} -> %{in_calls | pid => n - 1}
      _ -> in_calls
    end
  end

  defp returned(state, pid, line, scope) do
    case state.open do
      %{^pid => 1} -> line(%{state | open: Map.delete(state.open, pid)}, "#{inspect(pid)} #{line}", scope)
      %{^pid => n} -> line(%{state | open: %{state.open | pid => n - 1}}, "#{inspect(pid)} #{line}", scope)
      _ -> state
    end
  end

  defp called(m, f, arguments),
    do: "#{inspect(m)}.#{f}(#{Enum.map_join(arguments, ", ", &shown/1)})"

  defp shown(term), do: inspect(term, limit: 20, printable_limit: 400)

  defp line(state, line, scope) do
    at = Time.utc_now() |> Time.truncate(:millisecond) |> Time.to_string()
    %{state | lines: ["#{at} #{String.slice(scope.name.(line), 0, 2000)}" | state.lines]}
  end

  ## logs

  # A follow: what the app says to Logger, every level, as it runs for `subject`, kept for whoever follows it until
  # they read it, for `minutes` or until it has heard `lines`. Reading it again goes on from there.
  defp logs(request, scope) do
    name = {:global, {__MODULE__.Follow, request["follower"], scope.who, scope.subject}}

    if request["stop"] do
      with pid when is_pid(pid) <- GenServer.whereis(name), do: GenServer.stop(pid)
      %{"lines" => [], "ended" => "It stopped following.", "until" => nil}
    else
      follow =
        case GenServer.start(__MODULE__.Follow, {scope, request["minutes"] || 10, request["lines"] || 1000}, name: name) do
          {:ok, pid} -> pid
          {:error, {:already_started, pid}} -> pid
          {:error, why} -> raise why
        end

      GenServer.call(follow, {:read, (request["wait"] || 10) * 1000}, :infinity)
    end
  end

  @doc false
  # A `:logger` handler of a follow's, run in whatever process logged: what's the caller's is formatted there and
  # sent on, until the follow has heard all it may.
  def log(%{level: level, msg: msg, meta: meta}, %{config: %{to: to, left: left, scope: scope}}) do
    if heard?(meta, scope) and :atomics.sub_get(left, 1, 1) >= 0 do
      at = meta |> Map.get(:time, :os.system_time(:microsecond)) |> div(1000)
      at = at |> DateTime.from_unix!(:millisecond) |> DateTime.to_time() |> Time.to_string()
      send(to, {:received, "#{at} [#{level}] #{String.slice(scope.name.(message(msg)), 0, 4000)}"})
    end
  end

  defp heard?(meta, scope) do
    callers = Process.get(:"$callers")
    mine?(meta[:pid] || self(), Map.take(meta, [:subject, :who]), callers, scope)
  end

  defp message({:string, text}), do: IO.chardata_to_string(text)
  defp message({:report, report}), do: shown(report)

  defp message({format, arguments}) do
    IO.chardata_to_string(:io_lib.format(format, arguments))
  rescue
    _ -> shown({format, arguments})
  end

  defmodule Follow do
    @moduledoc false
    # A follow, as `logs` keeps it: a `:logger` handler of its own, in one of a few slots, and what it heard.
    use GenServer

    @slots 32

    def init({scope, minutes, lines}) do
      left = :atomics.new(1, [])
      :atomics.put(left, 1, lines)
      config = %{level: :all, config: %{to: self(), left: left, scope: scope}}
      probe = Module.split(__MODULE__) |> Enum.drop(-1) |> Module.concat()

      slot =
        Enum.find(0..(@slots - 1), fn slot ->
          :logger.add_handler(:"server_debug_#{slot}", probe, config) == :ok
        end)

      if slot do
        Process.send_after(self(), :ended, :timer.minutes(minutes))
        until = DateTime.utc_now() |> DateTime.add(minutes, :minute) |> DateTime.truncate(:second)

        {:ok,
         %{handler: :"server_debug_#{slot}", lines: [], received: 0, most: lines, until: until, ended: nil, waiting: nil}}
      else
        {:stop, "#{@slots} follows are running on this server already; one ends within an hour"}
      end
    end

    def handle_call({:read, wait}, from, state) do
      if state.lines != [] or state.ended do
        read(from, state)
      else
        timer = Process.send_after(self(), {:waited, from}, wait)
        {:noreply, %{state | waiting: {from, timer}}}
      end
    end

    def handle_info({:received, line}, state) do
      state = %{state | lines: [line | state.lines], received: state.received + 1}
      state = if state.received >= state.most, do: ended(state, "It heard #{state.most} lines, its most."), else: state

      # What's waiting is answered a moment after the first line, with whatever else came.
      with {from, timer} <- state.waiting, true <- state.lines == [line] or state.ended != nil do
        Process.cancel_timer(timer)
        Process.send_after(self(), {:waited, from}, if(state.ended, do: 0, else: 500))
      end

      {:noreply, state}
    end

    def handle_info({:waited, from}, %{waiting: {from, _timer}} = state), do: read(from, state)
    def handle_info({:waited, _from}, state), do: {:noreply, state}

    def handle_info(:ended, state) do
      state = ended(state, "Its time was up.")
      Process.send_after(self(), :stop, :timer.minutes(2))
      with {from, _timer} <- state.waiting, do: send(self(), {:waited, from})
      {:noreply, state}
    end

    def handle_info(:stop, state), do: {:stop, :normal, state}

    def terminate(_why, state), do: :logger.remove_handler(state.handler)

    defp ended(%{ended: nil} = state, why) do
      :logger.remove_handler(state.handler)
      %{state | ended: why}
    end

    defp ended(state, _why), do: state

    # What it heard since it was last read. One that's ended stops once it's read.
    defp read(from, state) do
      GenServer.reply(from, %{
        "lines" => Enum.reverse(state.lines),
        "until" => DateTime.to_iso8601(state.until),
        "ended" => state.ended
      })

      state = %{state | lines: [], waiting: nil}
      if state.ended, do: {:stop, :normal, state}, else: {:noreply, state}
    end
  end
end
