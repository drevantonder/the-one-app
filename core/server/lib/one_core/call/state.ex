defmodule OneCore.Call.State do
  @moduledoc """
  The state every tool call is in, whatever started it: an MCP client, `one <app> <tool>`, a code mode script, a
  app calling a tool (`OneCore.Call.tool/5`) or a scheduled job.

      asked ─► checking (its inputs, who may, the hooks that mutate and validate it)
                 ├─► refused ─► an MCP protocol error to a caller that asked for a task, and no task is made
                 └─► accepted ─► working: routing │ waiting │ running   (an MCP task is answered here)

      working ─► completed │ failed │ cancelled   (final: nothing moves out of them)
      running ⇄ input_required                    (`OneCore.Call.ask/2`, then its answer)

  Working is one of three: `routing`, passed on to the server that runs it; `waiting`, for where it runs to be ready
  (a server starting, a sandbox's VM loading), its message saying for what; and `running`, its message the app's
  last `OneCore.Call.stage/2`. An MCP task's `status` is `working` for all three, and its `statusMessage` the message.

  A tool that raised, or answered what its outputs don't say, is `completed`, its answer an error (`isError`), as MCP
  has it from 2026-07-28. `failed` is a JSON-RPC error: the server running it stopped, or One Core's own fault.
  Cancelling is allowed from checking and every state of working, by `notifications/cancelled` for a call that isn't a
  task and `tasks/cancel` for one that is (`OneCore.Tasks`), and reaches the server it's passed on to.

  The moves are a table, `@moves`, rather than a `:gen_statem` for each call: a call runs in the process that asked
  for it, as it did before it had states, so the many quick plain calls cost no process or message more, and the few
  that others act on (a task polled, a call cancelled, its progress told) are one row of an ETS table that every
  process reads. Each move changes that row only if nothing else moved it meanwhile, so a cancel arriving as a call
  completes is one or the other, and a move the table doesn't have is refused.
  """
  alias OneCore.{Iroh, Server, Servers}

  @table __MODULE__
  @stack :"$one_call_states"

  @working [:routing, :waiting, :running]
  @final [:refused, :completed, :failed, :cancelled]

  @moves %{
    asked: [:checking],
    checking: [:refused, :cancelled | @working],
    routing: [:waiting, :running, :completed, :failed, :cancelled],
    waiting: [:routing, :running, :completed, :failed, :cancelled],
    running: [:routing, :waiting, :input_required, :completed, :failed, :cancelled],
    input_required: [:running, :completed, :failed, :cancelled],
    refused: [],
    completed: [],
    failed: [],
    cancelled: []
  }

  # How long a cancelled call has to reach its next `stage` or `ask`, which raise in it so its `try … after` runs,
  # before its process is stopped outright.
  @grace 5_000

  defmodule Cancelled do
    @moduledoc "What a cancelled call raises in itself, at its next `OneCore.Call.stage/2` or `OneCore.Call.ask/2`."
    defexception message: "It was cancelled."
  end

  @doc "How long a cancelled call has to end before its process is stopped outright, in milliseconds."
  def grace, do: @grace

  @doc "Makes the table the states are in, once, from the application starting, whose process keeps it."
  def table, do: :ets.new(@table, [:named_table, :public, :set, read_concurrency: true])

  @doc "The moves a call may make from each state."
  def moves, do: @moves

  @doc """
  Begins a call's state in this process, `asked`, and gives its id. What started it says, in `about`, what others act
  on it by: `request`, the MCP request a client may cancel it by, as who sent it and its id; `ask`, the message this
  server is answering, whose progress notifications say what it's doing; and `task`, that the caller asked for an MCP
  task, with the process waiting to hear it was made (`OneCore.Tasks`).
  """
  def begin(about \\ %{}) do
    id = System.unique_integer([:positive])
    now = System.system_time(:millisecond)

    row =
      Map.merge(
        %{
          state: :asked,
          message: nil,
          stage: nil,
          pid: self(),
          task: nil,
          inputs: %{},
          asked: 0,
          forwarded: nil,
          stopped: false
        },
        Map.merge(about, %{created: now, updated: now})
      )

    :ets.insert(@table, {id, 0, row})
    Process.put(@stack, [id | Process.get(@stack, [])])
    id
  end

  @doc "Ends this process's call `id`: its row goes, and its final state is given."
  def finish(id) do
    Process.put(@stack, List.delete(Process.get(@stack, []), id))
    row = get(id)
    :ets.delete(@table, id)
    row
  end

  @doc """
  Runs a tool call's `fun` in its state: the one this process began for it, still `asked`, or one of its own, as for a
  call another app made. Moves it to `checking`, and once `fun` gives what `OneCore.Apps.Calls.call/6` does, to
  where that ends it. Gives what `fun` gave.
  """
  def run(fun) do
    {id, own} =
      case current() do
        {id, %{state: :asked}} -> {id, false}
        _ -> {begin(), true}
      end

    move(id, :checking)

    try do
      outcome = fun.()
      ended(id, outcome)
      outcome
    rescue
      error ->
        move(id, :failed, Exception.message(error))
        reraise error, __STACKTRACE__
    after
      if own, do: finish(id)
    end
  end

  # Where what a call gave leaves it. One that ended while still checking was refused, and one that was working
  # completed, its answer an error or not. A call passed on that would only have been passed on again is One Core's
  # own fault.
  defp ended(id, outcome) do
    case {get(id), outcome} do
      {nil, _outcome} -> :ok
      {%{state: :checking}, :forwarding_loop} -> refuse(id, Server.forwarding_loop(), "It would be passed on again")
      {%{state: :checking}, {:error, why}} -> refuse(id, -32602, why)
      {%{state: :checking}, _answered} -> with(:ok <- move(id, :running), do: move(id, :completed))
      {_working, :forwarding_loop} -> move(id, :failed, "It would be passed on again")
      {_working, _answered} -> move(id, :completed)
    end
  end

  @doc "This process's call: its id and its row, or `nil`."
  def current do
    with [id | _] <- Process.get(@stack, []), %{} = row <- get(id), do: {id, row}, else: (_ -> nil)
  end

  @doc "This process's call's id, or `nil`."
  def id, do: with({id, _row} <- current(), do: id)

  @doc """
  Has this process act for the call `id`, as a process a call started to do part of it does: what it's passed on
  and says is that call's.
  """
  def follow(nil), do: :ok
  def follow(id), do: Process.put(@stack, [id | Process.get(@stack, [])])

  @doc "The row of the call `id`, or `nil`."
  def get(id) do
    case :ets.lookup(@table, id) do
      [{^id, _version, row}] -> row
      [] -> nil
    end
  end

  @doc "The calls the process `pid` began and hasn't finished: each one's id and row."
  def left(pid),
    do: :ets.select(@table, [{{:"$1", :_, :"$2"}, [{:==, {:map_get, :pid, :"$2"}, pid}], [{{:"$1", :"$2"}}]}])

  @doc "The call that's the MCP task `task`: its id and its row, or `nil`."
  def task(task) do
    case :ets.select(@table, [{{:"$1", :_, :"$2"}, [{:==, {:map_get, :task, :"$2"}, task}], [{{:"$1", :"$2"}}]}]) do
      [found | _] -> found
      [] -> nil
    end
  end

  @doc """
  Moves the call `id` to `to`, saying `message` if it's a state of working or final. Gives `:ok`, or `{:refused,
  state}` with the state it was in where the table has no such move from it, as from a call already cancelled.
  """
  def move(id, to, message \\ nil) do
    changed(id, fn row ->
      if to in Map.fetch!(@moves, row.state),
        do: {:ok, %{row | state: to, message: message}},
        else: {:refused, row.state}
    end)
  end

  @doc """
  Refuses the call `id`, which is checking, with the JSON-RPC error `code` and `why`: what a caller that asked for
  a task is answered (`OneCore.Tasks`).
  """
  def refuse(id, code, why) do
    changed(id, fn
      %{state: :checking} = row -> {:ok, Map.merge(row, %{state: :refused, message: why, code: code})}
      row -> {:refused, row.state}
    end)
  end

  @doc """
  The call `id` is accepted: it has passed its checks and is running. A caller that asked for a task is answered it
  now, by `made`, which keeps the task and gives it, before the tool runs.
  """
  def accepted(id, made) do
    case get(id) do
      %{state: :checking, task: nil, requester: requester} = row when is_pid(requester) ->
        task = made.(row)
        changed(id, fn row -> {:ok, %{row | task: task["taskId"], state: :running}} end)
        send(requester, {__MODULE__, :accepted, task})
        :ok

      %{state: :checking} ->
        move(id, :running)

      _ ->
        :ok
    end
  end

  @doc """
  The call `id` is the MCP task `task` from now on, kept for it though its caller didn't ask for one
  (`OneCore.Tasks.within/3`), or with `nil`, no longer is.
  """
  def tasked(id, task), do: changed(id, fn row -> {:ok, %{row | task: task}} end)

  @doc """
  Says what the call `id` is doing, for a state of working: `routing` to the server it's passed on to, `waiting` for
  something to be ready, and `running` once it runs again, with what the app last said.
  """
  def working(nil, _to, _message), do: :ok

  def working(id, to, message) when to in @working do
    with %{state: state} when state != to <- get(id), do: move(id, to, message)
    with %{state: ^to} <- get(id), do: changed(id, fn row -> {:ok, %{row | message: message}} end)
    told(id)
  end

  @doc """
  Says what the app running the call `id` is doing (`OneCore.Call.stage/2`), as its task's `statusMessage` and the
  progress its caller is told. A call that was cancelled raises here instead, so what it began cleans up after it.
  """
  def stage(nil, _text), do: :ok

  def stage(id, text) when is_binary(text) do
    changed(id, fn
      %{state: :cancelled} -> {:refused, :cancelled}
      row -> {:ok, %{row | stage: text}}
    end)
    |> case do
      {:refused, :cancelled} -> stopped(id)
      _ -> told(id)
    end
  end

  # A cancelled call stops at what it calls next, which raises in it, before what it would have done after. Its row
  # says so, as one that ran to its end anyway, having called neither, isn't stopped: it has its result.
  defp stopped(id) do
    changed(id, fn row -> {:ok, Map.put(row, :stopped, true)} end)
    raise Cancelled
  end

  # What a call is doing, which its caller is told in its progress: what it said in the state it's in, or the
  # app's last stage while it runs.
  @doc false
  def message(%{state: :running, stage: stage}), do: stage
  def message(%{message: message}), do: message

  defp told(id) do
    with %{ask: ask} = row when ask != nil <- get(id),
         text when is_binary(text) <- message(row),
         do: Iroh.stage(ask, text)

    :ok
  end

  @doc """
  Asks the person behind the call `id` `question`, MCP's elicitation's params, and gives their answer, MCP's
  elicitation result: the call is `input_required` until it comes. Only a task is asked, by its `inputRequests`,
  since a caller that waits on a call has no way to answer it.
  """
  def ask(id, question) do
    key =
      changed(id, fn
        %{state: :cancelled} ->
          {:refused, :cancelled}

        %{task: nil} ->
          {:refused, :plain}

        row ->
          key = "ask-#{row.asked + 1}"
          request = %{"method" => "elicitation/create", "params" => question}
          inputs = Map.put(row.inputs, key, %{request: request, pid: self()})
          state = if row.state == :running, do: :input_required, else: row.state
          {:ok, %{row | inputs: inputs, asked: row.asked + 1, state: state}, key}
      end)

    case key do
      {:refused, :cancelled} -> stopped(id)
      {:refused, :plain} -> raise "It asks a person something, which a call asks only when it's made as an MCP task"
      {:refused, state} -> raise "It can't ask anything once it's #{state}"
      key -> answer_of(id, key)
    end
  end

  defp answer_of(id, key) do
    receive do
      {__MODULE__, :answered, ^key, answer} ->
        changed(id, fn
          %{state: :input_required, inputs: inputs} = row when inputs == %{} -> {:ok, %{row | state: :running}}
          row -> {:ok, row}
        end)

        answer

      {__MODULE__, :cancelled, ^id} ->
        stopped(id)
    end
  end

  @doc """
  Hands the call that's the task `task` the answers to what it asked, by their keys: what's asked that's no longer
  outstanding is ignored, as MCP's Tasks has it.
  """
  def answer(task, answers) do
    with {id, _row} <- task(task) do
      for {key, answer} <- answers do
        taken =
          changed(id, fn row ->
            case Map.pop(row.inputs, key) do
              {nil, _inputs} -> {:refused, :absent}
              {input, inputs} -> {:ok, %{row | inputs: inputs}, input}
            end
          end)

        with %{pid: pid} <- taken, do: send(pid, {__MODULE__, :answered, key, answer})
      end
    end

    :ok
  end

  @doc """
  Cancels the call `id`: it's `cancelled` at once, the server it's passed on to is told, and its process is stopped,
  first by what it calls next raising (`stage/2`, `ask/2`) and then outright, if it hasn't ended after a few seconds.
  Gives `:ok`, or `{:refused, state}` for one already final.
  """
  def cancel(id, why \\ "It was cancelled.") do
    with :ok <- move(id, :cancelled, why) do
      row = get(id)
      with {server, request} <- row.forwarded, do: Server.cancel(server, request)
      send(row.pid, {__MODULE__, :cancelled, id})
      :timer.kill_after(@grace, row.pid)
      :ok
    end
  end

  @doc "Cancels the call a client asked for by `request`, who sent it and its id, as `notifications/cancelled` does."
  def cancel_request(request) do
    for {id, %{request: ^request, task: nil}} <- :ets.select(@table, [{{:"$1", :_, :"$2"}, [], [{{:"$1", :"$2"}}]}]),
        do: cancel(id)

    :ok
  end

  @doc """
  Tells each call a client asked for by `request` that isn't a task that the client went away before its answer:
  `{OneCore.Call.State, :gone, id}`. What a call does then is its own: a code mode run is cancelled
  (`OneCore.CodeMode`), and any other runs to its end, as MCP's Streamable HTTP has a client's going.
  """
  def gone(request) do
    for {id, %{request: ^request, task: nil, pid: pid}} <-
          :ets.select(@table, [{{:"$1", :_, :"$2"}, [], [{{:"$1", :"$2"}}]}]),
        do: send(pid, {__MODULE__, :gone, id})

    :ok
  end

  @doc """
  Has `fun` ask `server` what was asked of this process's call, as the request `request`: the call is routed there
  meanwhile, so cancelling it tells that server too.
  """
  def forwarding(server, request, fun) do
    case current() do
      nil ->
        fun.()

      {id, row} ->
        working(id, :routing, "Passed on to #{Servers.name(server) || "another server"}")
        changed(id, fn row -> {:ok, %{row | forwarded: {server, request}}} end)

        try do
          fun.()
        after
          changed(id, fn row -> {:ok, %{row | forwarded: nil}} end)
          if row.state in @working, do: working(id, row.state, row.message)
        end
    end
  end

  @doc """
  The MCP request this process's call answers, as this server took it (`OneCore.Iroh`), or `nil`: what a server it's
  passed on to says it's doing is told whoever asked it.
  """
  def answering do
    case current() do
      {_id, %{ask: ask}} -> ask
      _ -> nil
    end
  end

  @doc "Whether `state` is final, which nothing moves out of."
  def final?(state), do: state in @final

  # Changes the row of `id` by `change`, which gives `{:ok, row}` or `{:ok, row, reply}`, or `{:refused, why}` to
  # leave it: only if nothing changed it meanwhile, else it's given the row again. Gives `:ok`, the reply, or what
  # refused it. A call whose row has gone changes nothing.
  defp changed(id, change) do
    case :ets.lookup(@table, id) do
      [{^id, version, row}] ->
        {outcome, now, reply} =
          case change.(row) do
            {:ok, now} -> {:ok, now, :ok}
            {:ok, now, reply} -> {:ok, now, reply}
            refused -> {refused, row, nil}
          end

        cond do
          outcome != :ok ->
            outcome

          now == row ->
            reply

          true ->
            now = %{now | updated: System.system_time(:millisecond)}
            swap = [{{id, version, :_}, [], [{:const, {id, version + 1, now}}]}]
            if :ets.select_replace(@table, swap) == 1, do: reply, else: changed(id, change)
        end

      [] ->
        {:refused, :ended}
    end
  end
end
