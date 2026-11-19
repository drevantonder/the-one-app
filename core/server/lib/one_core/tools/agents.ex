defmodule OneCore.Tools.Agents do
  @moduledoc "One Core's own tools an agent finds and runs the others with: `skills`, `search`, and code mode's `execute` and `call`, as `OneCore.Tools` calls them. "
  alias OneCore.{CodeMode, Apps, Search, Skills}

  alias OneCore.Apps.Deploy

  # One of code mode's calls, as `call` takes it.
  @code_call %{
    type: "object",
    properties: %{
      path: %{type: "string", description: "The tool's path, exactly as search gives it: tools.gmail.search"},
      input: %{type: "object", description: "Its inputs, as its signature says"}
    },
    required: ["path"]
  }

  @tools [
    %{
      listed: :always,
      name: "skills",
      description: """
      Lists the skills you have and reads one. A skill says when and how to use an app's tools, and One Core's own \
      say how to make an app (building-apps), install one (installing-apps) and write its app (ui). {} lists \
      every skill, each with its app, name, what it's for and its other files; { app: "gmail" } lists one \
      app's; { app: "gmail", name: "gmail" } reads that skill, and { name: "building-apps" } one of One \
      Core's own; add file to read one of its other files.\
      """,
      inputs: %{
        type: "object",
        properties: %{
          app: %{type: "string", description: "The app the skill is in, by the name its tools start with"},
          name: %{
            type: "string",
            description: "The skill's name, as its list gives it: an app's own is named as the app"
          },
          file: %{type: "string", description: "One of the skill's other files, by its path in the skill's folder"}
        },
        dependentRequired: %{file: ["name"]}
      },
      outputs: %{
        type: "object",
        properties: %{
          skills: %{
            type: "array",
            description: "With no name: the skills, each with its app, name, what it's for and its other files",
            items: %{
              type: "object",
              properties: %{
                app: %{type: "string", description: "The app it's in, or none for One Core's own"},
                name: %{type: "string"},
                description: %{type: "string"},
                files: %{type: "array", items: %{type: "string"}, description: "Its other files, by their paths"}
              },
              required: ["name", "description"]
            }
          },
          text: %{type: "string", description: "With a name: the skill's SKILL.md, or the file asked for"}
        }
      },
      read_only: true
    },
    %{
      listed: :code,
      name: "search",
      description: """
      Finds the tools for a task across every app: give a few words, like "unread email" or "add an event". \
      Answers the best matches, each with its exact call and what it does, then each one's app's skill, and \
      the apps you could install that do it. Search before guessing, to learn an app, and before saying you \
      can't.\
      """,
      inputs: %{
        type: "object",
        properties: %{
          query: %{type: "string", description: "A few words for the task, or an app's name"},
          limit: %{type: "integer", description: "How many tools at most, 8 if left out"}
        },
        required: ["query"]
      },
      outputs: %{
        type: "object",
        properties: %{text: %{type: "string", description: "The matches, their skills and what you could install"}},
        required: ["text"]
      },
      read_only: true
    },
    %{
      listed: :code,
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
      inputs: %{
        type: "object",
        properties: %{code: %{type: "string", description: "The script"}},
        required: ["code"]
      },
      outputs: %{
        type: "object",
        properties: %{text: %{type: "string", description: "What it returned, or what went wrong, with what it logged"}},
        required: ["text"]
      },
      read_only: false
    },
    %{
      listed: :code,
      name: "call",
      description: """
      Calls tools without a script: give one { path, input }, or several in a list, which run at once. Each path is \
      exactly as search gives it, like tools.gmail.search. Answers each in the same order, with ok and its value or \
      its error; a picture comes back as one. Use execute instead where one call's answer feeds another, or to cut a \
      large answer down before it reaches you. Each call runs as you, as any tool does, so what sends or can't be \
      undone waits for the person's yes, as always. Never put a password, key or token in an input: for one the \
      person pasted or must give, call vault.request, which has them type it on their own device.\
      """,
      inputs: %{
        type: "object",
        properties: %{calls: %{anyOf: [@code_call, %{type: "array", items: @code_call, minItems: 1}]}},
        required: ["calls"]
      },
      outputs: %{
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
      read_only: false
    }
  ]

  # What each takes and gives is read once here, as the schemas its inputs and answers are checked against on every
  # call.
  @taking Enum.map(@tools, &OneCore.Apps.Version.schemas/1)

  @doc "Its tools, each as an app says one of its own."
  def tools, do: @taking

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
