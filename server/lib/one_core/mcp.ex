defmodule OneCore.Mcp do
  @moduledoc """
  Answers what a client asks, in MCP's JSON-RPC: a message in, and its answer out. So a server is an MCP server to
  whatever reaches it, and the tools it lists are One Core's own and every running plugin's.

  Every request is from someone, which `OneCore.User.asking/2` says, and a plugin's tool runs for them.

  Servers ask each other the same way, and four things only a server acting for the same user may ask: to call a
  function of a plugin's service running here (`services/call`), and to run what was passed on of a service's
  call into its plugin (`plugins/call`), of a plugin's hook (`hooks/call`) or of having a plugin's scheduled jobs
  called (`plugins/schedule`).

  The parts of a plugin that run in a client are MCP resources: `ui://<plugin>/<code>/<path>`, where `code` names
  the set of files a version runs, so a file at an address never changes. A plugin's app is one of them, as an MCP
  App is.
  """
  require Logger
  alias OneCore.{Artifact, Hooks, Plugin, Plugins, Server, Servers, Services, User}
  alias OneCore.Plugins.Version

  # Tools are all a server speaks of MCP, and they are the same in each of these.
  @versions ~w(2025-11-25 2025-06-18 2025-03-26 2024-11-05)

  @tools [
    %{
      name: "deploy",
      description: """
      Adds a plugin to this network, or updates it, from its files.
      The server compiles them and runs the plugin at once, with its tools listed and called as <name>_<tool>. If
      they don't compile, it says why, and the version already running stays. The server's instructions say how to
      write a plugin.
      """,
      inputSchema: %{
        type: "object",
        properties: %{
          name: %{type: "string", description: "The plugin's name: lowercase letters, digits and hyphens"},
          files: %{type: "object", description: "The plugin's files: each one's text, by its path in the plugin"}
        },
        required: ["name", "files"]
      }
    },
    %{
      name: "remove",
      description: "Removes a plugin from this network. What it kept stays.",
      inputSchema: %{
        type: "object",
        properties: %{name: %{type: "string", description: "The plugin's name"}},
        required: ["name"]
      }
    },
    %{
      name: "upload",
      description: """
      Gives somewhere to send a file that's too large to go in a call: a link that takes it once, within a minute.
      Send the file as the link says, by its `method` to its `url` with its `headers`, then give `upload`, its
      name, to the plugin's tool that takes one. `one <plugin> <tool> --<input>-from <path>` does all of this.
      """,
      inputSchema: %{
        type: "object",
        properties: %{plugin: %{type: "string", description: "The plugin whose tool the file is for"}},
        required: ["plugin"]
      }
    },
    %{
      name: "add_server",
      description: "Lets another server act for you too, as a spare that takes over when this one stops.",
      inputSchema: %{
        type: "object",
        properties: %{key: %{type: "string", description: "The server's key, which it says when it's started"}},
        required: ["key"]
      }
    },
    %{
      name: "services",
      description: """
      Lists the services this network's plugins have, what each needs of a machine, and the servers that run it.
      A service is a part of a plugin that runs on particular machines, for what is only there.
      """,
      inputSchema: %{type: "object", properties: %{}},
      annotations: %{readOnlyHint: true}
    },
    %{
      name: "install_service",
      description: """
      Runs a plugin's service on a server from now on.
      That is the server's owner's to decide, knowing what the service needs of the machine, which `services` says.
      """,
      inputSchema: %{
        type: "object",
        properties: %{
          plugin: %{type: "string", description: "The plugin's name"},
          service: %{type: "string", description: "The service's name, as `services` lists it"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["plugin", "service"]
      }
    },
    %{
      name: "remove_service",
      description: "Stops running a plugin's service on a server.",
      inputSchema: %{
        type: "object",
        properties: %{
          plugin: %{type: "string", description: "The plugin's name"},
          service: %{type: "string", description: "The service's name, as `services` lists it"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["plugin", "service"]
      }
    }
  ]

  @own Enum.map(@tools, & &1.name)

  @doc "The answer to one message from whoever holds the key `from`, as JSON. A notification's is empty."
  def answer(message, from) do
    case JSON.decode(message) do
      {:ok, %{"method" => method, "id" => id} = request} ->
        params = Map.get(request, "params", %{})

        try do
          with {:ok, who} <- User.asking(from, said(params)),
               {:ok, result} <- result(method, params, from, who) do
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
      {:ok, %{"method" => "notifications/one/look"}} ->
        Server.look()
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

  defp result("services/call", %{"plugin" => plugin, "service" => service} = params, from, _who) do
    between(from, fn -> Services.run(plugin, service, params["function"], params["arguments"]) end)
  end

  defp result("plugins/call", %{"plugin" => plugin, "function" => function, "arguments" => arguments}, from, who) do
    with {:ok, result} <- between(from, fn -> Plugins.function(plugin, function, arguments, who, true) end) do
      Server.changed()
      {:ok, result}
    end
  end

  defp result("plugins/schedule", %{"plugin" => plugin}, from, _who) do
    between(from, fn -> Plugins.schedule(plugin, true) end)
  end

  defp result("hooks/call", %{"plugin" => plugin, "hook" => hook, "about" => about}, from, who) do
    between(from, fn -> Plugins.hook(plugin, hook, about, who, true) end)
  end

  defp result("tools/call", %{"name" => name} = params, _from, who) do
    case call(name, Map.get(params, "arguments", %{}), who, said(params)["passed_on"] == true) do
      {:ok, answer} -> changed(name, %{content: content(name, answer)})
      {:error, why} -> {:ok, %{content: [%{type: "text", text: why}], isError: true}}
      # Another server ran it, and told its own clients.
      {:passed_on, %{"isError" => true} = result} -> {:ok, result}
      {:passed_on, result} -> changed(name, result)
      :passed_back -> {:error, Server.passed_back(), "This server would pass #{name} on in turn"}
    end
  end

  defp result(method, params, _from, _who), do: result(method, params)

  # What one server asks of another is run only for a server acting for the same user, by one that acts for them
  # too.
  defp between(from, run) do
    if Services.acting?(from) and Services.acting?(Server.key()) do
      case run.() do
        {:ok, answer} -> {:ok, %{answer: answer}}
        {:error, why} -> {:error, -32000, why}
        {:passed_on, result} -> {:ok, result}
        :passed_back -> {:error, Server.passed_back(), "This server would pass that on in turn"}
      end
    else
      {:error, -32000, "Only a server acting for the same user may ask that"}
    end
  end

  defp result("initialize", params) do
    asked = params["protocolVersion"]

    {:ok,
     %{
       protocolVersion: if(asked in @versions, do: asked, else: hd(@versions)),
       capabilities: %{tools: %{}, resources: %{}},
       serverInfo: %{name: "one", version: to_string(Application.spec(:one_core, :vsn))},
       instructions: instructions()
     }}
  end

  defp result("ping", _), do: {:ok, %{}}

  defp result("tools/list", _) do
    running = Plugins.running()

    plugins =
      for {plugin, version} <- Enum.sort(running), tool <- version.tools do
        %{
          name: "#{plugin}_#{tool.name}",
          description: tool.description,
          inputSchema: tool.inputs,
          annotations: %{readOnlyHint: tool[:read_only] == true},
          _meta: %{fixed: tool[:fixed] == true}
        }
      end

    guides = for {plugin, %{guide: guide}} when guide != nil <- running, into: %{}, do: {plugin, guide}
    {:ok, %{tools: @tools ++ plugins, _meta: %{guides: guides}}}
  end

  defp result("resources/list", _) do
    resources =
      for {plugin, version} <- Enum.sort(Plugins.running()), path <- version.files do
        resource = %{uri: "ui://#{plugin}/#{version.code}/#{path}", name: path, mimeType: Version.kind(path)}

        case Enum.find(version.apps, &(&1.page == path)) do
          nil -> %{resource | mimeType: String.replace_suffix(resource.mimeType, ";profile=mcp-app", "")}
          app -> Map.merge(resource, %{name: app.name, icons: [%{src: app.icon}]})
        end
      end

    {:ok, %{resources: resources}}
  end

  defp result("resources/read", %{"uri" => uri}) do
    with ["ui:", "", plugin, code, path] <- String.split(uri, "/", parts: 5),
         {:ok, text} <- Plugins.file(plugin, code, path) do
      {:ok, %{contents: [%{uri: uri, mimeType: Version.kind(path), text: text}]}}
    else
      _ -> {:error, -32002, "There's nothing at #{uri}"}
    end
  end

  defp result(method, _), do: {:error, -32601, "This server has no method #{method}"}

  # One Core's own tools are for the network's users.
  defp call(name, _inputs, "links/" <> _, _passed_on) when name in @own,
    do: {:error, "#{name} isn't a tool that a sharing link's holder calls"}

  defp call("deploy", %{"name" => name, "files" => files}, _who, _passed_on) do
    case Plugins.deploy(name, files) do
      {:ok, number, ""} -> {:ok, "#{name} is running, at version #{number}."}
      {:ok, number, warnings} -> {:ok, "#{name} is running, at version #{number}. The compiler warns:\n#{warnings}"}
      {:error, why} -> {:error, why}
    end
  end

  defp call("remove", %{"name" => name}, _who, _passed_on) do
    with :ok <- Plugins.remove(name), do: {:ok, "#{name} is removed."}
  end

  # A file sent this way lands in the plugin's own artifact, under a name nobody could guess.
  defp call("upload", %{"plugin" => plugin}, _who, _passed_on) do
    if is_map_key(Plugins.running(), plugin) do
      name = "uploads/" <> Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)
      own = %Artifact{prefix: Plugins.own(plugin) <> "/"}
      {:ok, JSON.encode!(Map.put(Artifact.link(own, name, :write), :upload, name))}
    else
      {:error, "There's no plugin named #{plugin}"}
    end
  end

  defp call("add_server", %{"key" => key}, who, _passed_on) do
    with :ok <- Servers.add(key, Server.key()) do
      Hooks.heard("joined", %{"server" => key, "by" => Server.key()}, who)
      Server.tell_servers()
      {:ok, "That server acts for you too now. Start it, and it's a spare."}
    end
  end

  defp call("services", _inputs, _who, _passed_on) do
    {:ok, JSON.encode!(%{asked: Server.key(), servers: Servers.acting(), services: Services.list()})}
  end

  defp call("install_service", %{"plugin" => plugin, "service" => service} = inputs, who, _passed_on) do
    on(inputs["server"], "install_service", inputs, who, fn ->
      with :ok <- Services.install(plugin, service), do: {:ok, "#{service} is running on the server #{Server.key()}."}
    end)
  end

  defp call("remove_service", %{"plugin" => plugin, "service" => service} = inputs, who, _passed_on) do
    on(inputs["server"], "remove_service", inputs, who, fn ->
      with :ok <- Services.remove(plugin, service),
           do: {:ok, "#{service} no longer runs on the server #{Server.key()}."}
    end)
  end

  defp call(name, inputs, who, passed_on) do
    with [plugin, tool] <- String.split(name, "_", parts: 2),
         {:ok, answer} <- Plugins.call(plugin, tool, inputs, who, passed_on) do
      case answer do
        text when is_binary(text) -> {:ok, text}
        {:blob, _kind, _data} -> {:ok, answer}
        {:file, _kind, %Artifact{}, _name} -> {:ok, answer}
        _ -> {:ok, JSON.encode!(answer)}
      end
    else
      [_] -> {:error, "#{name} takes other inputs, or isn't a tool. tools/list says what each takes."}
      other -> other
    end
  end

  # Runs what a tool does on the server it names: here, or by asking that server to, for whoever asked.
  defp on(server, tool, inputs, who, here) do
    if server in [nil, Server.key()] do
      here.()
    else
      case Server.request(server, "tools/call", %{name: tool, arguments: inputs, _meta: %{who: who}}) do
        {:ok, result} -> {:passed_on, result}
        {:error, _code, why} -> {:error, why}
        down -> {:error, Server.said(down)}
      end
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
  defp changed(name, result) do
    if not reads?(name), do: Server.changed()
    {:ok, result}
  end

  # Whether calling a tool only reads: one of One Core's own that says so, or a plugin's.
  defp reads?(name) do
    with [plugin, tool] <- String.split(name, "_", parts: 2),
         %{^plugin => version} <- Plugins.running() do
      Enum.any?(version.tools, &(&1.name == tool and &1[:read_only] == true))
    else
      _ -> Enum.any?(@tools, &(&1.name == name and &1[:annotations][:readOnlyHint] == true))
    end
  end

  # What an agent is told when it connects: each plugin's guide, and how to make a plugin.
  defp instructions do
    guides =
      for {plugin, %{guide: guide}} when guide != nil <- Enum.sort(Plugins.running()),
          do: "# #{plugin}\n\n#{String.trim(guide)}"

    Enum.join(
      ["This is a One App network. Its tools are its plugins': <plugin>_<tool>." | guides] ++
        ["# Making a plugin\n\n" <> String.trim(Plugin.guide())],
      "\n\n"
    )
  end
end
