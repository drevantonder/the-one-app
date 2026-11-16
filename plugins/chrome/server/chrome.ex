defmodule Chrome do
  @moduledoc """
  Chrome: a headless Chromium in a container of the Containers plugin, driven by Microsoft's Playwright MCP server,
  which runs beside it in that container (the image `mcr.microsoft.com/playwright/mcp`). Its tools are that server's,
  named as it names them without `browser_`, and each takes the browser's `session`.

  A session is the container `chrome-<session>`, created through Containers' tools for whoever opened it, so whose
  it is and who reaches it is what Containers says, and Containers stops it once it's gone unused for its
  `idle_hours`. The MCP server's own session id for each is kept in the plugin's own artifact, under
  `sessions/<whose>/<session>`.

  As a browser starts, when it's created or started again after being stopped, it offers `chrome.starting`, for a
  plugin to hand it a storage state to start with, and as it closes `chrome.closing`, with the storage state it
  closed with, for that plugin to keep.

  A running browser is a screen in Computer Use (`computer-use.screens`), acted on by `screen` with Playwright's own
  mouse and keyboard, so its person can watch it and use it: to sign in, or to get an agent past a captcha.
  """
  @behaviour OneCore.Plugin
  alias OneCore.{Artifact, Call}

  @image "mcr.microsoft.com/playwright/mcp:v0.0.85"
  @port 8931
  # The MCP server's flags, after the image's own (headless Chromium, no sandbox): HTTP on the container's
  # interfaces, from whatever host `request` names it by, with the storage state tools and the mouse by coordinates
  # (vision), a viewport the size a screen says it is, and one browser that every MCP session shares, so a session
  # started again finds the same pages.
  @width 1280
  @height 800
  @command [
    "--port",
    "#{@port}",
    "--host",
    "0.0.0.0",
    "--allowed-hosts",
    "*",
    "--caps",
    "storage,vision",
    "--viewport-size",
    "#{@width}x#{@height}",
    "--shared-browser-context"
  ]
  # Where the storage state is written in the container: the MCP server's workspace is its user's home.
  @state "/home/node/state.json"

  @session %{type: "string", pattern: "^[a-z0-9][a-z0-9_.-]*$", description: "the browser, by the name `open` gave it"}
  @target %{type: "string", description: "the element: its ref from the snapshot, like e12, or a unique selector"}
  @element %{type: "string", description: "the element in a few words, as a person would name it"}

  @impl true
  def uses, do: ["containers"]

  @impl true
  def hooks, do: ["computer-use.screens"]

  @impl true
  def offers do
    [
      %{
        name: "starting",
        kind: :mutate,
        description:
          "A browser is about to start, created or started again after being stopped, for whoever the call is for: its " <>
            "`session`, `description` and the `url` it opens, " <>
            "and `storage_state`, nil. A plugin answers %{\"storage_state\" => state}: Playwright's storage state, " <>
            "%{\"cookies\" => [...], \"origins\" => [%{\"origin\" => ..., \"localStorage\" => [%{\"name\" => ..., \"value\" => ...}]}]}, " <>
            "which the browser starts with."
      },
      %{
        name: "closing",
        kind: :observe,
        description:
          "A browser was closed with `close`: its `session`, `description` and `storage_state`, its cookies and local " <>
            "storage as it closed, in Playwright's shape, for a plugin to keep."
      }
    ]
  end

  @impl true
  def tools do
    [
      %{
        name: "open",
        description:
          "Open a browser: a headless Chromium of yours in a container, signed in to nothing. Opening one that's open " <>
            "already, or that was stopped after going unused, carries on with it. The first on a machine pulls its " <>
            "image, about a minute; after that it's a few seconds.",
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
          "Look at or act on a browser's page by its pixels, for Computer Use, whose tools say what each action takes, and for an app that lets its person use the browser.",
        inputs:
          object(
            %{
              action: %{type: "string", enum: ~w(screenshot click double_click move drag scroll type key)},
              x: %{type: "integer", minimum: 0},
              y: %{type: "integer", minimum: 0},
              button: %{type: "string", enum: ~w(left right middle)},
              path: %{type: "array", minItems: 2, items: %{type: "object", required: ["x", "y"], properties: %{x: %{type: "integer"}, y: %{type: "integer"}}}},
              direction: %{type: "string", enum: ~w(up down left right)},
              amount: %{type: "integer", minimum: 1, maximum: 50},
              text: %{type: "string"},
              keys: %{type: "string", description: "as xdotool names them: ctrl+l, Return; several, space between"}
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
    # A container started again has a new MCP server, which knows none of the sessions kept from before.
    case Enum.find(containers(call), &(session(&1) == session)) do
      %{"state" => "running"} -> :ok
      %{} = stopped ->
        forget(call, session)
        state = starting(call, Map.put(inputs, "description", stopped["description"]))
        containers(call, "start", session)
        load(call, session, state)

      nil ->
        forget(call, session)
        create(call, inputs)
    end

    if inputs["url"], do: mcp(call, session, "browser_navigate", %{"url" => inputs["url"]}), else: "Opened #{session}"
  end

  def call("close", %{"session" => session}, call) do
    case Enum.find(containers(call), &(session(&1) == session)) do
      nil ->
        raise "You have no browser called #{session}"

      %{"state" => "running"} = c ->
        mcp(call, session, "browser_storage_state", %{"filename" => Path.basename(@state)})
        state = JSON.decode!(containers(call, "read_file", session, %{"path" => @state}))
        about = %{"session" => session, "description" => c["description"], "storage_state" => state}
        run_hook(call, "closing", about)

      %{} ->
        :ok
    end

    containers(call, "remove", session)
    forget(call, session)
    "Closed #{session}"
  end

  def call("screen", %{"session" => session, "action" => "screenshot"}, call),
    do: mcp(call, session, "browser_take_screenshot", %{"type" => "png"})

  # What's typed is often a password, and the MCP server says back the code it ran, which holds it: so nothing it
  # says is passed on.
  def call("screen", %{"session" => session, "action" => "type", "text" => text}, call) do
    code = "async (page) => { await page.keyboard.type(#{JSON.encode!(text)}) }"

    try do
      mcp(call, session, "browser_run_code_unsafe", %{"code" => code})
    rescue
      _ -> raise "Couldn't type in #{session}"
    end

    "Typed #{String.length(text)} characters"
  end

  def call("screen", %{"session" => session, "action" => action} = inputs, call) do
    for {tool, arguments} <- pointer(action, inputs), do: mcp(call, session, tool, arguments)
    did(action, inputs)
  end

  def call(tool, %{"session" => session} = inputs, call) do
    mcp(call, session, "browser_" <> tool, Map.delete(inputs, "session"))
  end

  defp create(call, %{"session" => session} = inputs) do
    state = starting(call, inputs)

    Call.tool(call, "containers", "create", %{
      "image" => @image,
      "name" => "chrome-" <> session,
      "description" => inputs["description"],
      "size" => "medium",
      "ports" => [@port],
      "command" => @command
    })

    load(call, session, state)
  end

  # The storage state a plugin hands a browser that's starting, if one does.
  defp starting(call, inputs) do
    about = %{"session" => inputs["session"], "description" => inputs["description"], "url" => inputs["url"], "storage_state" => nil}
    run_hook(call, "starting", about)["storage_state"]
  end

  defp load(_call, _session, nil), do: :ok

  defp load(call, session, state) do
    containers(call, "write_file", session, %{"path" => @state, "text" => JSON.encode!(state)})
    mcp(call, session, "browser_set_storage_state", %{"filename" => Path.basename(@state)})
  end

  # Computer Use asks for the caller's screens: each running browser of theirs is one, called what its session is.
  @impl true
  def hook("computer-use.screens", _about, call) do
    screens =
      for c <- containers(call), c["state"] == "running" do
        %{
          "name" => session(c),
          "description" => "#{c["description"]}, a browser on #{c["machine"]}",
          "width" => @width,
          "height" => @height,
          "tool" => "screen",
          "inputs" => %{"session" => session(c)}
        }
      end

    %{call.plugin => screens}
  end

  # Computer Use's actions, as the MCP server's mouse and keys: each a tool and its arguments.
  defp pointer("click", %{"x" => x, "y" => y} = inputs),
    do: [{"browser_mouse_click_xy", %{"x" => x, "y" => y, "button" => inputs["button"] || "left"}}]

  defp pointer("double_click", %{"x" => x, "y" => y}), do: [{"browser_mouse_click_xy", %{"x" => x, "y" => y, "clickCount" => 2}}]
  defp pointer("move", %{"x" => x, "y" => y}), do: [{"browser_mouse_move_xy", %{"x" => x, "y" => y}}]

  defp pointer("drag", %{"path" => [first | rest]}) do
    moves = for p <- rest, do: {"browser_mouse_move_xy", %{"x" => p["x"], "y" => p["y"]}}
    [{"browser_mouse_move_xy", %{"x" => first["x"], "y" => first["y"]}}, {"browser_mouse_down", %{}}] ++ moves ++ [{"browser_mouse_up", %{}}]
  end

  defp pointer("scroll", %{"x" => x, "y" => y, "direction" => direction} = inputs) do
    by = (inputs["amount"] || 5) * 100
    {dx, dy} = %{"up" => {0, -by}, "down" => {0, by}, "left" => {-by, 0}, "right" => {by, 0}}[direction]
    [{"browser_mouse_move_xy", %{"x" => x, "y" => y}}, {"browser_mouse_wheel", %{"deltaX" => dx, "deltaY" => dy}}]
  end

  defp pointer("key", %{"keys" => keys}), do: for(k <- String.split(keys), do: {"browser_press_key", %{"key" => key(k)}})
  defp pointer(action, _inputs), do: raise("#{action} needs more: Computer Use's tools say what each action takes")

  # A key as xdotool names it (ctrl+l, Return, BackSpace) as Playwright does (Control+l, Enter, Backspace).
  @keys %{
    "ctrl" => "Control",
    "control" => "Control",
    "alt" => "Alt",
    "shift" => "Shift",
    "super" => "Meta",
    "meta" => "Meta",
    "return" => "Enter",
    "enter" => "Enter",
    "backspace" => "Backspace",
    "tab" => "Tab",
    "escape" => "Escape",
    "delete" => "Delete",
    "space" => "Space",
    "left" => "ArrowLeft",
    "right" => "ArrowRight",
    "up" => "ArrowUp",
    "down" => "ArrowDown",
    "home" => "Home",
    "end" => "End",
    "prior" => "PageUp",
    "page_up" => "PageUp",
    "next" => "PageDown",
    "page_down" => "PageDown"
  }
  defp key(keys), do: keys |> String.split("+") |> Enum.map_join("+", &(@keys[String.downcase(&1)] || &1))

  defp did("click", %{"x" => x, "y" => y} = inputs), do: "Clicked #{inputs["button"] || "left"} at #{x}, #{y}"
  defp did("double_click", %{"x" => x, "y" => y}), do: "Double-clicked at #{x}, #{y}"
  defp did("move", %{"x" => x, "y" => y}), do: "Moved to #{x}, #{y}"
  defp did("drag", %{"path" => path}), do: "Dragged through #{length(path)} points"
  defp did("scroll", %{"direction" => direction} = inputs), do: "Scrolled #{direction} #{inputs["amount"] || 5}"
  defp did("key", %{"keys" => keys}), do: "Pressed #{keys}"

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
          %{"result" => %{"content" => content} = result} -> answer(content, result["isError"])
          %{"error" => %{"message" => why}} -> raise why
        end
    end
  end

  defp answer(content, error?) do
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

  # The browsers whoever this call is for may use: their containers of Chrome's image.
  defp containers(call), do: Enum.filter(Call.tool(call, "containers", "list", %{}), &(&1["image"] == @image and String.starts_with?(&1["name"], "chrome-")))

  defp session(%{"name" => "chrome-" <> session}), do: session
end
