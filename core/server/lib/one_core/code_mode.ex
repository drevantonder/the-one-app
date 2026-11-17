defmodule OneCore.CodeMode do
  @moduledoc """
  Code mode's `execute` and `call` over the web, as Executor has them. `execute` runs a short script that calls tools,
  by OpenCode's interpreter under this server's Bun (`core/codemode/run.js`), where nothing but `tools` reaches the
  world. Each call it makes comes back here and is run as whoever asked, exactly as their MCP call would be, its
  inputs checked against what the tool takes. `call` makes one call or several at once with no script. Every client
  has both run here, `one mcp --code` too, and no device fills in a script's inputs: a secret comes from its person
  through the Vault's `request`, and what needs a device's own key is a plain tool on that device.

  A run of one script, `codemode_run`, is in one of three states, and nothing it starts outlives it:

      running ─done / cancelled / caller gone─► stopping (its calls cancelled) ─► stopped

  It runs until its script is done, with no limit on its time or its calls, as OpenCode's Core has it. It's cancelled
  as any call is (`OneCore.Call.State`): by `notifications/cancelled`, or `tasks/cancel` when it runs as an MCP task.
  Its caller going away cancels it too, unless it's a task, which outlives its caller's connection by design. Stopping,
  its runner is killed, and every call it started that's still in flight is cancelled and awaited, so none ends after
  whoever asked was told it stopped. What it gives back says which of its calls finished and which were cancelled.
  """
  alias OneCore.{Entrypoint, Server}
  alias OneCore.Call.State

  # The processes a run started, whose ends it hears of as messages while it traps exits.
  @pids :"$one_codemode_run"

  @runner Path.expand("../../../codemode", __DIR__)
  @external_resource Path.join(@runner, "package.json")
  @external_resource Path.join(@runner, "bun.lock")
  @external_resource Path.join(@runner, "run.js")
  @files Map.new(~w(package.json bun.lock run.js), &{&1, File.read!(Path.join(@runner, &1))})

  @doc "`execute` and `call`, as MCP lists them."
  def tools, do: [execute(), call()]

  defp execute do
    %{
      name: "execute",
      description: """
      Runs a short script that calls tools, and gives back what it returns. It's JavaScript without imports, timers \
      or network: call a tool as `await tools.<app>.<tool>({ ... })`, or One Core's own as `await \
      tools.<tool>({ ... })` (search gives each exact call, and `search({ query })` in the script finds them too), use \
      plain JavaScript between calls, and `return` what you want back; what it logs comes back too. Use it where one \
      call's answer feeds another, or to cut a large answer down; for one call or a few that don't depend on each \
      other, use call. Each call runs as you, as any tool does, so what sends or can't be undone waits for the \
      person's yes, as always. It runs until it's done or cancelled, and gives back up to 60 KB; one that takes long \
      can run as a task. Never put a password, key or token in a script: for one the person pasted or must give, \
      call vault.request, which has them type it on their own device.\
      """,
      task_support: :optional,
      inputSchema: %{
        type: "object",
        properties: %{code: %{type: "string", description: "The script"}},
        required: ["code"]
      },
      outputSchema: %{
        type: "object",
        properties: %{text: %{type: "string", description: "What it returned, or what went wrong, with what it logged"}},
        required: ["text"]
      },
      annotations: %{readOnlyHint: false}
    }
  end

  defp call do
    request = %{
      type: "object",
      properties: %{
        path: %{type: "string", description: "The tool's path, exactly as search gives it: tools.gmail.search"},
        input: %{type: "object", description: "Its inputs, as its signature says"}
      },
      required: ["path"]
    }

    %{
      name: "call",
      description: """
      Calls tools without a script: give one { path, input }, or several in a list, which run at once. Each path is \
      exactly as search gives it, like tools.gmail.search. Answers each in the same order, with ok and its value or \
      its error; a picture comes back as one. Use execute instead where one call's answer feeds another, or to cut a \
      large answer down before it reaches you. Each call runs as you, as any tool does, so what sends or can't be \
      undone waits for the person's yes, as always. Never put a password, key or token in an input: for one the \
      person pasted or must give, call vault.request, which has them type it on their own device.\
      """,
      inputSchema: %{
        type: "object",
        properties: %{calls: %{anyOf: [request, %{type: "array", items: request, minItems: 1}]}},
        required: ["calls"]
      },
      outputSchema: %{
        type: "object",
        properties: %{
          calls: %{
            type: "array",
            description: "Each call's answer, in the order asked",
            items: %{
              type: "object",
              properties: %{
                path: %{type: "string"},
                ok: %{type: "boolean"},
                value: %{description: "What it answered, as its signature says"},
                error: %{type: "string", description: "Why it failed, where it did"}
              },
              required: ["path", "ok"]
            }
          }
        },
        required: ["calls"]
      },
      annotations: %{readOnlyHint: false}
    }
  end

  @doc """
  `call`'s answer, as MCP's result: each of `calls` run at once by `call`, given a tool's name and inputs as `run/4`'s
  is, `named` giving the name of the tool at each path a script reaches. Each answers, in order, with its `path`,
  `ok`, and its `value` or `error`, and what isn't text of its answer, such as a picture, comes after.
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

      %{
        content: [%{type: "text", text: JSON.encode!(%{calls: calls})} | files],
        structuredContent: %{calls: calls},
        isError: Enum.all?(answers, &match?({_, {:error, _}}, &1))
      }
    end)
  end

  @doc """
  Runs `code` with `tools`, each `%{name, app, description, input, output}`, and `namespaces`, what each app is for by
  its name, as this process's call (`OneCore.Call.State`). `call` runs one: given its name and inputs, it gives MCP's
  result of calling it. Gives `{:ok, text}` or `{:error, text}`: what it returned or what went wrong, with what it
  logged and which of its calls finished and which were cancelled.
  """
  def run(code, tools, namespaces, call) do
    with {:ok, bun} <- Entrypoint.bun(),
         {:ok, folder} <- runner(bun) do
      port =
        Port.open({:spawn_executable, bun}, [
          :binary,
          :exit_status,
          :stderr_to_stdout,
          {:line, 1_048_576},
          {:cd, folder},
          {:args, ["run.js"]},
          {:env, [{~c"DO_NOT_TRACK", ~c"1"}]}
        ])

      Port.command(port, JSON.encode!(%{code: code, tools: tools, namespaces: namespaces}) <> "\n")

      # What it starts is linked to it, so a run that's killed takes its calls with it, and they end as messages
      # here meanwhile, as its caller's going does.
      trapping = Process.flag(:trap_exit, true)

      run = %{codemode_run: :running, port: port, call: call, calls: %{}, finished: [], part: "", output: []}

      try do
        listen(run)
      after
        stop(port)
        Process.flag(:trap_exit, trapping)
        flushed([port | Process.get(@pids, [])])
        Process.delete(@pids)
      end
    end
  end

  # Running: each line from the runner is a call to run, which runs beside the others, or what the script came to.
  defp listen(%{codemode_run: :running, port: port} = run) do
    receive do
      {^port, {:data, {:noeol, more}}} ->
        listen(%{run | part: run.part <> more})

      {^port, {:data, {:eol, more}}} ->
        line = run.part <> more
        run = %{run | part: ""}

        case JSON.decode(line) do
          {:ok, %{"call" => id, "name" => name} = request} -> listen(started(run, id, name, request["input"]))
          {:ok, %{"done" => %{"ok" => ok, "text" => text}}} -> stopping(run, {if(ok, do: :ok, else: :error), text}, nil)
          _ -> listen(%{run | output: [line | run.output]})
        end

      {ref, result} when is_map_key(run.calls, ref) ->
        Port.command(port, JSON.encode!(Map.put(result, :reply, run.calls[ref].id)) <> "\n")
        listen(ended(run, ref))

      {^port, {:exit_status, _}} ->
        why = "The script stopped without an answer: " <> Enum.join(Enum.reverse(run.output), "\n")
        stopping(run, {:error, why}, nil)

      # Its own process is stopped outright once its grace is over, so it gives its calls a little less.
      {State, :cancelled, _id} ->
        stopping(run, {:error, "It was cancelled."}, System.monotonic_time(:millisecond) + State.grace() - 1_000)

      # Its caller went away (`OneCore.Call.State.gone/1`), which is its being cancelled.
      {State, :gone, id} ->
        State.cancel(id, "Its caller went away.")
        listen(run)

      # What started it is stopping, and it stops with it, once its calls have.
      {:EXIT, from, why} ->
        if from == port or from in Process.get(@pids, []) do
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

        try do
          called(run.call, name, input)
        after
          State.finish(call)
        end
      end)

    pid = task.pid
    Process.put(@pids, [pid | Process.get(@pids, [])])
    call = receive do: ({__MODULE__, :began, ^pid, call} -> call)
    %{run | calls: Map.put(run.calls, task.ref, %{id: id, name: name, call: call, pid: pid})}
  end

  defp ended(run, ref) do
    Process.demonitor(ref, [:flush])
    {%{name: name}, calls} = Map.pop(run.calls, ref)
    %{run | calls: calls, finished: [name | run.finished]}
  end

  # Stopping: the runner is killed, so the script calls nothing more, and each call still in flight is cancelled and
  # awaited, until `until` if the run has to end by then: what hasn't ended is stopped outright. One that ended
  # meanwhile, before it could be cancelled, finished.
  defp stopping(run, result, until) do
    run = %{run | codemode_run: :stopping}
    stop(run.port)

    cancelling =
      for {ref, %{call: call, name: name}} <- run.calls,
          do: {ref, name, State.cancel(call, "The script it was called from stopped.") == :ok}

    for {ref, %{pid: pid, call: call}} <- run.calls, do: awaited(ref, pid, call, until)
    finished = Enum.reverse(run.finished) ++ for({_ref, name, false} <- cancelling, do: name)
    cancelled = for {_ref, name, true} <- cancelling, do: name
    stopped(%{run | codemode_run: :stopped, calls: %{}}, result, finished, cancelled)
  end

  # Stopped: nothing it started is left, and it says which of its calls finished and which were cancelled.
  defp stopped(%{codemode_run: :stopped}, {done, text}, finished, cancelled),
    do: {done, text <> said(finished, cancelled)}

  # A cancelled call ends by what it calls next raising, or else is stopped by the call machine, or by the run if it
  # has to end first, which then ends its state too.
  defp awaited(ref, pid, call, until) do
    left = if until, do: max(until - System.monotonic_time(:millisecond), 0), else: :infinity

    receive do
      {^ref, _result} ->
        Process.demonitor(ref, [:flush])

      {:DOWN, ^ref, :process, _pid, _why} ->
        State.finish(call)
    after
      left ->
        Process.exit(pid, :kill)
        receive do: ({:DOWN, ^ref, :process, _pid, _why} -> State.finish(call))
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

  defp said([], []), do: ""

  # Each tool once, with how many times it was called where that's more than once.
  defp said(finished, cancelled) do
    names = fn
      [] ->
        "none"

      names ->
        Enum.map_join(Enum.frequencies(names), ", ", fn
          {name, 1} -> name
          {name, times} -> "#{name} ×#{times}"
        end)
    end

    "\n\nCalls finished: #{names.(finished)}. Cancelled, as the script stopped first: #{names.(cancelled)}."
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
        case JSON.decode!(JSON.encode!(result)) do
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

  defp stop(port) do
    case Port.info(port, :os_pid) do
      {:os_pid, pid} ->
        Port.close(port)
        System.cmd("kill", ["-9", to_string(pid)], stderr_to_stdout: true)

      nil ->
        :ok
    end
  end

  # The runner's folder, its packages installed with Bun, made whole beside it and moved in, so a folder that's
  # there is ready. Each set of its files has its own.
  defp runner(bun) do
    code = Base.encode16(:crypto.hash(:sha256, :erlang.term_to_binary(@files)), case: :lower)
    folder = Path.join([Server.home(), "codemode", binary_part(code, 0, 16)])

    if File.exists?(Path.join(folder, "node_modules/@opencode/codemode")) do
      {:ok, folder}
    else
      writing = folder <> "." <> Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)

      try do
        File.mkdir_p!(writing)
        for {name, text} <- @files, do: File.write!(Path.join(writing, name), text)

        case System.cmd(bun, ~w(install --frozen-lockfile --production), cd: writing, stderr_to_stdout: true) do
          {_, 0} ->
            # Another script may have made it meanwhile, which is as good.
            with {:error, _} <- File.rename(writing, folder), do: :ok
            {:ok, folder}

          {output, _} ->
            {:error, "Code mode's packages didn't install:\n#{String.trim(output)}"}
        end
      after
        File.rm_rf!(writing)
      end
    end
  end
end
