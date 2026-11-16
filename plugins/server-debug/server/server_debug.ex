defmodule ServerDebug do
  @moduledoc """
  Server Debug: Erlang's own ways of looking into a plugin as it runs on a live server, for whoever deploys it. Its
  processes as observer_cli lists them, its Logger output at every level, and a trace of a function as `:recon_trace`
  gives it. It's for a server that is no Erlang node (`core/server/rel/env.sh.eex`), which nothing attaches to.

  Who may: an `editor` of the plugin, as for deploying it, and only on its calls for themselves, or for a group or the
  network they own: a trace and a log show what a call was given, which is someone's. The plugin's own processes,
  kept between calls, are shown too where everyone it's installed for is that one. A plugin that says it's sensitive
  (`OneCore.Plugin`) is never looked into. The rule is kept here, on the server; what's shown is picked in the VM the
  plugin's code runs in, the server's or its own in sandbox mode, by `ServerDebug.Probe`.
  """
  @behaviour OneCore.Plugin

  alias OneCore.{Call, Hooks, Plugins, Sandbox, Server, Servers, User}
  alias OneCore.Plugins.Version
  alias ServerDebug.Probe

  @impl true
  def title, do: "Server Debug"

  @impl true
  def tools do
    common = %{
      plugin: %{type: "string", description: "The plugin, by the name you find it by: agent-memory"},
      subject: %{
        type: "string",
        description: "Whose calls: yours if left out, or a group's or the network's you own (groups/family, network)"
      },
      server: %{
        type: "string",
        description: "The server to look on, by its machine's name or its key. Left out, one that runs the plugin"
      }
    }

    takes = fn more, required ->
      %{type: "object", properties: Map.merge(common, more), required: ["plugin" | required]}
    end

    [
      %{
        name: "processes",
        description:
          "A plugin's processes on a server, as observer_cli lists them: the top by memory, reductions or message queue, with each one's current function, and the ETS tables they own. Only those running your calls, and the plugin's own where it's installed only for whoever `subject` is",
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
          )
      },
      %{
        name: "logs",
        description:
          "Follows what a plugin says to Logger, every level from debug up, as it runs your calls, for up to `minutes` or `lines`. The first call starts following; each call answers what was said since the last, waiting up to `wait` seconds for a first line. Call the plugin in between",
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
          )
      },
      %{
        name: "trace",
        description:
          "Traces a function of a plugin with :recon_trace as your calls run it: what it was called with and what it returned or raised, for up to `calls` calls or `seconds`. Call the plugin while it runs. Recon's own limit stops a trace of a function everyone calls often",
        inputs:
          takes.(
            %{
              module: %{type: "string", description: "Its module, as the plugin wrote it: Hello or Hello.Counter"},
              function: %{type: "string", description: "Its function: all of the module's if left out"},
              arity: %{type: "integer", minimum: 0, maximum: 255},
              calls: %{type: "integer", minimum: 1, maximum: 100, description: "How many calls: 10 if left out"},
              seconds: %{type: "integer", minimum: 1, maximum: 60, description: "How long: 20 if left out"}
            },
            ["module"]
          )
      }
    ]
  end

  @impl true
  def call(tool, inputs, %Call{} = call) do
    target = target!(call, inputs)
    on!(call, inputs, target)

    request =
      inputs |> Map.drop(["plugin", "subject", "server"]) |> Map.merge(%{"op" => tool, "follower" => call.actor})

    result = probed(target, request, tool == "logs")
    "#{target.name} on #{Servers.name(Server.key()) || Server.key()}, for #{target.whose}. " <> shown(tool, result)
  end

  # The plugin `plugin` as whoever `subject` is finds it, if whoever asked may look into it.
  defp target!(%Call{actor: actor} = call, %{"plugin" => name} = inputs) do
    if actor == nil or User.link?(actor), do: raise("Only a person looks into a plugin")
    whose = inputs["subject"] || actor

    # Another person's calls are never looked at, whoever owns the network: what they were given is theirs.
    group = whose == "network" or String.starts_with?(whose, "groups/")

    if not (whose == actor or (group and Call.owns?(%{call | subject: whose}))),
      do: raise("You look only at your own calls, or a group's or the network's you own, and #{whose} isn't one")

    {at, version} =
      Enum.find_value(Plugins.chain(whose), fn at ->
        with %Version{} = version <- Plugins.versions([at])[name], do: {at, version}
      end) || raise "#{whose} has no plugin named #{name}"

    if not editor?(actor, version.artifact),
      do: raise("You aren't an editor of #{name} (#{version.artifact}): only whoever may deploy it looks into it")

    %{name: name, at: at, version: version, whose: whose}
  end

  defp editor?(by, artifact) do
    String.starts_with?(artifact, by <> "/") or
      Hooks.validate("reach", %{"who" => by, "relation" => "editor", "artifact" => artifact}, {by, by}) == :ok
  end

  # The server to look on: the one asked for, or else this one if it runs the plugin, or else one that does. Another
  # runs this call instead, and it ends here with its answer.
  defp on!(call, inputs, %{name: name, at: at, version: version}) do
    servers = if version.sandbox, do: Servers.acting(at), else: Servers.running(at)

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

  defp here?(%Version{sandbox: true}, at), do: Servers.here?(at)
  defp here?(version, _at), do: not Version.described?(version)

  # What the plugin's VM answered: the server's own, or its own in sandbox mode, where only a running one is asked,
  # but by a follow, which keeps it running.
  defp probed(%{version: %Version{sandbox: true}} = target, request, start) do
    request = %{
      "source" => Probe.source(),
      "request" => request,
      "scope" => %{"subject" => target.whose, "own" => target.at == target.whose}
    }

    case Plugins.run([target.at], target.name, &Sandbox.debug(&1, &2, target.name, request, start)) do
      {:error, why} -> raise why
      result -> result
    end
  end

  defp probed(%{version: version} = target, request, _start) do
    plugin = version.plugin

    if function_exported?(plugin, :sensitive, 0) and plugin.sensitive(),
      do:
        raise("#{target.name} is sensitive: what it's given and keeps are secrets, so nothing looks into it as it runs")

    artifact = version.artifact
    # The plugin's own processes are shown where everyone it's installed for here is whoever `subject` is.
    own? = Enum.all?(Plugins.installed_for(artifact), &(&1 == target.whose))

    label = fn pid ->
      case Plugins.Process.which(pid) do
        {^artifact, module, _version} -> module
        _ -> nil
      end
    end

    Probe.run(request, %{
      subject: target.whose,
      who: artifact,
      own: if(own?, do: &(label.(&1) != nil)),
      label: label,
      module: &(Version.module(version, &1) || raise("#{target.name} has no module #{&1}")),
      calls: {OneCore.Plugins.Calls, :running, [%{who: artifact, subject: target.whose}, :_]},
      name: &Version.own_names(&1, version)
    })
  end

  defp shown("processes", %{"count" => count, "processes" => processes, "tables" => tables}) do
    columns = [
      {"PID", "pid"},
      {"NAME OR INITIAL CALL", "name"},
      {"MEMORY", "memory"},
      {"REDUCTIONS", "reductions"},
      {"MSG QUEUE", "message_queue_len"},
      {"CURRENT FUNCTION", "current_function"}
    ]

    tables_shown =
      if tables == [],
        do: "",
        else:
          "\n\nETS tables:\n" <>
            table([{"TABLE", "table"}, {"SIZE", "size"}, {"MEMORY", "memory"}, {"OWNER", "owner"}], tables)

    "#{count} processes#{if count > length(processes), do: ", the top #{length(processes)}", else: ""}:\n\n" <>
      table(columns, processes) <> tables_shown
  end

  defp shown("logs", %{"lines" => lines, "ended" => ended, "until" => until}) do
    following = if ended, do: ended, else: "Following until #{until}: call again for what's said next."
    if(lines == [], do: "Nothing was said yet. ", else: Enum.join(lines, "\n") <> "\n\n") <> following
  end

  defp shown("trace", %{"lines" => lines, "calls" => calls, "ended" => ended}) do
    if(calls == 0, do: "None of your calls ran it. ", else: Enum.join(lines, "\n") <> "\n\n") <> ended
  end

  defp table(columns, rows) do
    cells = [Enum.map(columns, &elem(&1, 0)) | for(row <- rows, do: Enum.map(columns, &to_string(row[elem(&1, 1)])))]
    widths = Enum.zip_with(cells, fn column -> column |> Enum.map(&String.length/1) |> Enum.max() end)

    Enum.map_join(cells, "\n", fn row ->
      row
      |> Enum.zip(widths)
      |> Enum.map_join("  ", fn {cell, width} -> String.pad_trailing(cell, width) end)
      |> String.trim_trailing()
    end)
  end
end
