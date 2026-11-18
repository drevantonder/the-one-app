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

  @actions ~w(screenshot click double_click move drag scroll type key)
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
  @done %{type: "object", properties: %{done: %{type: "string", description: "What it did, in a few words"}}, required: ["done"]}
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

  # Its data is everyone's together: which screen was last used by whom, kept by who used it.
  @impl true
  def own, do: :installed

  @impl true
  def icon, do: "client/icon.svg"

  @impl true
  def requires, do: %{"core" => ">= 0.23.33", "sign-in" => "to name a phone as its person knows it"}

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
        subject: "user"
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
        description: "A picture of the screen as it is now. Its pixels are what the other tools' x and y count.",
        inputs: act(),
        outputs: @screenshot
      },
      %{
        name: "click",
        description: "Click at a point.",
        inputs: act(Map.put(@at, :button, %{type: "string", enum: ~w(left right middle), description: "default left"}), ["x", "y"]),
        outputs: @done
      },
      %{name: "double_click", description: "Double-click at a point.", inputs: act(@at, ["x", "y"]), outputs: @done},
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
            },
            ["path"]
          ),
        outputs: @done
      },
      %{
        name: "scroll",
        description: "Scroll at a point.",
        inputs:
          act(
            Map.merge(@at, %{
              direction: %{type: "string", enum: ~w(up down left right)},
              amount: %{type: "integer", minimum: 1, maximum: 50, description: "steps of the wheel; default 5"}
            }),
            ["x", "y", "direction"]
          ),
        outputs: @done
      },
      %{name: "type", description: "Type text where the focus is.", inputs: act(%{text: %{type: "string"}}, ["text"]), outputs: @done},
      %{
        name: "key",
        description: "Press keys: ctrl+l, Return, ctrl+shift+t, BackSpace; several, space between, one after another.",
        inputs: act(%{keys: %{type: "string", description: "the keys, as xdotool names them"}}, ["keys"]),
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

  defp act(properties \\ %{}, required \\ []) do
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
    if s["desktop"], do: Desktop.use(call, s["desktop"], "stream", inputs)
    if s["phone"], do: Phone.use(call, s["phone"], "stream", inputs)
    Call.tool(call, s["app"], s["tool"], inputs |> Map.delete("screen") |> Map.merge(s["inputs"]) |> Map.put("action", "stream"))
  end

  def call(action, inputs, call) when action in @actions do
    s = screen(call, inputs["screen"])
    if action != "screenshot" or inputs["doing"], do: note(call, s, inputs["doing"])
    inputs = Map.drop(inputs, ~w(screen doing))

    cond do
      s["desktop"] -> Desktop.use(call, s["desktop"], action, inputs)
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
