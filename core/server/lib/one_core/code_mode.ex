defmodule OneCore.CodeMode do
  @moduledoc """
  Code mode's `execute` and `call` over the web, as Executor has them. `execute` runs a short script that calls tools,
  by OpenCode's interpreter under this server's Bun (`core/codemode/run.js`), where nothing but `tools` reaches the
  world. Each call it makes comes back here and is run as whoever asked, exactly as their MCP call would be, its
  inputs checked against what the tool takes. `call` makes one call or several at once with no script. The CLI does
  both on its own device (`clients/cli/src/codemode.rs`), so that what a device fills in of a tool's inputs is filled
  in there.
  """
  alias OneCore.{App, Server}

  @runner Path.expand("../../../codemode", __DIR__)
  @external_resource Path.join(@runner, "package.json")
  @external_resource Path.join(@runner, "bun.lock")
  @external_resource Path.join(@runner, "run.js")
  @files Map.new(~w(package.json bun.lock run.js), &{&1, File.read!(Path.join(@runner, &1))})

  # The script's own limits are the runner's: ten minutes, and a minute for its last call to come back.
  @stopped :timer.minutes(11)

  @doc "`execute` and `call`, as MCP lists them."
  def tools, do: [execute(), call()]

  defp execute do
    %{
      name: "execute",
      description: """
      Runs a short script that calls tools, and gives back what it returns. It's JavaScript without imports, timers \
      or network: call a tool as `await tools.<plugin>.<tool>({ ... })`, or One Core's own as `await \
      tools.<tool>({ ... })` (search gives each exact call, and `search({ query })` in the script finds them too), use \
      plain JavaScript between calls, and `return` what you want back; what it logs comes back too. Use it where one \
      call's answer feeds another, or to cut a large answer down; for one call or a few that don't depend on each \
      other, use call. Each call runs as you, as any tool does, so what sends or can't be undone waits for the \
      person's yes, as always. Up to 100 calls, 10 minutes and 60 KB back.\
      """,
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
      undone waits for the person's yes, as always.\
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
      timeout: @stopped,
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
  Runs `code` with `tools`, each `%{name, plugin, description, input, output}`, and `namespaces`, what each plugin is for by
  its name. `call` runs one: given its name and inputs, it gives MCP's result of calling it. Gives `{:ok, text}` or
  `{:error, text}`: what it returned or what went wrong, with what it logged.
  """
  def run(code, tools, namespaces, call) do
    with {:ok, bun} <- App.bun(),
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

      try do
        listen(port, call, %{}, "", [], System.monotonic_time(:millisecond) + @stopped)
      after
        stop(port)
      end
    end
  end

  # Each line from the runner is a call to run, which runs beside the others, or what the script came to.
  defp listen(port, call, running, part, output, until) do
    receive do
      {^port, {:data, {:noeol, more}}} ->
        listen(port, call, running, part <> more, output, until)

      {^port, {:data, {:eol, more}}} ->
        case JSON.decode(part <> more) do
          {:ok, %{"call" => id, "name" => name} = request} ->
            task = Task.async(fn -> called(call, name, request["input"]) end)
            listen(port, call, Map.put(running, task.ref, id), "", output, until)

          {:ok, %{"done" => %{"ok" => ok, "text" => text}}} ->
            {if(ok, do: :ok, else: :error), text}

          _ ->
            listen(port, call, running, "", [part <> more | output], until)
        end

      {ref, result} when is_map_key(running, ref) ->
        Process.demonitor(ref, [:flush])
        Port.command(port, JSON.encode!(Map.put(result, :reply, running[ref])) <> "\n")
        listen(port, call, Map.delete(running, ref), part, output, until)

      {^port, {:exit_status, _}} ->
        {:error, "The script stopped without an answer: " <> (output |> Enum.reverse() |> Enum.join("\n"))}
    after
      max(until - System.monotonic_time(:millisecond), 0) ->
        {:error, "The script ran past its time and was stopped."}
    end
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
