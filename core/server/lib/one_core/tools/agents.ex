defmodule OneCore.Tools.Agents do
  @moduledoc "One Core's own tools an agent finds and runs the others with: `skills`, `search`, and code mode's `execute` and `call`, as `OneCore.Tools` calls them, with their definitions in `OneCore.Tools.Definitions`."
  alias OneCore.{CodeMode, Apps, Search, Skills}
  alias OneCore.Apps.Deploy

  @doc "Runs its tool `name` with `inputs`, as `OneCore.Tools.call/5` hands it on."
  def run("skills", inputs, {_actor, subject}, _forward, _request) do
    with {:error, why} <- Skills.run(inputs, subject), do: raise(why), else: ({:ok, read} -> read)
  end

  # What anyone has deployed is read from the bucket, meanwhile.
  def run("search", %{"query" => query} = inputs, {_actor, subject}, _forward, request) do
    deployed = Task.async(&Deploy.deployed/0)
    tools = request.tools.()

    %{
      text:
        Search.run(
          query,
          Map.get(inputs, "limit", 8),
          tools,
          Apps.running(subject),
          Task.await(deployed, 30_000),
          request.mode
        )
    }
  end

  # A script's calls are each answered here as that call of the tool would be, for whoever asked, and so are
  # `call`'s. An app's tools in a script are in a namespace of its name, which says what the app is for.
  def run("execute", %{"code" => code}, {_actor, subject}, _forward, request) do
    tools =
      for tool <- request.tools.(),
          do: %{
            name: tool.name,
            app: tool[:_meta][:app],
            description: tool.description,
            input: tool.inputSchema,
            output: tool[:outputSchema]
          }

    namespaces = Map.new(Apps.running(subject), fn {name, version} -> {name, Search.described(version)} end)

    case CodeMode.run(code, tools, namespaces, request.call) do
      {:ok, text} -> %{text: text}
      {:error, text} -> raise text
    end
  end

  def run("call", %{"calls" => calls}, _principal, _forward, request) do
    # A path as search gives it, or without its `tools.`, as a script's `search()` gives it.
    named =
      Map.new(
        for tool <- request.tools.(),
            dotted = CodeMode.path(tool),
            path <- [Search.path(dotted), dotted],
            do: {path, tool.name}
      )

    CodeMode.calls(calls, named, request.call)
  end
end
