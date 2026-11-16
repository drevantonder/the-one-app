defmodule OneCore.CodeMode do
  @moduledoc """
  MCP's `execute` over the web: a short script that calls tools, run by OpenCode's interpreter under this server's
  Bun (`core/codemode/run.js`), where nothing but `tools` reaches the world. Each call it makes comes back here and is
  run as whoever asked, exactly as their MCP call would be, its inputs checked against what the tool takes. The CLI
  runs the same runner on its own device (`clients/cli/src/codemode.rs`), so that what a device fills in of a tool's
  inputs is filled in there.
  """
  alias OneCore.{App, Server}

  @runner Path.expand("../../../codemode", __DIR__)
  @external_resource Path.join(@runner, "package.json")
  @external_resource Path.join(@runner, "bun.lock")
  @external_resource Path.join(@runner, "run.js")
  @files Map.new(~w(package.json bun.lock run.js), &{&1, File.read!(Path.join(@runner, &1))})

  # The script's own limits are the runner's: ten minutes, and a minute for its last call to come back.
  @stopped :timer.minutes(11)

  @doc "The tool, as MCP lists it."
  def tool do
    %{
      name: "execute",
      description: """
      Runs a short script that calls tools, and gives back what it returns. It's JavaScript without imports, timers \
      or network: call a tool as `await tools.<plugin>.<tool>({ ... })`, or One Core's own as `await \
      tools.<tool>({ ... })` (search gives each exact call), use plain JavaScript between calls, and `return` what you \
      want back; what it logs comes back too. Each call runs as you, as any tool does, so what sends or can't be undone \
      waits for the person's yes, as always. Up to 100 calls, 10 minutes and 60 KB back.\
      """,
      inputSchema: %{
        type: "object",
        properties: %{code: %{type: "string", description: "The script"}},
        required: ["code"]
      },
      annotations: %{readOnlyHint: false}
    }
  end

  @doc """
  Runs `code` with `tools`, each `%{name, plugin, description, input}`. `call` runs one: given its name and inputs, it gives MCP's result of calling it. Gives `{:ok, text}` or
  `{:error, text}`: what it returned or what went wrong, with what it logged.
  """
  def run(code, tools, call) do
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

      Port.command(port, JSON.encode!(%{code: code, tools: tools}) <> "\n")

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

      {ref, answer} when is_map_key(running, ref) ->
        Process.demonitor(ref, [:flush])
        Port.command(port, JSON.encode!(Map.put(answer, :reply, running[ref])) <> "\n")
        listen(port, call, Map.delete(running, ref), part, output, until)

      {^port, {:exit_status, _}} ->
        {:error, "The script stopped without an answer: " <> (output |> Enum.reverse() |> Enum.join("\n"))}
    after
      max(until - System.monotonic_time(:millisecond), 0) ->
        {:error, "The script ran past its time and was stopped."}
    end
  end

  # One call, as MCP answered it: text that's JSON is that, other text is a string, and anything else is MCP's
  # content as it is. An error is what the script's call throws.
  defp called(call, name, input) do
    case call.(name, input || %{}) do
      {:ok, %{isError: true, content: content}} -> %{error: texts(content)}
      {:ok, %{"isError" => true, "content" => content}} -> %{error: texts(content)}
      {:ok, %{content: content}} -> %{ok: value(content)}
      {:ok, %{"content" => content}} -> %{ok: value(content)}
      {:error, _code, why} -> %{error: why}
      {:error, why} -> %{error: why}
    end
  rescue
    error -> %{error: Exception.message(error)}
  catch
    :exit, why -> %{error: Exception.format_exit(why)}
  end

  defp value(content) do
    content = JSON.decode!(JSON.encode!(content))

    if Enum.all?(content, &(&1["type"] == "text")) do
      text = texts(content)

      case JSON.decode(text) do
        {:ok, value} -> value
        _ -> text
      end
    else
      content
    end
  end

  defp texts(content), do: content |> JSON.encode!() |> JSON.decode!() |> Enum.map_join("\n", & &1["text"])

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
