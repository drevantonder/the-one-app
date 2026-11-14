defmodule OneCore.Mcp do
  @moduledoc """
  Answers what a client asks, in MCP's JSON-RPC: a message in, and its answer out. So a server is an MCP server to
  whatever reaches it, and the tools it lists are One Core's own and those of the plugins added for whoever asks
  and for the network.

  Every request is from someone and for someone, which `OneCore.User.asking/2` says, and a plugin's tool runs for
  them.

  Servers ask each other the same way, and four things only a server acting for whoever it asks for may ask: to call a
  function of a plugin's service running here (`services/call`), and to run what was passed on of a service's
  call into its plugin (`plugins/call`), of a plugin's hook (`hooks/call`) or of having a plugin's scheduled jobs
  called (`plugins/schedule`).

  The parts of a plugin that run in a client are MCP resources: `ui://<plugin>/<code>/<path>`, where `code` names
  the set of files a version runs, so a file at an address never changes. A plugin's app is one of them, as an MCP
  App is.
  """
  require Logger
  alias OneCore.{Artifact, Plugin, Plugins, Server, Services, Tools, User}
  alias OneCore.Plugins.Version

  # Tools are all a server speaks of MCP, and they are the same in each of these.
  @versions ~w(2025-11-25 2025-06-18 2025-03-26 2024-11-05)

  @doc "The answer to one message from whoever holds the key `from`, as JSON. A notification's is empty."
  def answer(message, from) do
    case JSON.decode(message) do
      {:ok, %{"method" => method, "id" => id} = request} ->
        params = Map.get(request, "params", %{})

        try do
          with {:ok, asked, passed_on} <- User.asking(from, said(params), method == "hooks/call"),
               {:ok, result} <- result(method, params, from, asked, passed_on) do
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

      # Another server saying there's something new in the bucket.
      {:ok, %{"method" => "notifications/one/look"} = said} ->
        Server.look(for(owner <- List.wrap(said["params"]["for"]), User.owner?(owner), do: owner))
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

  defp result("services/call", %{"plugin" => plugin, "service" => service} = params, _from, _asked, passed_on) do
    between(passed_on, fn -> Services.run(plugin, service, params["function"], params["arguments"]) end)
  end

  defp result("plugins/call", %{"added" => at, "plugin" => plugin} = params, _from, asked, passed_on) do
    function = fn -> Plugins.function(at, plugin, params["function"], params["arguments"], asked, true) end

    with {:ok, result} <- between(passed_on, function) do
      Server.changed()
      {:ok, result}
    end
  end

  defp result("plugins/schedule", %{"added" => at, "plugin" => plugin}, _from, _asked, passed_on) do
    between(passed_on, fn -> Plugins.schedule(at, plugin, true) end)
  end

  defp result("hooks/call", %{"added" => at, "plugin" => plugin, "hook" => hook} = params, _from, asked, passed_on) do
    between(passed_on, fn -> Plugins.hook(at, plugin, hook, params["about"], asked, true) end)
  end

  defp result("tools/call", %{"name" => name} = params, _from, asked, passed_on) do
    case call(name, Map.get(params, "arguments", %{}), asked, passed_on) do
      {:ok, answer} -> changed(name, %{content: content(name, answer)}, asked)
      {:error, why} -> {:ok, %{content: [%{type: "text", text: why}], isError: true}}
      # Another server ran it, and told its own clients.
      {:passed_on, %{"isError" => true} = result} -> {:ok, result}
      {:passed_on, result} -> changed(name, result, asked)
      :passed_back -> {:error, Server.passed_back(), "This server would pass #{name} on in turn"}
    end
  end

  defp result("initialize", params, _from, {_by, who}, _passed_on) do
    asked = params["protocolVersion"]

    {:ok,
     %{
       protocolVersion: if(asked in @versions, do: asked, else: hd(@versions)),
       capabilities: %{tools: %{}, resources: %{}},
       serverInfo: %{name: "one", version: to_string(Application.spec(:one_core, :vsn))},
       instructions: instructions(who)
     }}
  end

  defp result("ping", _params, _from, _asked, _passed_on), do: {:ok, %{}}

  # A sharing link's holder is told of the tools they may call, and of nothing else that's in the network.
  defp result("tools/list", _params, _from, {by, who}, _passed_on) do
    holder = by != nil and User.link?(by)

    running =
      if holder,
        do: Map.new(Plugins.running(who), fn {name, version} -> {name, %{version | guide: nil}} end),
        else: Plugins.running(who)

    open = fn tools -> if holder, do: Enum.filter(tools, &(&1[:by_link] == true)), else: tools end

    listed = fn name, tool ->
      %{
        name: name,
        description: tool.description,
        inputSchema: tool.inputs,
        annotations: %{readOnlyHint: tool[:read_only] == true},
        _meta: %{fixed: tool[:fixed] == true}
      }
    end

    own = for tool <- open.(Tools.tools()), do: listed.(tool.name, tool)

    plugins =
      for {plugin, version} <- Enum.sort(running),
          tool <- open.(version.tools),
          do: listed.("#{plugin}_#{tool.name}", tool)

    guides = for {plugin, %{guide: guide}} when guide != nil <- running, into: %{}, do: {plugin, guide}
    {:ok, %{tools: own ++ plugins, _meta: %{guides: guides}}}
  end

  defp result("resources/" <> _ = method, params, _from, {by, who}, _passed_on) do
    if by != nil and User.link?(by),
      do: {:error, -32002, "A sharing link's holder reads nothing of a network but what a tool gives them"},
      else: resources(method, params, who)
  end

  defp result(method, _params, _from, _asked, _passed_on),
    do: {:error, -32601, "This server has no method #{method}"}

  defp resources("resources/list", _params, who) do
    resources =
      for {plugin, version} <- Enum.sort(Plugins.running(who)), path <- version.files do
        resource = %{uri: "ui://#{plugin}/#{version.code}/#{path}", name: path, mimeType: Version.kind(path)}

        case Enum.find(version.apps, &(&1.page == path)) do
          nil -> %{resource | mimeType: String.replace_suffix(resource.mimeType, ";profile=mcp-app", "")}
          app -> Map.merge(resource, %{name: app.name, icons: [%{src: app.icon}]})
        end
      end

    {:ok, %{resources: resources}}
  end

  defp resources("resources/read", %{"uri" => uri}, who) do
    with ["ui:", "", plugin, code, path] <- String.split(uri, "/", parts: 5),
         %{^plugin => version} <- Plugins.running(who),
         {:ok, text} <- Plugins.file(version.artifact, code, path) do
      {:ok, %{contents: [%{uri: uri, mimeType: Version.kind(path), text: text}]}}
    else
      _ -> {:error, -32002, "There's nothing at #{uri}"}
    end
  end

  defp resources(method, _params, _who), do: {:error, -32601, "This server has no method #{method}"}

  # What one server asks of another is run only for a server that says who it asks for, which only one acting
  # for them is believed about. This server runs it if it acts for whoever it's for too.
  defp between(true, run) do
    case run.() do
      {:ok, answer} -> {:ok, %{answer: answer}}
      {:error, why} -> {:error, -32000, why}
      {:passed_on, result} -> {:ok, result}
      :passed_back -> {:error, Server.passed_back(), "This server would pass that on in turn"}
    end
  end

  defp between(_passed_on, _run), do: {:error, -32000, "Only a server acting for whoever it asks for may ask that"}

  # A tool is one of One Core's own, or a plugin's by its name and the plugin's.
  defp call(name, inputs, asked, passed_on) do
    with :none <- Tools.call(name, inputs, asked, passed_on),
         [plugin, tool] <- String.split(name, "_", parts: 2),
         {:ok, answer} <- Plugins.call(plugin, tool, inputs, asked, passed_on) do
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

  # What an agent is told when it connects: each plugin's guide, and how to make a plugin.
  defp instructions(who) do
    guides =
      for {plugin, %{guide: guide}} when guide != nil <- Enum.sort(Plugins.running(who)),
          do: "# #{plugin}\n\n#{String.trim(guide)}"

    Enum.join(
      ["This is a One App network. Its tools are its plugins': <plugin>_<tool>." | guides] ++
        ["# Making a plugin\n\n" <> String.trim(Plugin.guide())],
      "\n\n"
    )
  end
end
