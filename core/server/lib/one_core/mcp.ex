defmodule OneCore.Mcp do
  @moduledoc """
  Answers what a client asks, in MCP's JSON-RPC: a message in, and its answer out. So a server is an MCP server to
  whatever reaches it, and the tools it lists are One Core's own and those of the plugins installed for whoever asks
  and for the network.

  Every request is from someone and for someone, which `OneCore.User.asking/2` says, and a plugin's tool runs for
  them.

  Servers ask each other the same way, and four things only a server acting for whoever it asks for may ask: to call a
  function of a plugin's service running here (`services/call`), and to run what was passed on of a service's
  call into its plugin (`plugins/call`), of a plugin's hook (`hooks/call`) or of having a plugin's scheduled jobs
  called (`plugins/schedule`).

  What of a plugin runs in a client, its folder `client/`, is MCP resources: `ui://<plugin>/<code>/<path>`, where
  `code` names the set of files a version runs, so a file at an address never changes. What a page loads is listed,
  a plugin's apps among it as an MCP App is, and the rest of `client/` is read by name.

  Skills are served as MCP's Skills Extension says (SEP-2640), in the shapes it gives for the protocol versions a
  server speaks: `skills/list` lists them, `skills/get` gives one by its address, and each of a skill's files is
  a resource too, `skill://<path>/<file>`, which `resources/read` reads (`OneCore.Skills`).
  """
  require Logger
  alias OneCore.{Artifact, Builds, Edge, Hooks, Plugins, Server, Servers, Services, Skills, Tools, Trace, User}
  alias OneCore.Plugins.{Calls, Store, Version}

  # What a server speaks of MCP is the same in each of these.
  @versions ~w(2025-11-25 2025-06-18 2025-03-26 2024-11-05)

  @doc "The answer to one message from whoever holds the key `from`, as JSON. A notification's is empty."
  def answer(message, from) do
    case JSON.decode(message) do
      {:ok, %{"method" => method, "id" => id} = request} ->
        params = Map.get(request, "params", %{})

        try do
          Trace.follow(said(params)["traceparent"])

          with {:ok, asked, forwarded} <- User.asking(from, said(params), method == "hooks/call"),
               {:ok, result} <- result(method, params, from, asked, forwarded) do
            JSON.encode!(%{jsonrpc: "2.0", id: id, result: result})
          else
            {:error, why} -> JSON.encode!(%{jsonrpc: "2.0", id: id, error: %{code: -32000, message: why}})
            {:error, code, why} -> JSON.encode!(%{jsonrpc: "2.0", id: id, error: %{code: code, message: why}})
          end
        rescue
          error ->
            Logger.error(Exception.format(:error, error, __STACKTRACE__))
            JSON.encode!(%{jsonrpc: "2.0", id: id, error: %{code: -32603, message: Exception.message(error)}})
        end

      # A client saying an app of its is on screen, and where a server reaches it, or with no address, that none is.
      {:ok, %{"method" => "notifications/one/watch"} = said} when from != nil ->
        Server.watch(from, said["params"]["address"])
        ""

      # A client saying how long its apps took to open, and which of its calls failed before a server answered.
      {:ok, %{"method" => "notifications/one/measured", "params" => %{} = said}} when from != nil ->
        Hooks.observe("measured", Map.put(said, "device", from), {nil, "network"})
        ""

      # The relay at the network's address saying one of its servers came or went.
      {:ok, %{"method" => "notifications/one/seen"}} ->
        Servers.seen_again()
        ""

      # Another server, or a plugin's edge function, saying that what this one kept of some hooks
      # was decided from what has since changed.
      {:ok, %{"method" => "notifications/one/forget"} = said} ->
        Hooks.forget(for hook <- List.wrap(said["params"]["hooks"]), is_binary(hook), do: hook)
        ""

      # Another server saying there's something new in the bucket for some owners, or with none, this server saying
      # it can reach its network's address again.
      {:ok, %{"method" => "notifications/one/reload"} = said} ->
        case said["params"]["for"] do
          nil -> Server.reconnected()
          owners -> for owner <- List.wrap(owners), User.owner?(owner), do: Server.reload(owner)
        end

        ""

      {:ok, %{"method" => _notification}} ->
        ""

      _ ->
        JSON.encode!(%{jsonrpc: "2.0", id: nil, error: %{code: -32700, message: "That isn't a JSON-RPC request"}})
    end
  end

  # What a request says of itself.
  defp said(%{"_meta" => %{} = said}), do: said
  defp said(_params), do: %{}

  defp result("services/call", %{"plugin" => plugin, "service" => service} = params, _from, _asked, forwarded) do
    between(forwarded, fn -> Services.run(plugin, service, params["function"], params["arguments"]) end)
  end

  defp result("plugins/call", %{"installed" => at, "plugin" => plugin} = params, _from, asked, forwarded) do
    function = fn -> Calls.function(at, plugin, params["function"], params["arguments"], asked, true) end

    with {:ok, result} <- between(forwarded, function) do
      Server.changed()
      {:ok, result}
    end
  end

  defp result("plugins/schedule", %{"installed" => at, "plugin" => plugin}, _from, _asked, forwarded) do
    between(forwarded, fn -> Calls.schedule(at, plugin, true) end)
  end

  defp result("hooks/call", %{"installed" => at, "plugin" => plugin, "hook" => hook} = params, _from, asked, forwarded) do
    between(forwarded, fn -> Calls.hook(at, plugin, hook, params["about"], asked, true) end)
  end

  defp result("tools/call", %{"name" => name} = params, _from, asked, forwarded) do
    case call(name, Map.get(params, "arguments", %{}), asked, forwarded) do
      {:ok, answer} -> changed(name, %{content: content(name, answer)}, asked)
      {:error, why} -> {:ok, %{content: [%{type: "text", text: why}], isError: true}}
      # Another server ran it, and told its own clients.
      {:forwarded, %{"isError" => true} = result} -> {:ok, result}
      {:forwarded, result} -> changed(name, result, asked)
      :forwarding_loop -> {:error, Server.forwarding_loop(), "This server would pass #{name} on in turn"}
    end
  end

  defp result("initialize", params, _from, {by, _who}, _forwarded) do
    asked = params["protocolVersion"]

    {:ok,
     %{
       protocolVersion: if(asked in @versions, do: asked, else: hd(@versions)),
       capabilities: %{tools: %{}, resources: %{}, extensions: %{"io.modelcontextprotocol/skills" => %{}}},
       serverInfo: %{name: "one", version: Builds.build()},
       instructions: instructions(holder?(by))
     }}
  end

  defp result("ping", _params, _from, _asked, _forwarded), do: {:ok, %{}}

  # A user's device asks for its token to the relay at the network's address, and for the next before that one
  # ends, and is told whose it is: the device a run of it proved here, or the edge did there. It says which
  # build it is as it asks.
  defp result("one/token", params, from, {"users/" <> _ = by, _for}, _forwarded) do
    device = User.device(from, said(params))

    if not User.link?(by) and is_binary(device) and device =~ ~r/^[0-9a-f]{64}$/ do
      Builds.seen(by, device, params["build"])
      {:ok, %{token: Edge.token(device), who: by}}
    else
      {:error, "A token is for a user's device, by its key"}
    end
  end

  # A sharing link's holder is told of the tools they may call, and of nothing else that's in the network.
  defp result("tools/list", _params, _from, {by, who}, _forwarded) do
    open = fn tools -> if holder?(by), do: Enum.filter(tools, &(&1[:by_link] == true)), else: tools end

    listed = fn name, tool ->
      %{
        name: name,
        description: tool.description,
        inputSchema: tool.inputs,
        annotations: %{readOnlyHint: tool[:read_only] == true}
      }
    end

    own = for tool <- open.(Tools.tools()), do: listed.(tool.name, tool)

    plugins =
      for {plugin, version} <- Enum.sort(Plugins.running(who)),
          tool <- open.(version.tools),
          do: listed.("#{plugin}_#{tool.name}", tool)

    {:ok, %{tools: own ++ plugins}}
  end

  defp result("skills/list", _params, _from, asked, _forwarded),
    do: {:ok, %{resultType: "complete", skills: skills(asked)}}

  defp result("skills/get", params, _from, asked, _forwarded) do
    case Enum.find(skills(asked), &(&1.uri == params["uri"])) do
      nil -> {:error, -32602, "There's no skill at #{params["uri"]}. skills/list says what there is."}
      skill -> {:ok, %{resultType: "complete", skill: skill}}
    end
  end

  defp result("resources/" <> _ = method, params, _from, {by, who}, _forwarded) do
    if holder?(by),
      do: {:error, -32002, "A sharing link's holder reads nothing of a network but what a tool gives them"},
      else: resources(method, params, who)
  end

  defp result(method, _params, _from, _asked, _forwarded),
    do: {:error, -32601, "This server has no method #{method}"}

  # A skill is listed here too, for an agent whose client lists resources and not yet skills.
  defp resources("resources/list", _params, who) do
    skills =
      for %{uri: uri, frontmatter: %{"name" => name, "description" => description}} <- Skills.list(who),
          do: %{uri: uri, name: name, description: description, mimeType: "text/markdown"}

    resources =
      for {plugin, version} <- Enum.sort(Plugins.running(who)), path <- Version.files(version) do
        resource = %{uri: "ui://#{plugin}/#{version.code}/#{path}", name: path, mimeType: Version.kind(path)}

        case Enum.find(version.apps, &(&1.page == path)) do
          nil ->
            %{resource | mimeType: String.replace_suffix(resource.mimeType, ";profile=mcp-app", "")}

          # A part of Settings says so, so a client that opens a plugin's app opens the one that isn't.
          %{settings: true} = app ->
            Map.merge(resource, %{name: app.name, icons: [%{src: app.icon}], _meta: %{settings: true}})

          app ->
            Map.merge(resource, %{name: app.name, icons: [%{src: app.icon}]})
        end
      end

    {:ok, %{resources: skills ++ resources}}
  end

  defp resources("resources/read", %{"uri" => "skill://" <> _ = uri}, who) do
    case Skills.text(who, uri) do
      {:ok, text} -> {:ok, %{contents: [%{uri: uri, mimeType: "text/markdown", text: text}]}}
      :absent -> {:error, -32602, "There's no skill at #{uri}. skills/list says what there is."}
    end
  end

  defp resources("resources/read", %{"uri" => uri}, who) do
    with ["ui:", "", plugin, code, path] <- String.split(uri, "/", parts: 5),
         true <- Version.client?(path),
         %{^plugin => version} <- Plugins.running(who),
         {:ok, data} <- Store.file(version.artifact, code, path) do
      # A file that isn't text, such as a picture or a font, is base64 as MCP's `blob`.
      content = if String.valid?(data), do: %{text: data}, else: %{blob: Base.encode64(data)}
      {:ok, %{contents: [Map.merge(%{uri: uri, mimeType: Version.kind(path)}, content)]}}
    else
      _ -> {:error, -32002, "There's nothing at #{uri}"}
    end
  end

  defp resources(method, _params, _who), do: {:error, -32601, "This server has no method #{method}"}

  # What one server asks of another is run only for a server that says who it asks for, which only one acting
  # for them is believed about. This server runs it if it acts for whoever it's for too.
  defp between(forwarded, run) when forwarded in [true, :to] do
    case run.() do
      {:ok, answer} -> {:ok, %{answer: answer}}
      {:error, why} -> {:error, -32000, why}
      {:forwarded, result} -> {:ok, result}
      :forwarding_loop -> {:error, Server.forwarding_loop(), "This server would pass that on in turn"}
    end
  end

  defp between(_forwarded, _run), do: {:error, -32000, "Only a server acting for whoever it asks for may ask that"}

  # A tool is one of One Core's own, or a plugin's by its name and the plugin's.
  defp call(name, inputs, asked, forwarded) do
    with :none <- Tools.call(name, inputs, asked, forwarded),
         [plugin, tool] <- String.split(name, "_", parts: 2),
         {:ok, answer} <- Calls.call(plugin, tool, inputs, asked, forwarded) do
      case answer do
        text when is_binary(text) -> {:ok, text}
        {:blob, _kind, _data} -> {:ok, answer}
        {:file, _kind, %Artifact{}, _name} -> {:ok, answer}
        _ -> {:ok, JSON.encode!(answer)}
      end
    else
      [_] -> {:error, "#{name} isn't a tool. tools/list says what there is."}
      other -> other
    end
  end

  # An answer as MCP's content. A picture is an image, which an agent that sees pictures sees, and any other file
  # is a resource inside the answer, unless it's large.
  defp content(_name, {:blob, "image/" <> _ = kind, data}),
    do: [%{type: "image", mimeType: kind, data: Base.encode64(data)}]

  defp content(name, {:blob, kind, data}),
    do: [%{type: "resource", resource: %{uri: "one://#{name}", mimeType: kind, blob: Base.encode64(data)}}]

  # A large file is a short-lived link to its blob in the bucket, under the blob's key, which never names another.
  defp content(_name, {:file, kind, %Artifact{prefix: prefix} = artifact, blob}) do
    [%{type: "resource_link", uri: Artifact.link(artifact, blob, :read).url, name: prefix <> blob, mimeType: kind}]
  end

  defp content(_name, text), do: [%{type: "text", text: text}]

  # After a tool that doesn't only read, every client connected here is told that something changed.
  defp changed(name, result, {_by, who}) do
    if not reads?(name, who), do: Server.changed()
    {:ok, result}
  end

  # Whether calling a tool only reads: one of One Core's own that says so, or a plugin's.
  defp reads?(name, who) do
    with [plugin, tool] <- String.split(name, "_", parts: 2),
         %{^plugin => version} <- Plugins.running(who) do
      Enum.any?(version.tools, &(&1.name == tool and &1[:read_only] == true))
    else
      _ -> Enum.any?(Tools.tools(), &(&1.name == name and &1[:read_only] == true))
    end
  end

  defp holder?(by), do: by != nil and User.link?(by)

  # A sharing link's holder sees no skill.
  defp skills({by, who}), do: if(holder?(by), do: [], else: Skills.list(who))

  # What an agent is told when it connects. Skills say the rest, so it's pointed at them: not every client lists
  # skills yet, and any reads one that it has the address of.
  defp instructions(holder) do
    network = """
    This is a One App network: what one user, family or workplace keeps and makes, which you use as whoever you act \
    for. Its tools are its plugins', each named <plugin>_<tool>, and One Core's own, which add and remove plugins, \
    servers and services.\
    """

    skills = """
    A plugin's skill says when and how to use its tools, and the skill building-plugins how to make a plugin. \
    skills/list and resources/list list them, and resources/read reads one: a plugin's are under \
    skill://<plugin>/, and building-plugins is skill://building-plugins/SKILL.md.\
    """

    if holder, do: network, else: network <> "\n" <> skills
  end
end
