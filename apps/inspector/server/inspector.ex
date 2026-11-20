defmodule Inspector do
  @moduledoc """
  Inspector: an agent reads an app's native window on any of its person's devices and drives it, as a test would. A
  native window on a computer serves the inspector's protocol (`clients/app/renderer/src/inspect.rs`, which documents
  it), and the device's app answers each of this app's tools for it, as a tool of the device's own
  (`OneCore.Server.ask_device/3`): `windows` lists them, and each other tool is that protocol's op of the same name,
  its inputs passed on as they are. A device is its person's: only they, or the network's owner, inspect it.
  """
  @behaviour OneCore.App
  alias OneCore.{Call, Hooks, Server}

  @impl true
  def icon, do: "icon.svg"

  @impl true
  def requires, do: %{"core" => ">= 0.25.4"}

  @device %{type: "string", description: "the device, by its key or name, as `windows` lists it; the only one with a native window open if not given"}
  @window %{type: "integer", description: "the window, as `windows` lists it; the device's only native window if not given"}
  @selector %{
    node: %{type: "integer", description: "a node by its id in the tree"},
    role: %{type: "string", description: "ARIA's role: button, searchbox, heading, listitem"},
    name: %{type: "string", description: "its accessible name, exactly"},
    text: %{type: "string", description: "words in its accessible name, any case"},
    key: %{type: "string", description: "its id prop, as the DOM's id names an element"},
    nth: %{type: "integer", minimum: 0, description: "the nth match, from 0, rather than the first"}
  }
  @point %{
    x: %{type: "number", description: "CSS pixels from the window's left, as rects are"},
    y: %{type: "number", description: "CSS pixels from the window's top"}
  }
  @result %{type: "object"}

  @impl true
  def tools do
    [
      %{
        name: "windows",
        read_only: true,
        description:
          "The app windows, drawn natively, open on your devices, each with its device, `window`, the entrypoint it shows, its size and its clock. A device whose app isn't running has none.",
        inputs: %{type: "object", properties: %{device: @device}},
        outputs: %{type: "object", properties: %{windows: %{type: "array", items: %{type: "object"}}}, required: ["windows"]}
      },
      op("tree", true, "The window's semantic tree, as a screen reader reads it: each node's id, role, name, value, states, actions and rect.", %{}),
      op(
        "query",
        true,
        "The nodes that match a selector, each with its rect, the style and class it was given, its colours and lengths as the design tokens resolve them, and the source file:line of the JSX that made it where its build says.",
        @selector
      ),
      %{
        name: "snap",
        read_only: true,
        description: "A PNG of the window as drawn.",
        inputs: %{type: "object", properties: %{device: @device, window: @window}},
        outputs: %{type: "object", properties: %{mimeType: %{type: "string"}, size: %{type: "integer"}}, required: ["mimeType"]}
      },
      %{
        name: "profile",
        read_only: false,
        description:
          "Steps what moves on the paused clock and draws the frame at each time in `at` (ms from now: 0, 80, 200, 400 unless given), with a picture of each and what it cost: the JS mutations applied since the frame before, layout and paint time, draw calls and the glyph atlas's use. For checking an animation or a slow screen; the clock stays paused.",
        inputs: %{
          type: "object",
          properties: %{
            at: %{type: "array", items: %{type: "number", minimum: 0}, description: "ms from now, in order"},
            snap: %{type: "boolean", description: "false for no pictures"},
            device: @device,
            window: @window
          }
        },
        outputs: %{
          type: "object",
          properties: %{
            frames: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  at: %{type: "number", description: "ms from when it started"},
                  mutations: %{type: "integer", description: "the JS's mutations applied since the frame before"},
                  layout_ms: %{type: "number", description: "laying out its tree"},
                  paint_ms: %{type: "number", description: "styling, laying out and painting it, all told"},
                  draws: %{type: "integer", description: "its GPU draw calls"},
                  work: %{type: "object", description: "what its GPU did, the same on any GPU: the device pixels of quads, shadows, sprites, backdrops and rasters it drew, as often as each is drawn over, the pixels it copied of the frames before and the bytes it uploaded"},
                  atlas: %{type: "object", description: "the glyph and picture atlas: its tiles, textures, bytes on the GPU, and how much of them is filled, 0 to 1"},
                  width: %{type: "integer"},
                  height: %{type: "integer"}
                }
              }
            },
            time: %{type: "number", description: "the clock's time after, in ms"},
            paused: %{type: "boolean"}
          },
          required: ["frames"]
        }
      },
      op(
        "replay",
        false,
        "Plays a press, from a node by a selector or a point, along `moves` (each {dx, dy, ms}; one with neither holds still) on the paused clock, drawing every frame of a display `hz` (60) times a second until `after` ms (2000) past its end or nothing moves, as a person's swipe, fling, drag or pull through the window's own gesture and spring code. Gives each frame's cost and work, where `track` was drawn (x, y) and how far what scrolls under the press shows scrolled, its pictures with `snap`, and how many frames went `over` their budget.",
        Map.merge(Map.merge(@selector, @point), %{
          moves: %{type: "array", items: %{type: "object", properties: %{dx: %{type: "number"}, dy: %{type: "number"}, ms: %{type: "number", minimum: 0}}, required: ["ms"]}},
          lift: %{type: "boolean", description: "false to hold it down at the end"},
          hz: %{type: "number", minimum: 1, description: "the display's frames a second: 60 or 120"},
          input_hz: %{type: "number", minimum: 1, description: "its moves a second, hz unless given"},
          after: %{type: "number", minimum: 0},
          track: %{type: "object", description: "a selector of a node to follow, as `query` takes one"},
          snap: %{type: "boolean", description: "true for each frame's picture"}
        }),
        ["moves"]
      ),
      op(
        "host",
        true,
        "Whether the device's host keeps the renderer's contract with it: the window is kept moving for a second, then still, and it says what held: a tick each display refresh, a frame presented each tick, nothing while still, and inputs with the device's own times, in order (move the pointer or a finger over it as it runs).",
        %{}
      ),
      op("tap", false, "Presses a node, by a selector, or what's at a point, as a finger does: an input takes the keyboard, and what listens for click gets one. Gives `tapped`, the node that took it.", Map.merge(@selector, @point)),
      op("type", false, "Types text into an input, by a selector, or the one with the keyboard, a character at a time. Gives the node and its `value`.", Map.put(@selector, :text, %{type: "string", description: "what to type; with a selector, `text` is what's typed, so pick the input by `role`, `name`, `key` or `node`"}), ["text"]),
      op(
        "key",
        false,
        "Presses a key where the keyboard is, as the DOM names it: Enter, Escape, Tab, Backspace, a. Tab moves the keyboard; Enter and Space press what has it.",
        %{key: %{type: "string"}, shift: %{type: "boolean"}, alt: %{type: "boolean"}, ctrl: %{type: "boolean"}, meta: %{type: "boolean"}},
        ["key"]
      ),
      op("scroll", false, "Scrolls the window by dx and dy CSS pixels, down and right positive.", %{dx: %{type: "number"}, dy: %{type: "number"}}),
      op(
        "drag",
        false,
        "Drags from a node, by a selector, or a point, by dx and dy CSS pixels: the pointer goes down, moves in `steps` (10) even moves and comes up. With the clock paused, it's stepped `ms` (200) over them, so what's let go has their speed.",
        Map.merge(Map.merge(@selector, @point), %{dx: %{type: "number"}, dy: %{type: "number"}, steps: %{type: "integer", minimum: 1}, ms: %{type: "number", minimum: 0}})
      ),
      op(
        "clock",
        false,
        "The app's clock, its Date, timers and the window's animations: `pause` true or false, `step` ms on, running each timer due at its own time, or `seek` to ms since it started. Gives its `time` and whether it's `paused`.",
        %{pause: %{type: "boolean"}, step: %{type: "number", minimum: 0}, seek: %{type: "number", minimum: 0}}
      ),
      op(
        "settle",
        true,
        "Waits until nothing's pending that ends by itself, or `deadline` ms (5000), and says what still is: requests the app asked that aren't answered, timers (an interval never ends, so isn't waited for), animations.",
        %{deadline: %{type: "number", minimum: 0}}
      )
    ]
  end

  defp op(name, read_only, description, properties, required \\ []) do
    %{
      name: name,
      read_only: read_only,
      description: description,
      inputs: %{type: "object", required: required, properties: Map.merge(properties, %{device: @device, window: @window})},
      outputs: @result
    }
  end

  @impl true
  def call("windows", inputs, call) do
    devices = if inputs["device"], do: [device(call, inputs["device"])], else: mine(call)

    windows =
      devices
      |> Task.async_stream(&{&1, ask(&1, "windows", %{})}, timeout: :infinity, ordered: false)
      |> Enum.flat_map(fn
        {:ok, {device, {:ok, %{"structuredContent" => %{"windows" => windows} = listed}}}} ->
          for window <- windows, do: Map.merge(window, %{"device" => device, "name" => listed["device"]})

        # A device whose app has no native windows to give: a phone, or one on a build before them.
        {:ok, _} ->
          []
      end)

    %{"windows" => windows}
  end

  def call(tool, inputs, call) do
    device =
      case inputs["device"] do
        nil -> only(call)
        device -> device(call, device)
      end

    case ask(device, tool, Map.delete(inputs, "device")) do
      {:ok, %{"isError" => true, "content" => content}} -> raise Enum.map_join(content, "\n", & &1["text"])
      {:ok, %{"content" => [%{"type" => "image", "data" => data}]}} -> {:blob, "image/png", Base.decode64!(data)}
      # What it says, with each frame's picture after it, as an agent sees pictures.
      {:ok, %{"structuredContent" => result, "content" => [_ | pictures]}} when pictures != [] -> {:content, result, pictures}
      {:ok, %{"structuredContent" => result}} -> result
      {:error, why} -> raise why
    end
  end

  # The devices the caller may inspect, of those whose app is watching: their own, or any where they own the network.
  defp mine(call), do: Enum.filter(Server.devices(), &may?(call, &1))

  defp may?(call, device), do: Hooks.whose(device) == call.subject or Call.owns?(%{call | subject: "network"})

  # A device by its key, or by the name its app gives itself, which `windows` lists.
  defp device(call, named) do
    cond do
      named in mine(call) -> named
      true -> Enum.find_value(call("windows", %{}, call)["windows"], &(&1["name"] == named && &1["device"])) || raise("None of your devices called #{named} has a native window open.")
    end
  end

  # The one device with a native window open, when none is named.
  defp only(call) do
    case Enum.uniq(for w <- call("windows", %{}, call)["windows"], do: w["device"]) do
      [device] -> device
      [] -> raise "None of your devices has a native window open."
      _ -> raise "More than one of your devices has a native window open: say which, as `windows` lists them."
    end
  end

  defp ask(device, tool, inputs) do
    request = JSON.encode!(%{jsonrpc: "2.0", id: 1, method: "tools/call", params: %{name: "inspector_" <> tool, arguments: inputs}})

    case Server.ask_device(device, request, wake: false) do
      {:ok, answer} ->
        case JSON.decode(answer) do
          {:ok, %{"result" => result}} -> {:ok, result}
          {:ok, %{"error" => %{"message" => why}}} -> {:error, why}
          _ -> {:error, "The device's app answered what isn't a tool's result"}
        end

      :unreachable ->
        {:error, "The device's app isn't running, or didn't answer."}

      :stopped ->
        {:error, "The device's app went away before it answered, so it may or may not have done it."}
    end
  end
end
