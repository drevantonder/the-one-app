defmodule ServerDebug do
  @moduledoc """
  Server Debug: Erlang's own ways of looking into an app as it runs on a live server, for whoever deploys it. Its
  processes as observer_cli lists them, its Logger output at every level, and a trace of a function as `:recon_trace`
  gives it. It's for a server that is no Erlang node (`core/server/rel/env.sh.eex`), which nothing attaches to.

  Who may: an `editor` of the app, as for deploying it, and only on its calls for themselves, or for a group or the
  network they own: a trace and a log show what a call was given, which is someone's. The app's own processes,
  kept between calls, are shown too where everyone it's installed for is that one. An app that says it's sensitive
  (`OneCore.App`) is never looked into. The rule is kept here, on the server; what's shown is picked in the VM the
  app's code runs in, the server's or, for an untrusted app, its sandbox's, by `ServerDebug.Probe`.
  """
  @behaviour OneCore.App

  alias OneCore.{Call, Hooks, Apps, Sandbox, Server, Servers, User}
  alias OneCore.Apps.Version
  alias ServerDebug.Probe

  @impl true
  def icon, do: "icon.svg"

  @impl true
  def requires, do: %{"core" => ">= 0.32.196"}

  @impl true
  def tools do
    common = %{
      app: %{type: "string", description: "The app, by the name you find it by: agent-memory"},
      subject: %{
        type: "string",
        description: "Whose calls: yours if left out, or a group's or the network's you own (groups/family, network)"
      },
      server: %{
        type: "string",
        description: "The server to look on, by its machine's name or its key. Left out, one that runs the app"
      }
    }

    takes = fn more, required ->
      %{type: "object", properties: Map.merge(common, more), required: ["app" | required]}
    end

    # Each answer says which app, which server and whose calls, with what it found.
    gives = fn more, required ->
      looked = %{
        app: %{type: "string"},
        server: %{type: "string", description: "The server it looked on, by its machine's name or its key"},
        for: %{type: "string", description: "Whose calls"}
      }

      %{type: "object", properties: Map.merge(looked, more), required: ["app", "server", "for" | required]}
    end

    [
      %{
        name: "processes",
        read_only: true,
        description:
          "An app's processes on a server, as observer_cli lists them: the top by memory, reductions or message queue, with each one's current function, and the ETS tables they own. Only those running your calls, and the app's own where it's installed only for whoever `subject` is",
        inputs:
          takes.(
            %{
              sort: %{
                type: "string",
                enum: ["memory", "reductions", "message_queue_len"],
                description: "memory if left out"
              },
              top: %{type: "integer", minimum: 1, maximum: 100, description: "How many: 20 if left out"}
            },
            []
          ),
        outputs: gives.(
          %{
            count: %{type: "integer", description: "How many processes it has"},
            processes: %{
              type: "array",
              description: "The top ones",
              items: %{
                type: "object",
                properties: %{
                  pid: %{type: "string"},
                  name: %{type: ["string", "null"], description: "Its registered name or initial call"},
                  memory: %{type: "integer", description: "In bytes"},
                  reductions: %{type: "integer"},
                  message_queue_len: %{type: "integer"},
                  current_function: %{type: ["string", "null"]}
                },
                required: ["pid", "memory", "reductions", "message_queue_len"]
              }
            },
            tables: %{
              type: "array",
              description: "The ETS tables they own",
              items: %{
                type: "object",
                properties: %{
                  table: %{type: "string"},
                  owner: %{type: "string", description: "Its owner's pid"},
                  size: %{type: "integer", description: "How many objects it holds"},
                  memory: %{type: "integer", description: "In bytes"}
                },
                required: ["table", "owner", "size", "memory"]
              }
            }
          },
          ["count", "processes", "tables"]
        )
      },
      %{
        name: "logs",
        read_only: true,
        description:
          "Follows what an app says to Logger, every level from debug up, as it runs your calls, for up to `minutes` or `lines`. The first call starts following; each call answers what was said since the last, waiting up to `wait` seconds for a first line. Call the app in between",
        inputs:
          takes.(
            %{
              minutes: %{type: "integer", minimum: 1, maximum: 60, description: "How long to follow: 10 if left out"},
              lines: %{
                type: "integer",
                minimum: 1,
                maximum: 5000,
                description: "The most lines to hear: 1000 if left out"
              },
              wait: %{
                type: "integer",
                minimum: 0,
                maximum: 25,
                description: "Seconds to wait for a line: 10 if left out"
              },
              stop: %{type: "boolean", description: "Stops following"}
            },
            []
          ),
        outputs: gives.(
          %{
            lines: %{type: "array", items: %{type: "string"}, description: "What was said since the last call"},
            ended: %{type: ["string", "null"], description: "Why it stopped following, once it has"},
            until: %{type: ["string", "null"], description: "When it stops following, in ISO 8601, while it follows"}
          },
          ["lines", "ended"]
        )
      },
      %{
        name: "trace",
        read_only: true,
        description:
          "Traces a function of an app with :recon_trace as your calls run it: what it was called with and what it returned or raised, for up to `calls` calls or `seconds`. Call the app while it runs. Recon's own limit stops a trace of a function everyone calls often",
        inputs:
          takes.(
            %{
              module: %{type: "string", description: "Its module, as the app wrote it: Hello or Hello.Counter"},
              function: %{type: "string", description: "Its function: all of the module's if left out"},
              arity: %{type: "integer", minimum: 0, maximum: 255},
              calls: %{type: "integer", minimum: 1, maximum: 100, description: "How many calls: 10 if left out"},
              seconds: %{type: "integer", minimum: 1, maximum: 60, description: "How long: 20 if left out"}
            },
            ["module"]
          ),
        outputs: gives.(
          %{
            lines: %{type: "array", items: %{type: "string"}, description: "Each call and what it returned or raised"},
            calls: %{type: "integer", description: "How many of your calls ran it"},
            ended: %{type: ["string", "null"], description: "Why the trace ended"}
          },
          ["lines", "calls", "ended"]
        )
      }
    ]
  end

  @impl true
  def call(tool, inputs, %Call{} = call) do
    target = target!(call, inputs)
    on!(call, inputs, target)

    request =
      inputs |> Map.drop(["app", "subject", "server"]) |> Map.merge(%{"op" => tool, "follower" => call.actor})

    result = probed(target, request, tool == "logs")
    Map.merge(result, %{"app" => target.name, "server" => Servers.name(Server.key()) || Server.key(), "for" => target.whose})
  end

  # The app `app` as whoever `subject` is finds it, if whoever asked may look into it.
  defp target!(%Call{actor: actor} = call, %{"app" => name} = inputs) do
    if actor == nil or User.link?(actor), do: raise("Only a person looks into an app")
    whose = inputs["subject"] || actor

    # Another person's calls are never looked at, whoever owns the network: what they were given is theirs.
    group = whose == "network" or String.starts_with?(whose, "groups/")

    if not (whose == actor or (group and Call.owns?(%{call | subject: whose}))),
      do: raise("You look only at your own calls, or a group's or the network's you own, and #{whose} isn't one")

    {at, version} =
      Enum.find_value(Apps.chain(whose), fn at ->
        with %Version{} = version <- Apps.versions([at])[name], do: {at, version}
      end) || raise "#{whose} has no app named #{name}"

    if not editor?(actor, version.artifact),
      do: raise("You aren't an editor of #{name} (#{version.artifact}): only whoever may deploy it looks into it")

    %{name: name, at: at, version: version, whose: whose}
  end

  defp editor?(by, artifact) do
    String.starts_with?(artifact, by <> "/") or
      Hooks.validate("reach", %{"who" => by, "relation" => "editor", "artifact" => artifact}, {by, by}) == :ok
  end

  # The server to look on: the one asked for, or else this one if it runs the app, or else one that does. Another
  # runs this call instead, and it ends here with its answer.
  defp on!(call, inputs, %{name: name, at: at, version: version}) do
    servers = if version.trusted, do: Apps.servers(at), else: Servers.covering(at)

    case inputs["server"] do
      nil ->
        if not here?(version, at) do
          for server <- servers -- [Server.key()], do: Call.forward_to(call, server)
          raise "None of the servers that run #{name} answered"
        end

      wanted ->
        server =
          Enum.find(servers, &(&1 == wanted or Servers.name(&1) == wanted)) ||
            raise "#{wanted} isn't a server that runs #{name}. These are: " <>
                    Enum.map_join(servers, ", ", &(Servers.name(&1) || &1))

        if server != Server.key() do
          Call.forward_to(call, server)
          raise "#{wanted} didn't answer"
        end
    end
  end

  defp here?(%Version{trusted: false}, at), do: Servers.here?(at)
  defp here?(version, _at), do: not Version.described?(version)

  # What the app's VM answered: the server's own, or an untrusted app's sandbox's, where only a running one is
  # asked, but by a follow, which keeps it running.
  defp probed(%{version: %Version{trusted: false}} = target, request, start) do
    request = %{
      "source" => Probe.source(),
      "request" => request,
      "scope" => %{"subject" => target.whose, "own" => target.at == target.whose}
    }

    case Apps.run([target.at], target.name, &Sandbox.debug(&1, &2, target.name, request, start)) do
      {:error, why} -> raise why
      result -> result
    end
  end

  defp probed(%{version: version} = target, request, _start) do
    app = version.app

    if function_exported?(app, :sensitive, 0) and app.sensitive(),
      do:
        raise("#{target.name} is sensitive: what it's given and keeps are secrets, so nothing looks into it as it runs")

    artifact = version.artifact
    # The app's own processes are shown where everyone it's installed for here is whoever `subject` is.
    own? = Enum.all?(Apps.installed_for(artifact), &(&1 == target.whose))

    label = fn pid ->
      case Apps.Process.which(pid) do
        {^artifact, module, _version} -> module
        _ -> nil
      end
    end

    Probe.run(request, %{
      subject: target.whose,
      app_artifact: artifact,
      own: if(own?, do: &(label.(&1) != nil)),
      label: label,
      module: &(Version.module(version, &1) || raise("#{target.name} has no module #{&1}")),
      calls: {OneCore.Apps.Calls, :running, [%{app_artifact: artifact, subject: target.whose}, :_]},
      name: &Version.own_names(&1, version)
    })
  end
end
