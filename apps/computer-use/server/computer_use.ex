defmodule ComputerUse do
  @moduledoc """
  Computer Use: an agent looks at a screen and acts on it, and its person watches.

  Its own screens are computers' own (`ComputerUse.Desktop`, its service on a Mac, Linux or Windows desktop), and
  Android phones' own, which their app answers for (`ComputerUse.Phone`). Other apps bring theirs: it offers the hook `screens`, which an app with screens hooks into
  (`computer-use.screens`): a hook that collects, answered with the app's list of the screens the caller may
  use, so lists from many apps sit side by side, and one app that can't answer takes away only its own. Each screen there says
  which of that app's tools acts on it and with what inputs; acting is a call of that tool, as the same caller,
  with the action's inputs added. So who may use a screen is the app's to decide, and nothing here checks it again.

  What each person's screens were last used for, by whom, is kept in this app's own artifact, for the app.
  """
  @behaviour OneCore.App
  alias OneCore.{Artifact, Call}
  alias ComputerUse.{Desktop, Phone}

  @actions ~w(screenshot windows elements click double_click move drag scroll type key set_value)
  # What only looks, which isn't noted as a use of the screen unless it says what it's for.
  @looks ~w(screenshot windows elements)
  # What aims an action at one window or element of a computer's own screen, as Cua Driver names it.
  @aimed ~w(pid window_id element_token)
  # How often what a screen was last used for is written again while it's the same: the app says "2 min ago".
  @written_every 60_000

  @screen %{type: "string", description: "the screen, as `screens` names it"}
  @doing %{type: "string", description: "what you're doing on it, in a few words, for whoever watches"}
  # What a screen's tool answers: a screenshot, what it did in a few words, or what a stream connects with.
  @screenshot %{
    type: "object",
    description: "The screenshot, a PNG: an agent sees it",
    properties: %{mimeType: %{type: "string"}, size: %{type: "integer"}},
    required: ["mimeType"]
  }
  @done %{
    type: "object",
    properties: %{
      done: %{type: "string", description: "What it did, in a few words"},
      effect: %{type: "string", description: "On a computer's own screen, what Cua Driver saw of it: confirmed, unverifiable or suspected_noop"},
      escalation: %{type: "object", description: "On a computer's own screen, what Cua Driver recommends when it didn't land"}
    },
    required: ["done"]
  }
  @stream %{
    type: "object",
    properties: %{
      iceServers: %{type: "array", items: %{type: "object"}, description: "With no offer: as RTCPeerConnection takes them"},
      answer: %{type: "string", description: "With an offer: the screen's SDP answer"}
    }
  }

  @at %{
    x: %{type: "integer", minimum: 0, description: "pixels from the left of the screenshot"},
    y: %{type: "integer", minimum: 0, description: "pixels from the top of the screenshot"}
  }

  # A window of a computer's own screen, from `windows`, or an element of one, from `elements`, which an action aimed
  # at it reaches in the background; x and y are then pixels of that window's own screenshot.
  @window %{
    pid: %{type: "integer", description: "a computer's own screen: the process whose window it is, from `windows`"},
    window_id: %{type: "integer", description: "a computer's own screen: the window, from `windows`, acted on in the background"}
  }
  @element %{
    element_token: %{type: "string", description: "a computer's own screen: an element of a window, `<snapshot_id>:N` from `elements`, acted on in the background"}
  }

  # Its data is everyone's together: which screen was last used by whom, kept by who used it.
  @impl true
  def own, do: :installed

  @impl true
  def icon, do: "client/icon.svg"

  @impl true
  def requires, do: %{"core" => ">= 0.32.111", "sign-in" => "to name a phone as its person knows it", "webrtc" => "to show a computer's screen live"}

  @impl true
  def offers do
    [
      %{
        name: "screens",
        kind: :collect,
        description:
          "The screens the caller may use. It's about nothing. An app with screens answers with its list of them: each screen a map of its `name`, what it is (`description`, a few words), its `width` and `height` in pixels, and how it's acted on: the app's `tool` and the `inputs` that name the screen to it. That tool is then called as the caller, with those inputs and an `action`: `screenshot` (answered with a PNG as `{:blob, \"image/png\", data}`), `click` (x, y, button: left, right or middle), `double_click` (x, y), `move` (x, y), `drag` (path: a list of {x, y}), `scroll` (x, y, direction: up, down, left or right, amount: steps), `type` (text) or `key` (keys, as xdotool names them: ctrl+l, Return; space between presses one after another). It answers anything but a screenshot as `%{done: ...}`, what it did in a few words. A screen that a person's app can show as live video over WebRTC takes `stream` too: with no `offer`, it answers the ICE servers to connect with (WebRTC's `ice_servers`); with the app's whole SDP `offer`, `%{\"answer\" => sdp}`."
      }
    ]
  end

  @impl true
  def entrypoints,
    do: [
      %{
        page: "client/index.html",
        subject: "user",
        runtime_version: 4
      }
    ]

  @impl true
  def tools do
    [
      %{
        name: "screens",
        read_only: true,
        description:
          "The screens you can use, as `screens`: each one's name, what it is, its width and height in pixels, the app it's from, and what it was last used for, by whom, when (ms since 1970). An app that couldn't list its screens is in `failed`, with why.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            screens: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: %{type: "string"},
                  description: %{type: "string", description: "What it is, in a few words"},
                  width: %{type: "integer"},
                  height: %{type: "integer"},
                  app: %{type: "string", description: "The app it's from"},
                  used: %{
                    type: ["object", "null"],
                    description: "What it was last used for",
                    properties: %{
                      by: %{type: ["string", "null"]},
                      doing: %{type: ["string", "null"]},
                      at: %{type: "integer", description: "When, in milliseconds since 1970"}
                    }
                  }
                },
                required: ["name", "description", "width", "height", "app", "used"]
              }
            },
            failed: %{
              type: "object",
              additionalProperties: %{type: "string"},
              description: "Why each app that couldn't list its screens couldn't, by its name"
            }
          },
          required: ["screens", "failed"]
        }
      },
      %{
        name: "screenshot",
        read_only: true,
        description:
          "A picture of the screen as it is now, or with `pid` and `window_id` of one window of a computer's own screen, taken in the background as it is, behind whatever is in front. Its pixels are what the other tools' x and y count.",
        inputs: act(@window),
        outputs: @screenshot
      },
      %{
        name: "windows",
        read_only: true,
        description:
          "A computer's own screen's windows, as Cua Driver's `list_windows` gives them: each one's `window_id`, `pid`, `app_name`, `title`, `bounds` and `z_index` (the highest is in front). Give `app` or `pid`: without either it lists every window on the computer.",
        inputs:
          act(%{
            app: %{type: "string", description: "only the windows of apps whose name has this in it"},
            pid: %{type: "integer", description: "only this process's windows"},
            on_screen_only: %{type: "boolean", description: "only windows on the screen now"}
          }),
        outputs: %{type: "object", properties: %{windows: %{type: "array", items: %{type: "object"}}}, required: ["windows"]}
      },
      %{
        name: "elements",
        read_only: true,
        description:
          "One window's elements, read through accessibility in the background, as Cua Driver's `get_window_state` gives them: `tree_markdown`, a row `[N]` for each element you can act on, whose `element_token` is `<snapshot_id>:N`. A new read of the window replaces the last, and its tokens. Narrow a large window with `query`.",
        inputs:
          act(
            Map.merge(@window, %{
              query: %{type: "string", description: "only rows with this in them, and what they're in; `a|b` for either"},
              max_elements: %{type: "integer", minimum: 1, description: "how many elements to read at most; default 250"}
            }),
            ["pid", "window_id"]
          ),
        outputs: %{
          type: "object",
          properties: %{snapshot_id: %{type: "string"}, tree_markdown: %{type: "string"}, truncated: %{type: "boolean"}},
          required: ["snapshot_id"]
        }
      },
      %{
        name: "click",
        description: "Click at a point, or on an element of a window (`element_token`) in the background.",
        inputs:
          act(
            @at
            |> Map.merge(@window)
            |> Map.merge(@element)
            |> Map.put(:button, %{type: "string", enum: ~w(left right middle), description: "default left"})
          ),
        outputs: @done
      },
      %{
        name: "double_click",
        description: "Double-click at a point, or on an element of a window (`element_token`) in the background.",
        inputs: act(@at |> Map.merge(@window) |> Map.merge(@element)),
        outputs: @done
      },
      %{name: "move", description: "Move the pointer to a point, as hovering does.", inputs: act(@at, ["x", "y"]), outputs: @done},
      %{
        name: "drag",
        description: "Press at the first point, move through the rest and let go at the last.",
        inputs:
          act(
            %{
              path: %{
                type: "array",
                minItems: 2,
                items: %{type: "object", required: ["x", "y"], properties: @at},
                description: "the points, at least two"
              }
            }
            |> Map.merge(@window),
            ["path"]
          ),
        outputs: @done
      },
      %{
        name: "scroll",
        description: "Scroll at a point.",
        inputs:
          act(
            @at
            |> Map.merge(@window)
            |> Map.merge(@element)
            |> Map.merge(%{
              direction: %{type: "string", enum: ~w(up down left right)},
              amount: %{type: "integer", minimum: 1, maximum: 50, description: "steps of the wheel; default 5"}
            }),
            ["direction"]
          ),
        outputs: @done
      },
      %{
        name: "type",
        description: "Type text where the focus is, or into a window or an element of it (`element_token`) in the background.",
        inputs: act(%{text: %{type: "string"}} |> Map.merge(@window) |> Map.merge(@element), ["text"]),
        outputs: @done
      },
      %{
        name: "key",
        description:
          "Press keys: ctrl+l, Return, ctrl+shift+t, BackSpace; several, space between, one after another. Where the focus is, or to a window or an element of it (`element_token`) in the background.",
        inputs: act(%{keys: %{type: "string", description: "the keys, as xdotool names them"}} |> Map.merge(@window) |> Map.merge(@element), ["keys"]),
        outputs: @done
      },
      %{
        name: "set_value",
        description:
          "Set an element's value in the background, as Cua Driver's `set_value` does: a text field's text, a slider's or a date's value, or the option of a pop-up menu whose title is `value`.",
        inputs: act(Map.merge(@element, %{value: %{type: "string"}}), ["element_token", "value"]),
        outputs: @done
      },
      %{
        name: "stream",
        description:
          "For a person's app that shows a screen as live video over WebRTC, not for agents: with no offer, the ICE servers to connect with; with the app's whole SDP offer, the screen's answer.",
        inputs: %{
          type: "object",
          required: ["screen"],
          properties: %{screen: @screen, offer: %{type: "string", description: "the app's SDP offer, with its candidates"}}
        },
        outputs: @stream
      },
      %{
        name: "wait",
        read_only: true,
        description: "Wait for the screen to settle, then give a screenshot.",
        inputs: act(%{seconds: %{type: "number", minimum: 0.1, maximum: 10, description: "default 1"}}),
        outputs: @screenshot
      }
    ]
  end

  defp act(properties, required \\ []) do
    %{
      type: "object",
      required: ["screen" | required],
      properties: Map.merge(properties, %{screen: @screen, doing: @doing})
    }
  end

  @impl true
  def call("screens", _inputs, call) do
    {screens, failed} = screens(call)
    used = used(call, screens)

    %{
      "screens" => for(s <- screens, do: Map.merge(Map.take(s, ~w(name description width height app)), %{"used" => used[key(call, s)]})),
      "failed" => failed
    }
  end

  def call("wait", inputs, call) do
    Process.sleep(round(Map.get(inputs, "seconds", 1) * 1000))
    call("screenshot", inputs, call)
  end

  # Watching a screen is no use of it, so it isn't noted.
  def call("stream", inputs, call) do
    s = screen(call, inputs["screen"])

    cond do
      s["desktop"] -> Desktop.use(call, s["desktop"], "stream", inputs)
      s["phone"] -> Phone.use(call, s["phone"], "stream", inputs)
      true -> Call.tool(call, s["app"], s["tool"], inputs |> Map.delete("screen") |> Map.merge(s["inputs"]) |> Map.put("action", "stream"))
    end
  end

  def call(action, inputs, call) when action in @actions do
    s = screen(call, inputs["screen"])
    if action not in @looks or inputs["doing"], do: note(call, s, inputs["doing"])
    inputs = Map.drop(inputs, ~w(screen doing))

    cond do
      s["desktop"] -> Desktop.use(call, s["desktop"], action, inputs)
      action in ~w(windows elements set_value) or Enum.any?(@aimed, &Map.has_key?(inputs, &1)) -> raise "Only a computer's own screen has windows and elements to aim at: #{s["name"]} is used whole"
      action in ~w(click double_click scroll) and not (Map.has_key?(inputs, "x") and Map.has_key?(inputs, "y")) -> raise "Give x and y: on #{s["name"]} it's at a point"
      s["phone"] -> Phone.use(call, s["phone"], action, inputs)
      true -> Call.tool(call, s["app"], s["tool"], inputs |> Map.merge(s["inputs"]) |> Map.put("action", action))
    end
  end

  # Every screen the caller may use, each with the app it's from, its own computers' and phones' first, and why each
  # app that couldn't say its own couldn't.
  defp screens(call) do
    {:ok, lists, failed} = Call.run_hook(call, "screens", %{})
    theirs = for {app, list} <- lists, s <- List.wrap(list), do: Map.put(s, "app", app)

    try do
      {Enum.map(Desktop.screens(call) ++ Phone.screens(call), &Map.put(&1, "app", "computer-use")) ++ theirs, failed}
    rescue
      problem -> {theirs, Map.put(failed, "computer-use", Exception.message(problem))}
    end
  end

  # A screen by its name, or by its app's name and its own (`containers/proof`) where two are called the same.
  defp screen(call, name) do
    {screens, failed} = screens(call)

    case Enum.filter(screens, &(&1["name"] == name or "#{&1["app"]}/#{&1["name"]}" == name)) do
      [s] -> s
      [] when failed == %{} -> raise "You have no screen called #{name}.\n\n`screens` lists yours."
      [] -> raise "You have no screen called #{name}, and #{Enum.map_join(failed, ", ", fn {app, why} -> "#{app} couldn't list its screens: #{why}" end)}"
      two -> raise "#{Enum.map_join(two, " and ", &"#{&1["app"]}/#{name}")} are both called #{name}: name one so"
    end
  end

  # What a person's screen was last used for is theirs, so it's kept by whom it's for and the screen.
  defp key(call, s), do: "used/" <> Base.url_encode64(:crypto.hash(:sha256, "#{call.subject} #{s["app"]} #{s["name"]}"), padding: false)

  defp used(_call, []), do: %{}

  defp used(call, screens) do
    keys = Enum.map(screens, &key(call, &1))

    for {key, {:ok, data, _tag}} <- Enum.zip(keys, Artifact.read_all(call.own, keys)), into: %{},
        do: {key, JSON.decode!(data)}
  end

  # Writes who uses a screen and for what, when that changed or was last written a while ago.
  defp note(call, s, doing) do
    key = key(call, s)
    now = System.system_time(:millisecond)

    case Artifact.read(call.own, key) do
      {:ok, data, tag} ->
        was = JSON.decode!(data)
        used = %{"by" => call.actor, "doing" => doing || if(was["by"] == call.actor, do: was["doing"]), "at" => now}

        if used["by"] != was["by"] or used["doing"] != was["doing"] or now - was["at"] > @written_every,
          do: Artifact.replace(call.own, key, JSON.encode!(used), tag)

      :absent ->
        Artifact.write(call.own, key, JSON.encode!(%{"by" => call.actor, "doing" => doing, "at" => now}))
    end
  end
end
