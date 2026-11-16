defmodule Chrome do
  @moduledoc """
  Chrome: Google Chrome in a container of the Containers plugin, of its image `desktop`, headed on that container's
  display, and driven by Microsoft's Playwright MCP server, which runs beside it and reaches it over its DevTools
  Protocol. Its tools are that server's, named as it names them without `browser_`, and each takes the browser's
  `session`.

  A session is the container `chrome-<session>`, created through Containers' tools for whoever opened it, so whose
  it is and who reaches it is what Containers says, and Containers stops it once it's gone unused for its
  `idle_hours`. The MCP server's own session id for each is kept in the plugin's own artifact, under
  `sessions/<whose>/<session>`.

  A browser's profile is its user data dir, `/root/.chromium` in its container. As one is created it offers
  `chrome.starting`, for a plugin to hand it a user data dir to start from, and as it closes `chrome.closing`, with
  the one it closed with, for that plugin to keep: each a gzipped tarball in its container, which the plugin writes
  or reads with Containers' tools, so no hook carries a profile's bytes.

  A running browser is a screen in Computer Use, as every container with a display is, and `screen` acts on it for an
  app that lets its person use the browser: to sign in, or to get an agent past a captcha.
  """
  @behaviour OneCore.Plugin
  alias OneCore.{Artifact, Call}

  @port 8931
  @user_data "/root/.chromium"
  # Where a user data dir is handed over, as a gzipped tarball, in the browser's container.
  @tarball "/tmp/user-data.tar.gz"
  # Starts Chrome (the image's `browser`), at the page given or a new tab, and the MCP server beside it: HTTP on the container's
  # interfaces, from whatever host `request` names it by, on Chrome's DevTools Protocol, with one browser context
  # that every MCP session shares, so a session started again finds the same pages. Neither is the container's own
  # command, so each is started in a session of its own, to keep running once this has.
  @launch ~s(setsid browser "$@" </dev/null >/dev/null 2>&1 & ) <>
            ~s(setsid playwright-mcp --port #{@port} --host 0.0.0.0 --allowed-hosts '*' ) <>
            ~s(--cdp-endpoint http://127.0.0.1:9222 --shared-browser-context </dev/null >/tmp/mcp.log 2>&1 &)
  # Quits Chrome as a person would, over its DevTools Protocol (Browser.close), so it writes everything down, which
  # a signal doesn't wait for. Once it has (or, after 10 seconds, it's killed, as are its helpers) packs what of its
  # user data dir travels: `Local State` and the profile, `Default`, without its caches. The rest is what Chrome
  # downloads by itself (its components, tens of MB). A Service Worker's ScriptCache stays: it's the only copy of an
  # extension's background script.
  @pack ~s"""
  main=$(pgrep -o -f -- '[-]-user-data-dir=#{@user_data}')
  node -e 'const ws = new WebSocket(process.argv[1]); ws.onopen = () => ws.send(JSON.stringify({ id: 1, method: "Browser.close" }))' \\
    "$(curl -s 127.0.0.1:9222/json/version | sed -n 's/.*"webSocketDebuggerUrl": "\\(.*\\)".*/\\1/p')" &
  for i in $(seq 100); do kill -0 "$main" 2>/dev/null || break; sleep 0.1; done
  pkill -9 -f -- '[-]-user-data-dir=#{@user_data}'
  sed -i 's/"exit_type":"Crashed"/"exit_type":"Normal"/' #{@user_data}/Default/Preferences 2>/dev/null
  cd #{@user_data} && tar -czf #{@tarball} --anchored --warning=no-file-changed \\
    --exclude='./Default/Cache' --exclude='./Default/Code Cache' --exclude='./Default/GPUCache' \\
    --exclude='./Default/DawnGraphiteCache' --exclude='./Default/DawnWebGPUCache' \\
    --exclude='./Default/Service Worker/CacheStorage' './Local State' ./Default
  """
  @session %{type: "string", pattern: "^[a-z0-9][a-z0-9_.-]*$", description: "the browser, by the name `open` gave it"}
  @target %{type: "string", description: "the element: its ref from the snapshot, like e12, or a unique selector"}
  @element %{type: "string", description: "the element in a few words, as a person would name it"}

  @impl true
  def uses, do: ["containers"]

  @impl true
  def offers do
    [
      %{
        name: "starting",
        kind: :mutate,
        description:
          "A browser is about to start for the first time, for whoever the call is for: its `session`, `description`, " <>
            "the `url` it opens, its `container`, as Containers' tools name it, and `user_data`, nil. A plugin that has " <>
            "a user data dir for it writes it into that container as a gzipped tarball of the folder's contents " <>
            "(Containers' `write_file`, at #{@tarball}), and answers %{\"user_data\" => \"#{@tarball}\"}: the browser " <>
            "starts from it."
      },
      %{
        name: "closing",
        kind: :validate,
        description:
          "A browser is being closed with `close`, and Chrome has quit: its `session`, `description`, `container`, " <>
            "and `user_data`, the path in that container of a gzipped tarball of its user data dir as it closed, " <>
            "without caches, for a plugin to read now (Containers' `read_file`), before the container is removed. " <>
            "A plugin refuses when it couldn't keep what it needed of it: the browser then stays, its Chrome " <>
            "stopped, and `close` says why."
      }
    ]
  end

  @impl true
  def tools do
    [
      %{
        name: "open",
        description:
          "Open a browser: a Google Chrome of yours in a container, signed in to nothing. Opening one that's open " <>
            "already, or that was stopped after going unused, carries on with it. The first on a machine builds its " <>
            "image, a few minutes; after that it's a few seconds.",
        inputs:
          object(%{description: %{type: "string", description: "what it's for, in a few words"}, url: %{type: "string", description: "the page to open"}}, [
            "description"
          ])
      },
      %{
        name: "list",
        read_only: true,
        description: "Your browsers: each one's session, what it's for, its state (running or exited), its machine, and when it was created and last used (ms since 1970).",
        inputs: %{type: "object", properties: %{}}
      },
      %{name: "close", description: "Close a browser and remove its container, for good.", inputs: object()},
      %{name: "navigate", description: "Navigate to a URL.", inputs: object(%{url: %{type: "string"}}, ["url"])},
      %{name: "navigate_back", description: "Go back to the previous page in the history.", inputs: object()},
      %{
        name: "snapshot",
        read_only: true,
        description:
          "The page as its accessibility tree, in YAML, each element with a ref (like e12) that the other tools take as its `target`. Far cheaper than a screenshot, and what to act from.",
        inputs:
          object(%{
            target: %{type: "string", description: "only this element's part of the page"},
            depth: %{type: "number", description: "how deep to go"},
            boxes: %{type: "boolean", description: "with each element's box as [box=x,y,width,height], in CSS pixels"}
          })
      },
      %{
        name: "click",
        description: "Click an element.",
        inputs:
          object(
            %{
              target: @target,
              element: @element,
              doubleClick: %{type: "boolean"},
              button: %{type: "string", enum: ~w(left right middle)},
              modifiers: %{type: "array", items: %{type: "string", enum: ~w(Alt Control ControlOrMeta Meta Shift)}}
            },
            ["target"]
          )
      },
      %{
        name: "hover",
        description: "Hover over an element.",
        inputs: object(%{target: @target, element: @element}, ["target"])
      },
      %{
        name: "type",
        description: "Type text into an editable element, replacing what's there.",
        inputs:
          object(
            %{
              target: @target,
              element: @element,
              text: %{type: "string"},
              submit: %{type: "boolean", description: "press Enter after"},
              slowly: %{type: "boolean", description: "one character at a time, for pages that listen to keys"}
            },
            ["target", "text"]
          )
      },
      %{
        name: "fill_form",
        description: "Fill several fields of a form at once.",
        inputs:
          object(
            %{
              fields: %{
                type: "array",
                items: %{
                  type: "object",
                  required: ["target", "name", "type", "value"],
                  properties: %{
                    target: @target,
                    name: %{type: "string", description: "the field's name, as a person reads it"},
                    type: %{type: "string", enum: ~w(textbox checkbox radio combobox slider)},
                    value: %{type: "string", description: "its value: true or false for a checkbox, the option's text for a combobox"}
                  }
                }
              }
            },
            ["fields"]
          )
      },
      %{
        name: "select_option",
        description: "Select options in a dropdown.",
        inputs: object(%{target: @target, element: @element, values: %{type: "array", items: %{type: "string"}}}, ["target", "values"])
      },
      %{
        name: "press_key",
        description: "Press a key, like ArrowLeft, Enter or a.",
        inputs: object(%{key: %{type: "string"}}, ["key"])
      },
      %{
        name: "wait_for",
        description: "Wait for text to appear or to go, or for some seconds to pass (at most 30), whichever is given.",
        inputs: object(%{text: %{type: "string"}, textGone: %{type: "string"}, time: %{type: "number"}})
      },
      %{
        name: "evaluate",
        description: "Run JavaScript in the page, and give what it returns.",
        inputs:
          object(
            %{
              function: %{type: "string", description: "() => { ... }, or (element) => { ... } with a target"},
              target: @target,
              element: @element
            },
            ["function"]
          )
      },
      %{
        name: "take_screenshot",
        read_only: true,
        description: "A picture of the page, or of one element. Use snapshot to act from; this is for what only a picture shows.",
        inputs:
          object(%{
            type: %{type: "string", enum: ~w(png jpeg)},
            fullPage: %{type: "boolean", description: "the whole page, not only what's in view"},
            target: @target,
            element: @element
          })
      },
      %{
        name: "tabs",
        description: "List, open (new), close or select a tab. close and select take its index; new takes a url.",
        inputs:
          object(%{action: %{type: "string", enum: ~w(list new close select)}, index: %{type: "number"}, url: %{type: "string"}}, [
            "action"
          ])
      },
      %{
        name: "handle_dialog",
        description: "Accept or dismiss the alert, confirm or prompt the page opened.",
        inputs: object(%{accept: %{type: "boolean"}, promptText: %{type: "string"}}, ["accept"])
      },
      %{
        name: "screen",
        description:
          "Look at or act on a browser's screen by its pixels: the whole browser, its address bar and tabs too, for an app that lets its person use it. Computer Use's tools say what each action takes. `resize` makes the screen width × height, and the browser, maximised, follows it. `stream` shows it as live video over WebRTC: with no `offer`, the ICE servers to connect with; with the app's whole SDP `offer`, {answer}.",
        inputs:
          object(
            %{
              action: %{type: "string", enum: ~w(screenshot click double_click move drag scroll type key resize stream)},
              offer: %{type: "string", description: "for stream: the app's SDP offer, with its candidates"},
              x: %{type: "integer", minimum: 0},
              y: %{type: "integer", minimum: 0},
              button: %{type: "string", enum: ~w(left right middle)},
              path: %{type: "array", minItems: 2, items: %{type: "object", required: ["x", "y"], properties: %{x: %{type: "integer"}, y: %{type: "integer"}}}},
              direction: %{type: "string", enum: ~w(up down left right)},
              amount: %{type: "integer", minimum: 1, maximum: 50},
              text: %{type: "string"},
              keys: %{type: "string", description: "as xdotool names them: ctrl+l, Return; several, space between"},
              width: %{type: "integer", minimum: 320, maximum: 7680},
              height: %{type: "integer", minimum: 240, maximum: 4320}
            },
            ["action"]
          )
      }
    ]
  end

  defp object(properties \\ %{}, required \\ []),
    do: %{type: "object", required: ["session" | required], properties: Map.put(properties, :session, @session)}

  @impl true
  def call("list", _inputs, call) do
    for c <- containers(call) do
      %{session: session(c), description: c["description"], state: c["state"], machine: c["machine"], created: c["created"], last_used: c["last_used"]}
    end
  end

  def call("open", %{"session" => session} = inputs, call) do
    case Enum.find(containers(call), &(session(&1) == session)) do
      %{"state" => "running"} ->
        if inputs["url"], do: mcp(call, session, "browser_navigate", %{"url" => inputs["url"]}), else: "Opened #{session}"

      # A container started again keeps its files, its user data dir with them, and has a new MCP server, which knows
      # none of the sessions kept from before.
      %{} ->
        forget(call, session)
        containers(call, "start", session)
        launch(call, session, inputs["url"])

      nil ->
        forget(call, session)
        create(call, inputs)
    end
  end

  def call("close", %{"session" => session}, call) do
    case Enum.find(containers(call), &(session(&1) == session)) do
      nil ->
        raise "You have no browser called #{session}"

      %{"state" => "running"} = c ->
        packed = containers(call, "exec", session, %{"command" => @pack, "timeout" => 120})
        if packed["exit"] != 0, do: raise("Couldn't close #{session}: #{String.trim(packed["stderr"])}")
        about = %{"session" => session, "description" => c["description"], "container" => c["name"], "user_data" => @tarball}
        run_hook(call, "closing", about)

      %{} ->
        :ok
    end

    containers(call, "remove", session)
    forget(call, session)
    "Closed #{session}"
  end

  def call("screen", %{"session" => session} = inputs, call), do: containers(call, "screen", session, Map.delete(inputs, "session"))

  def call(tool, %{"session" => session} = inputs, call) do
    mcp(call, session, "browser_" <> tool, Map.delete(inputs, "session"))
  end

  # Creates the browser's container, with its display and no Chrome yet, puts in the user data dir a plugin hands
  # it, if one does, and starts Chrome on it.
  defp create(call, %{"session" => session} = inputs) do
    c =
      Call.tool(call, "containers", "create", %{
        "image" => "desktop",
        "name" => "chrome-" <> session,
        "description" => inputs["description"],
        "size" => "medium",
        "ports" => [@port],
        "command" => ["desktop", "--no-browser"]
      })

    about = %{"session" => session, "description" => inputs["description"], "url" => inputs["url"], "container" => c["name"], "user_data" => nil}

    case run_hook(call, "starting", about)["user_data"] do
      nil ->
        :ok

      tarball ->
        unpack = "mkdir -p #{@user_data} && tar -xzf #{shell(tarball)} -C #{@user_data} && rm #{shell(tarball)}"
        unpacked = containers(call, "exec", session, %{"command" => unpack})
        if unpacked["exit"] != 0, do: raise("Couldn't open the profile #{session} was handed: #{String.trim(unpacked["stderr"])}")
    end

    launch(call, session, inputs["url"])
  end

  defp launch(call, session, url) do
    containers(call, "exec", session, %{"command" => "sh -c #{shell(@launch)} sh #{if url, do: shell(url)}"})
    "Opened #{session}" <> if(url, do: " at #{url}", else: "")
  end

  defp shell(word), do: "'" <> String.replace(word, "'", ~S('\'')) <> "'"

  # Runs one of the hooks Chrome offers, and gives what's changed: nothing on a server from before plugins could
  # offer hooks (One Core 7929556e6), where no plugin can join one.
  defp run_hook(call, hook, about) do
    if function_exported?(Call, :run_hook, 3) do
      case apply(Call, :run_hook, [call, hook, about]) do
        {:ok, changes, _by} -> changes
        {:refuse, why} -> raise why
        :ok -> %{}
      end
    else
      %{}
    end
  end

  # Calls a tool of the MCP server in the session's container, in the MCP session kept for it, and gives its answer:
  # its text, or its picture.
  defp mcp(call, session, tool, arguments, again? \\ true) do
    id = session_id(call, session) || initialize(call, session)
    # Each request has an id of its own: the server answers two at once in one session by their ids.
    request = %{
      "jsonrpc" => "2.0",
      "id" => System.unique_integer([:positive]),
      "method" => "tools/call",
      "params" => %{"name" => tool, "arguments" => arguments}
    }

    case request(call, session, id, request) do
      # The MCP server forgets its sessions when its container stops, so one kept from before is begun again.
      %{"status" => 404} when again? ->
        forget(call, session)
        mcp(call, session, tool, arguments, false)

      %{"body" => body} ->
        case decode(body) do
          %{"result" => %{"content" => content} = result} -> tool_result(content, result["isError"])
          %{"error" => %{"message" => why}} -> raise why
        end
    end
  end

  defp tool_result(content, error?) do
    text = Enum.join(for(%{"type" => "text", "text" => t} <- content, do: t), "\n")

    case for(%{"type" => "image"} = image <- content, do: image) do
      _ when error? == true -> raise text
      [%{"data" => data, "mimeType" => type} | _] -> {:blob, type, Base.decode64!(data)}
      [] -> text
    end
  end

  # Begins an MCP session with the server in the session's container, and keeps its id. The server takes a second
  # or two to listen after its container starts, which is why it's asked again, for up to 20 seconds.
  defp initialize(call, session, tries \\ 80) do
    request = %{
      "jsonrpc" => "2.0",
      "id" => 0,
      "method" => "initialize",
      "params" => %{"protocolVersion" => "2025-06-18", "capabilities" => %{}, "clientInfo" => %{"name" => "chrome", "version" => "1"}}
    }

    %{"status" => 200, "headers" => %{"mcp-session-id" => id}} = request(call, session, nil, request)
    Artifact.write(call.own, key(call, session), id)
    id
  rescue
    e ->
      if tries == 0, do: reraise(e, __STACKTRACE__)
      Process.sleep(250)
      initialize(call, session, tries - 1)
  end

  defp request(call, session, id, body) do
    headers = %{"content-type" => "application/json", "accept" => "application/json, text/event-stream"}
    headers = if id, do: Map.put(headers, "mcp-session-id", id), else: headers

    containers(call, "request", session, %{"port" => @port, "path" => "/mcp", "method" => "POST", "headers" => headers, "body" => JSON.encode!(body)})
  end

  # A response is one JSON-RPC message, as JSON or as the data of one server-sent event.
  defp decode(text) do
    case Regex.run(~r/^data: (.*)$/m, text) do
      [_, data] -> JSON.decode!(data)
      nil -> JSON.decode!(text)
    end
  end

  defp session_id(call, session) do
    case Artifact.read(call.own, key(call, session)) do
      {:ok, id, _tag} -> id
      :absent -> nil
    end
  end

  defp forget(call, session), do: Artifact.remove(call.own, key(call, session))
  defp key(call, session), do: "sessions/#{call.subject}/#{session}"

  # Calls a tool of Containers on the session's container. What went wrong is said of the browser where it's that
  # there's none, or that it was stopped.
  defp containers(call, tool, session, inputs \\ %{}) do
    Call.tool(call, "containers", tool, Map.put(inputs, "container", "chrome-" <> session))
  rescue
    e ->
      case Enum.find(containers(call), &(session(&1) == session)) do
        nil -> raise "You have no browser called #{session}"
        %{"state" => "running"} -> reraise e, __STACKTRACE__
        %{} -> raise "#{session} was stopped after going unused: `open` starts it again, afresh"
      end
  end

  # The browsers whoever this call is for may use: their containers called chrome-<session>, of the image desktop.
  defp containers(call),
    do: Enum.filter(Call.tool(call, "containers", "list", %{}), &(String.starts_with?(&1["name"], "chrome-") and &1["image"] =~ "containers-desktop"))

  defp session(%{"name" => "chrome-" <> session}), do: session
end
