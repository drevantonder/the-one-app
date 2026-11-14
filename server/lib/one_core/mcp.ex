defmodule OneCore.Mcp do
  @moduledoc """
  Answers what a client asks, in MCP's JSON-RPC: a message in, and its answer out. So a server is an MCP server to
  whatever reaches it, and the tools it lists are One Core's own two and every running plugin's.

  The parts of a plugin that run in a client are MCP resources: `ui://<plugin>/<code>/<path>`, where `code` names
  the set of files a version runs, so a file at an address never changes. A plugin's app is one of them, as an MCP
  App is.
  """
  require Logger
  alias OneCore.{Plugin, Plugins}
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
    }
  ]

  @doc "The answer to one message, as JSON. A notification's is empty."
  def answer(message) do
    case JSON.decode(message) do
      {:ok, %{"method" => method, "id" => id} = request} ->
        try do
          case result(method, Map.get(request, "params", %{})) do
            {:ok, result} -> JSON.encode!(%{jsonrpc: "2.0", id: id, result: result})
            {:error, code, why} -> JSON.encode!(%{jsonrpc: "2.0", id: id, error: %{code: code, message: why}})
          end
        rescue
          error ->
            Logger.error(Exception.format(:error, error, __STACKTRACE__))
            JSON.encode!(%{jsonrpc: "2.0", id: id, error: %{code: -32603, message: Exception.message(error)}})
        end

      {:ok, %{"method" => _notification}} ->
        ""

      _ ->
        JSON.encode!(%{jsonrpc: "2.0", id: nil, error: %{code: -32700, message: "That isn't a JSON-RPC request"}})
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
          annotations: %{readOnlyHint: tool[:read_only] == true}
        }
      end

    guides = for {plugin, %{guide: guide}} when guide != nil <- running, into: %{}, do: {plugin, guide}
    {:ok, %{tools: @tools ++ plugins, _meta: %{guides: guides}}}
  end

  defp result("tools/call", %{"name" => name} = params) do
    case call(name, Map.get(params, "arguments", %{})) do
      {:ok, text} ->
        if not reads?(name), do: OneCore.Server.tell(%{jsonrpc: "2.0", method: "notifications/one/changed"})
        {:ok, %{content: [%{type: "text", text: text}]}}

      {:error, why} ->
        {:ok, %{content: [%{type: "text", text: why}], isError: true}}
    end
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

  defp call("deploy", %{"name" => name, "files" => files}) do
    case Plugins.deploy(name, files) do
      {:ok, number, ""} -> {:ok, "#{name} is running, at version #{number}."}
      {:ok, number, warnings} -> {:ok, "#{name} is running, at version #{number}. The compiler warns:\n#{warnings}"}
      {:error, why} -> {:error, why}
    end
  end

  defp call("remove", %{"name" => name}) do
    with :ok <- Plugins.remove(name), do: {:ok, "#{name} is removed."}
  end

  defp call(name, inputs) do
    with [plugin, tool] <- String.split(name, "_", parts: 2),
         {:ok, answer} <- Plugins.call(plugin, tool, inputs) do
      if is_binary(answer), do: {:ok, answer}, else: {:ok, JSON.encode!(answer)}
    else
      [_] -> {:error, "#{name} takes other inputs, or isn't a tool. tools/list says what each takes."}
      {:error, why} -> {:error, why}
    end
  end

  # Whether calling a tool only reads. One Core's own two don't.
  defp reads?(name) do
    with [plugin, tool] <- String.split(name, "_", parts: 2),
         %{^plugin => version} <- Plugins.running() do
      Enum.any?(version.tools, &(&1.name == tool and &1[:read_only] == true))
    else
      _ -> false
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
