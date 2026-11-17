defmodule OneCore.Tasks do
  @moduledoc """
  MCP's Tasks, as its extension `io.modelcontextprotocol/tasks` has them from MCP's 2026-07-28: a tool call a client
  starts, carries on from, checks back on with `tasks/get`, answers with `tasks/update` when it asks something, and
  cancels with `tasks/cancel`.

  A tool is made a task by its app, which says `task_support: :optional` or `:required` beside its name. A client
  that says it takes tasks, in its request's `_meta` (`io.modelcontextprotocol/clientCapabilities`), is answered a task
  for a call of one of those once the call is accepted (`OneCore.Call.State`); one that doesn't is answered as it
  always was, and a call of one that's `:required` is refused (`-32021`). A call is never a task for whoever
  calls with no person behind them, as an app, a script or the clock does, which waits on its answer.

  A task is its call, running on the server that runs it, in a process of its own, so it carries on whatever its
  caller does. What it is is kept in the bucket, `<whoever it runs for>/tasks/<id>`, which is its `taskId` too, so any
  server finds it: when it's made, and once it ends, for an hour after. While it runs, the server running it answers
  for it, and one that has stopped meanwhile, or whose run ended without returning, is taken to have failed it. Only
  whoever started a task gets it, answers it or cancels it: to anyone else, it isn't there.
  """
  alias OneCore.{Artifact, Call, Server, Servers, Trace, User}
  alias OneCore.Call.State

  # The extension, as a client says it takes it and a server says it has it.
  @extension "io.modelcontextprotocol/tasks"
  @capabilities "io.modelcontextprotocol/clientCapabilities"

  # How long a task is kept once it ends, and how often a client is asked to check on one, in milliseconds.
  @kept :timer.hours(1)
  @poll 1_000

  @doc "What a server says it has of tasks, among its capabilities' extensions."
  def extension, do: %{@extension => %{}}

  @doc "Whether the request `params` say its client takes tasks."
  def capable?(%{"_meta" => %{@capabilities => %{"extensions" => %{@extension => _}}}}), do: true
  def capable?(_params), do: false

  @doc "The error a request is refused with when its client doesn't say it takes tasks and it needs one."
  def required,
    do: {:error, -32021, "Missing required client capability", %{requiredCapabilities: %{extensions: extension()}}}

  @doc """
  What the request this process's call passes on to another server says of its client: that it takes tasks, where
  its caller asked for one and none is made yet, so that server makes it, since the call runs there.
  """
  def asking do
    case State.current() do
      {_id, %{requester: _requester, task: nil}} -> %{@capabilities => %{"extensions" => extension()}}
      _ -> %{}
    end
  end

  @doc "How a tool's app says it's made a task, `:optional` or `:required`, or `nil` for never."
  def support(tool), do: tool[:task_support]

  @doc """
  Runs `run`, a tool call that gives what MCP's `tools/call` answers, as a task if it's accepted: in a process of its
  own, which answers the task here as soon as it is, and keeps its result once it ends. A call that's refused gives
  the protocol error it was refused with, and one passed on to another server that answered, what that one did.
  """
  def start(run, tool, principal) do
    requester = self()
    trace = Trace.traceparent()

    {:ok, runner} =
      Task.Supervisor.start_child(OneCore.Asks, fn ->
        Trace.follow(trace)
        id = State.begin(%{requester: requester, principal: principal, tool: tool})

        outcome =
          try do
            run.()
          rescue
            error -> {:error, -32603, Exception.message(OneCore.Redact.redact(error))}
          end

        row = State.get(id)
        if row.task, do: ended(row, outcome), else: send(requester, {__MODULE__, :ended, row, outcome})
        State.finish(id)
      end)

    Task.Supervisor.start_child(OneCore.Asks, fn -> watched(runner) end)
    watched = Process.monitor(runner)

    receive do
      {State, :accepted, task} ->
        Process.demonitor(watched, [:flush])
        {:ok, Map.put(task, "resultType", "task")}

      {__MODULE__, :ended, %{state: :refused, code: code, message: why}, _outcome} ->
        Process.demonitor(watched, [:flush])
        {:error, code, why}

      {__MODULE__, :ended, _row, outcome} ->
        Process.demonitor(watched, [:flush])
        outcome

      {:DOWN, ^watched, :process, _runner, why} ->
        {:error, -32603, "It stopped before it was accepted: #{Exception.format_exit(why)}"}
    end
  end

  # What a task runs in can end without returning, as it does when a process it's linked to crashes, or when it's
  # stopped outright once cancelled. Its task has then failed, as MCP's Tasks has one whose run fails (or stays
  # cancelled), and what it began goes.
  defp watched(runner) do
    monitor = Process.monitor(runner)

    receive do
      {:DOWN, ^monitor, :process, _runner, why} ->
        for {id, row} <- State.left(runner) do
          if row.task, do: ended(row, {:error, -32603, "It ended without an answer: #{Exception.format_exit(why)}"})
          State.finish(id)
        end
    end
  end

  @doc """
  Makes the task a call that was accepted is, from its row (`OneCore.Call.State`): kept in the bucket before it's
  given, so any server finds it as soon as its caller asks. Gives the task as MCP has it.
  """
  def made(%{principal: {actor, subject}, tool: tool, created: created}) do
    name = "#{created}-#{Base.url_encode64(:crypto.strong_rand_bytes(18), padding: false)}"

    task = %{
      "taskId" => "#{subject}/tasks/#{name}",
      "status" => "working",
      "createdAt" => at(created),
      "lastUpdatedAt" => at(created),
      "ttlMs" => nil,
      "pollIntervalMs" => @poll
    }

    record = Map.merge(task, %{"actor" => actor, "server" => Server.key(), "tool" => tool})
    :ok = Artifact.write(artifact(subject), name, JSON.encode!(record))
    Task.Supervisor.start_child(OneCore.Asks, fn -> swept(subject) end)
    task
  end

  @doc """
  Answers `tasks/get`, `tasks/update` or `tasks/cancel` for whoever asked, `principal`: `{:ok, result}`, `{:error, code,
  why}`, or `{:forwarded, result}`, what a server acting for whoever the task runs for answered it.
  """
  def handle(method, %{"taskId" => id} = params, {actor, _subject} = principal, forwarded) when is_binary(id) do
    with {:ok, subject, name} <- named(id),
         :here <- here(method, params, principal, forwarded, User.owner(subject)),
         {:ok, record} <- record(subject, name, actor) do
      handled(method, params, record, subject, name, principal, forwarded)
    end
  end

  def handle(_method, _params, _principal, _forwarded), do: {:error, -32602, "A task is asked for by its taskId"}

  # A task whose call is over is answered from what's kept of it. One still running is answered by the server
  # running it, this one or another, and one whose server has stopped has failed.
  defp handled(method, params, record, subject, name, principal, forwarded) do
    cond do
      final?(record) -> {:ok, over(method, record)}
      record["server"] == Server.key() -> running(method, params, record, subject, name)
      # Another server that passed it on here took this one for the one running it: it's as it was kept.
      forwarded != false -> {:ok, over(method, record)}
      true -> elsewhere(method, params, record, subject, name, principal)
    end
  end

  defp over("get", record), do: complete(shown(record))
  defp over(_method, _record), do: complete(%{})

  defp running(method, params, %{"taskId" => id} = record, subject, name) do
    case {method, State.task(id)} do
      {_method, nil} ->
        {:ok, over(method, gone(subject, name, Server.key()))}

      {"get", {_call, row}} ->
        {:ok, complete(detailed(record, row))}

      {"update", _found} ->
        State.answer(id, Map.get(params, "inputResponses") || %{})
        {:ok, complete(%{})}

      {"cancel", {call, _row}} ->
        with :ok <- State.cancel(call),
             do: ended(subject, name, %{"status" => "cancelled", "statusMessage" => "It was cancelled."})

        {:ok, complete(%{})}
    end
  end

  defp elsewhere(method, params, record, subject, name, {actor, for}) do
    server = record["server"]
    meta = %{@capabilities => %{"extensions" => extension()}, actor: actor, subject: for, forwarded: true}
    params = params |> Map.delete("_meta") |> Map.put(:_meta, meta)

    case Server.request(server, "tasks/" <> method, params) do
      {:ok, result} -> {:ok, result}
      {:error, code, why} -> {:error, code, why}
      _down -> {:ok, over(method, gone(subject, name, server))}
    end
  end

  # A task is read where whoever it runs for is: by a server acting for them, which this one asks if it isn't one.
  defp here(method, params, principal, forwarded, owner) do
    if Servers.here?(owner) do
      :here
    else
      meta = %{@capabilities => %{"extensions" => extension()}}

      Call.forward(
        {"tasks/" <> method, params |> Map.delete("_meta") |> Map.put(:_meta, meta)},
        principal,
        forwarded,
        owner
      )
    end
  rescue
    error -> {:error, -32603, Exception.message(error)}
  catch
    :throw, {Call, {:forwarded, result}} -> {:forwarded, result}
    :throw, {Call, :forwarding_loop} -> {:error, Server.forwarding_loop(), "This server would pass that on in turn"}
  end

  # What a task's id names: whoever it runs for, and its record's name among theirs.
  defp named(id) do
    with [subject, name] <- String.split(id, "/tasks/", parts: 2),
         true <- User.owner?(subject) and name =~ ~r/^[0-9]+-[A-Za-z0-9_-]+$/ do
      {:ok, subject, name}
    else
      _ -> not_found()
    end
  end

  # The task kept as `name` for `subject`, if `actor` started it: to anyone else it isn't there. One kept past its
  # hour is removed.
  defp record(subject, name, actor) do
    with true <- is_binary(actor) and not User.link?(actor),
         {:ok, data, _tag} <- Artifact.read(artifact(subject), name),
         %{"actor" => ^actor} = record <- JSON.decode!(data) do
      if expired?(record) do
        Artifact.remove(artifact(subject), name)
        {:error, -32602, "Failed to retrieve task: Task has expired"}
      else
        {:ok, record}
      end
    else
      _ -> not_found()
    end
  end

  defp not_found, do: {:error, -32602, "Failed to retrieve task: Task not found"}

  @doc false
  # Keeps how the call that's a task ended, from the process that ran it: what it answered, or the JSON-RPC error
  # it ended with. One cancelled meanwhile was kept so by whoever cancelled it.
  def ended(%{task: id, state: state}, outcome) do
    {:ok, subject, name} = named(id)

    case {state, outcome} do
      {:cancelled, _outcome} -> :ok
      {_state, {:ok, result}} -> ended(subject, name, %{"status" => "completed", "result" => result})
      {_state, {:error, code, why}} -> ended(subject, name, failed(code, why))
      {_state, {:error, code, why, _data}} -> ended(subject, name, failed(code, why))
    end
  end

  defp ended(subject, name, changes) do
    Artifact.update(artifact(subject), name, fn was ->
      record = JSON.decode!(was)

      if final?(record) do
        was
      else
        now = System.system_time(:millisecond)
        created = millisecond(record["createdAt"])

        JSON.encode!(
          Map.merge(record, Map.merge(changes, %{"lastUpdatedAt" => at(now), "ttlMs" => now - created + @kept}))
        )
      end
    end)
    |> JSON.decode!()
  end

  defp failed(code, why),
    do: %{"status" => "failed", "statusMessage" => why, "error" => %{"code" => code, "message" => why}}

  # A task whose server stopped while it ran it, or no longer has it, as one that started again doesn't, failed.
  defp gone(subject, name, server) do
    why = "#{Servers.name(server) || "Its server"} stopped while it ran this, so it may or may not have been done"
    ended(subject, name, failed(-32603, why))
  end

  # A running task as MCP has it: working, in any of working's states, or asking for input, with what it's doing.
  defp detailed(record, row) do
    status =
      case row.state do
        :input_required -> "input_required"
        :cancelled -> "cancelled"
        _working -> "working"
      end

    shown = Map.merge(shown(record), %{"status" => status, "lastUpdatedAt" => at(row.updated)})
    shown = if message = State.message(row), do: Map.put(shown, "statusMessage", message), else: shown

    if status == "input_required",
      do: Map.put(shown, "inputRequests", Map.new(row.inputs, fn {key, %{request: request}} -> {key, request} end)),
      else: shown
  end

  # What's kept of a task that its caller is shown: MCP's task, without who started it and where it runs.
  defp shown(record), do: Map.drop(record, ["actor", "server", "tool"])

  defp complete(result), do: Map.put(result, "resultType", "complete")

  defp final?(%{"status" => status}), do: status in ["completed", "failed", "cancelled"]

  defp expired?(%{"ttlMs" => ttl, "createdAt" => created}) when is_integer(ttl),
    do: millisecond(created) + ttl < System.system_time(:millisecond)

  defp expired?(_record), do: false

  # Removes what's kept of `subject`'s tasks that ended over an hour ago, and has those whose server has stopped, or
  # started again, which loses what it was running, fail, as each task made does: so none is kept for long, however
  # few are asked about again.
  defp swept(subject) do
    old = System.system_time(:millisecond) - @kept

    for name <- Artifact.list(artifact(subject)),
        [created | _] = String.split(name, "-", parts: 2),
        String.to_integer(created) < old,
        {:ok, data, _tag} <- [Artifact.read(artifact(subject), name)] do
      record = JSON.decode!(data)

      cond do
        expired?(record) ->
          Artifact.remove(artifact(subject), name)

        final?(record) ->
          :ok

        record["server"] == Server.key() and State.task(record["taskId"]) == nil ->
          gone(subject, name, Server.key())

        record["server"] != Server.key() and
            Servers.presence(record["server"])["server_daemon_status"] in ["starting", "stopped"] ->
          gone(subject, name, record["server"])

        true ->
          :ok
      end
    end
  end

  defp artifact(subject), do: %Artifact{prefix: "#{subject}/tasks/"}

  defp at(millisecond), do: millisecond |> DateTime.from_unix!(:millisecond) |> DateTime.to_iso8601()

  defp millisecond(at) do
    {:ok, at, _offset} = DateTime.from_iso8601(at)
    DateTime.to_unix(at, :millisecond)
  end

  @doc """
  What `OneCore.Call.ask/2` asks, as MCP's elicitation's params: a form with what its `requestedSchema` says, or for
  only a message, nothing to fill in but yes or no.
  """
  def question(message) when is_binary(message),
    do: %{"mode" => "form", "message" => message, "requestedSchema" => %{"type" => "object", "properties" => %{}}}

  def question(%{} = question), do: Map.merge(%{"mode" => "form"}, JSON.decode!(JSON.encode!(question)))
end
