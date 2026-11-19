defmodule OneCore.CodeMode do
  @moduledoc """
  Code mode's `execute` and `call` over the web, as Executor has them. `execute` runs a short script that calls tools,
  by OpenCode's interpreter in a QuickJS sandbox of the server's own (`OneCore.CodeMode.Runner`), where nothing but
  `tools` reaches the world. Each call it makes comes back here and is run as whoever asked, exactly as their MCP call would be, its
  inputs checked against what the tool takes. `call` makes one call or several at once with no script. Every client
  has both run here, `one mcp --code` too, and no device fills in a script's inputs: a secret comes from its person
  through the Vault's `request`, and what needs a device's own key is a plain tool on that device.

  A run of one script, `codemode_run`, is in one of three states, and nothing it starts outlives it:

      running ─done / cancelled / caller gone─► stopping (its calls cancelled) ─► stopped

  It runs until its script is done, with no limit on its time or its calls, as OpenCode's Core has it. It's cancelled
  as any call is (`OneCore.Call.State`): by `notifications/cancelled`, or `tasks/cancel` when it runs as an MCP task.
  Its caller going away cancels it too, unless it's a task, which outlives its caller's connection by design. Stopping,
  its runner is stopped, and every call it started that's still in flight is cancelled and awaited, so none ends after
  whoever asked was told it stopped. What it gives back says which of its calls finished and which were cancelled.
  """
  alias OneCore.Call.State
  alias OneCore.CodeMode.Runner

  # The processes a run started, whose ends it hears of as messages while it traps exits.
  @pids :"$one_codemode_run"

  @doc """
  `call`'s answer: each of `calls` run at once by `call`, given a tool's name and inputs as `run/4`'s is, `named`
  giving the name of the tool at each path a script reaches. Each answers, in order, with its `path`, `ok`, and its
  `value` or `error`, and what isn't text of its answer, such as a picture, is MCP's content after it
  (`{:content, answer, parts}`, `OneCore.Apps.Calls.structured/1`). Raises with them all where every one failed.
  """
  def calls(calls, named, call) do
    calls
    |> List.wrap()
    |> Task.async_stream(
      fn request ->
        path = request["path"]
        name = named[path]

        if name,
          do: {path, answered(call, name, request["input"])},
          else: {path, {:error, "#{path} isn't a tool's path. search gives each exactly, as tools.gmail.search."}}
      end,
      timeout: :infinity,
      ordered: true
    )
    |> Enum.map(fn {:ok, answer} -> answer end)
    |> then(fn answers ->
      calls =
        for {path, answer} <- answers do
          case answer do
            {:ok, result} -> %{path: path, ok: true, value: value(result)}
            {:error, why} -> %{path: path, ok: false, error: why}
          end
        end

      # What isn't text of an answer, as a picture is, stays MCP's content, for whoever called to see.
      files = for {_path, {:ok, result}} <- answers, part <- result["content"], part["type"] != "text", do: part

      if Enum.all?(answers, &match?({_, {:error, _}}, &1)),
        do: raise(JSON.encode!(%{calls: calls})),
        else: {:content, %{calls: calls}, files}
    end)
  end

  @doc "A tool's path in a script, as MCP lists it: `gmail.search`, or One Core's own by its name, `deploy`."
  def path(%{_meta: %{app: app}} = tool), do: app <> "." <> String.replace_prefix(tool.name, app <> "_", "")
  def path(tool), do: tool.name

  @doc """
  Runs `code` with `tools`, each `%{name, app, description, input, output}`, and `namespaces`, what each app is for by
  its name, as this process's call (`OneCore.Call.State`). `call` runs one: given its name and inputs, it gives MCP's
  result of calling it. Gives `{:ok, text}` or `{:error, text}`: what it returned or what went wrong, with what it
  logged and which of its calls finished and which were cancelled.
  """
  def run(code, tools, namespaces, call) do
    tag = System.unique_integer()
    # The interpreter, bundled as this server's build has it (`core/codemode`).
    bundle = File.read!(Application.app_dir(:one_core, "priv/codemode.js"))
    runner = Runner.start(bundle, JSON.encode!(%{code: code, tools: tools, namespaces: namespaces}), tag)

    # What it starts is linked to it, so a run that's killed takes its calls with it, and they end as messages
    # here meanwhile, as its caller's going does.
    trapping = Process.flag(:trap_exit, true)

    run = %{codemode_run: :running, runner: runner, tag: tag, call: call, calls: %{}, ended: []}

    try do
      listen(run)
    after
      Runner.stop(runner)
      Process.flag(:trap_exit, trapping)
      flushed(Process.get(@pids, []))
      told(tag)
      Process.delete(@pids)
    end
  end

  # Running: the runner tells of each call to run, which runs beside the others, or what the script came to.
  defp listen(%{codemode_run: :running, runner: runner, tag: tag} = run) do
    receive do
      {:codemode, ^tag, {:call, line}} ->
        %{"call" => id, "name" => name} = request = JSON.decode!(line)
        listen(started(run, id, name, request["input"]))

      {:codemode, ^tag, {:done, ok, text}} ->
        stopping(run, {if(ok, do: :ok, else: :error), text}, nil)

      {:codemode, ^tag, {:exit, why}} ->
        stopping(run, {:error, "The script stopped without an answer: " <> why}, nil)

      {ref, {result, _stopped}} when is_map_key(run.calls, ref) ->
        Runner.reply(runner, JSON.encode!(Map.put(result, :reply, run.calls[ref].id)))
        listen(ended(run, ref, :finished))

      # A call whose process ended without answering, as one does when a process it's linked to crashes, failed: the
      # script's call throws why, and the run goes on.
      {:DOWN, ref, :process, _pid, why} when is_map_key(run.calls, ref) ->
        State.finish(run.calls[ref].call)
        failed = %{error: "It ended without an answer: " <> Exception.format_exit(why), reply: run.calls[ref].id}
        Runner.reply(runner, JSON.encode!(failed))
        listen(ended(run, ref, :unanswered))

      # Its own process is stopped outright once its grace is over, so it gives its calls a little less.
      {State, :cancelled, _id} ->
        stopping(run, {:error, "It was cancelled."}, System.monotonic_time(:millisecond) + State.grace() - 1_000)

      # Its caller went away (`OneCore.Call.State.gone/1`), which is its being cancelled.
      {State, :gone, id} ->
        State.cancel(id, "Its caller went away.")
        listen(run)

      # What started it is stopping, and it stops with it, once its calls have.
      {:EXIT, from, why} ->
        if from in Process.get(@pids, []) do
          listen(run)
        else
          stopping(run, {:error, "It was stopped."}, nil)
          exit(why)
        end
    end
  end

  # A call the script made, run beside its others in a process of its own, as a call of its own that stopping
  # cancels. The run waits for it to begin, so it has every call's id before it can stop.
  defp started(run, id, name, input) do
    here = self()

    task =
      Task.async(fn ->
        call = State.begin()
        send(here, {__MODULE__, :began, self(), call})

        # Its answer, and whether it stopped at its cancel before its end, which its state says.
        try do
          {called(run.call, name, input), State.get(call)[:stopped] == true}
        after
          State.finish(call)
        end
      end)

    pid = task.pid
    Process.put(@pids, [pid | Process.get(@pids, [])])
    call = receive do: ({__MODULE__, :began, ^pid, call} -> call)
    %{run | calls: Map.put(run.calls, task.ref, %{id: id, name: name, call: call, pid: pid})}
  end

  # A call that ended, as `how`: `:finished` with its answer, `:cancelled`, or `:unanswered`, which may or may not have
  # done what it does.
  defp ended(run, ref, how) do
    Process.demonitor(ref, [:flush])
    {%{name: name}, calls} = Map.pop(run.calls, ref)
    %{run | calls: calls, ended: [{name, how} | run.ended]}
  end

  # Stopping: the runner is stopped, so the script calls nothing more, and each call still in flight is cancelled and
  # awaited, until `until` if the run has to end by then: what hasn't ended is stopped outright. How each ended is what
  # it gave: one with its answer finished, however late it was cancelled.
  defp stopping(run, result, until) do
    run = %{run | codemode_run: :stopping}
    Runner.stop(run.runner)
    for {_ref, %{call: call}} <- run.calls, do: State.cancel(call, "The script it was called from stopped.")
    ended = for {ref, %{pid: pid, call: call, name: name}} <- run.calls, do: {name, awaited(ref, pid, call, until)}
    stopped(%{run | codemode_run: :stopped, calls: %{}}, result, Enum.reverse(run.ended) ++ ended)
  end

  # Stopped: nothing it started is left, and it says which of its calls finished and which were cancelled.
  defp stopped(%{codemode_run: :stopped}, {done, text}, ended), do: {done, text <> said(ended)}

  # A cancelled call ends by what it calls next raising, or else is stopped by the call machine, or by the run if it
  # has to end first, which then ends its state too. Gives how it ended, as `ended/3` has it.
  defp awaited(ref, pid, call, until) do
    left = if until, do: max(until - System.monotonic_time(:millisecond), 0), else: :infinity

    receive do
      {^ref, {_result, stopped}} ->
        Process.demonitor(ref, [:flush])
        if stopped, do: :cancelled, else: :finished

      {:DOWN, ^ref, :process, _pid, _why} ->
        State.finish(call)
        :unanswered
    after
      left ->
        Process.exit(pid, :kill)
        receive do: ({:DOWN, ^ref, :process, _pid, _why} -> State.finish(call))
        :unanswered
    end
  end

  defp flushed(pids) do
    for pid <- pids do
      receive do
        {:EXIT, ^pid, _why} -> :ok
      after
        0 -> :ok
      end
    end
  end

  # What a runner said as it was stopped, which nothing hears now.
  defp told(tag) do
    receive do
      {:codemode, ^tag, _} -> told(tag)
    after
      0 -> :ok
    end
  end

  defp said([]), do: ""

  # Each tool once, with how many times it was called where that's more than once.
  defp said(ended) do
    names = fn how ->
      case for({name, ^how} <- ended, do: name) do
        [] ->
          "none"

        names ->
          Enum.map_join(Enum.frequencies(names), ", ", fn
            {name, 1} -> name
            {name, times} -> "#{name} ×#{times}"
          end)
      end
    end

    unanswered =
      if List.keymember?(ended, :unanswered, 1),
        do: " Ended without answering, so they may or may not have been done: #{names.(:unanswered)}.",
        else: ""

    "\n\nCalls finished: #{names.(:finished)}. Cancelled, as the script stopped first: #{names.(:cancelled)}." <>
      unanswered
  end

  # One call, as MCP answered it: what it answered as JSON (`structuredContent`). An error is what the script's call
  # throws.
  defp called(call, name, input) do
    case answered(call, name, input) do
      {:ok, result} -> %{ok: value(result)}
      {:error, why} -> %{error: why}
    end
  end

  # MCP's result of a call, as JSON has it.
  defp answered(call, name, input) do
    case call.(name, input || %{}) do
      {:ok, result} ->
        case OneCore.JSON.normalize(result) do
          %{"isError" => true, "content" => content} -> {:error, texts(content)}
          result -> {:ok, result}
        end

      {:error, _code, why} ->
        {:error, why}

      {:error, why} ->
        {:error, why}
    end
  rescue
    error -> {:error, Exception.message(error)}
  catch
    :exit, why -> {:error, Exception.format_exit(why)}
  end

  # What a call answered as JSON. A tool of a version deployed before tools said what they answer has none, so its
  # text is read: text that's JSON is that, and other text is a string.
  defp value(%{"structuredContent" => structured}), do: structured

  defp value(%{"content" => content}) do
    text = texts(content)

    case JSON.decode(text) do
      {:ok, value} -> value
      _ -> text
    end
  end

  defp texts(content), do: content |> Enum.filter(&(&1["type"] == "text")) |> Enum.map_join("\n", & &1["text"])
end
