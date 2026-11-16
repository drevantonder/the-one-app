defmodule ComputerUse do
  @moduledoc """
  Computer Use: an agent looks at a screen and acts on it, and its person watches.

  It knows no kind of screen. It offers the hook `screens`, which a plugin with screens hooks into
  (`computer-use.screens`): a hook that changes, answered with the plugin's own name and its list of the screens
  the caller may use, so lists from many plugins sit side by side and none changes another's. Each screen there says
  which of that plugin's tools acts on it and with what inputs; acting is a call of that tool, as the same caller,
  with the action's inputs added. So who may use a screen is the plugin's to decide, and nothing here checks it again.

  What each person's screens were last used for, by whom, is kept in this plugin's own artifact, for the app.
  """
  @behaviour OneCore.Plugin
  alias OneCore.{Artifact, Call}

  @actions ~w(screenshot click double_click move drag scroll type key)
  # How often what a screen was last used for is written again while it's the same: the app says "2 min ago".
  @written_every 60_000

  @screen %{type: "string", description: "the screen, as `screens` names it"}
  @doing %{type: "string", description: "what you're doing on it, in a few words, for whoever watches"}
  @at %{
    x: %{type: "integer", minimum: 0, description: "pixels from the left of the screenshot"},
    y: %{type: "integer", minimum: 0, description: "pixels from the top of the screenshot"}
  }

  @impl true
  def offers do
    [
      %{
        name: "screens",
        kind: :mutate,
        description:
          "The screens the caller may use. It's about nothing. A plugin with screens answers with its own name, `%{call.plugin => screens}`, so that each plugin's list is its own: each screen a map of its `name`, what it is (`description`, a few words), its `width` and `height` in pixels, and how it's acted on: the plugin's `tool` and the `inputs` that name the screen to it. That tool is then called as the caller, with those inputs and an `action`: `screenshot` (answered with a PNG as `{:blob, \"image/png\", data}`), `click` (x, y, button: left, right or middle), `double_click` (x, y), `move` (x, y), `drag` (path: a list of {x, y}), `scroll` (x, y, direction: up, down, left or right, amount: steps), `type` (text) or `key` (keys, as xdotool names them: ctrl+l, Return; space between presses one after another). It answers anything but a screenshot in a few words."
      }
    ]
  end

  @impl true
  def apps, do: [%{name: "Computer Use", page: "client/index.html", icon: "client/icon.svg"}]

  @impl true
  def tools do
    [
      %{
        name: "screens",
        read_only: true,
        description:
          "The screens you can use: each one's name, what it is, its width and height in pixels, the plugin it's from, and what it was last used for, by whom, when (ms since 1970).",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "screenshot",
        read_only: true,
        description: "A picture of the screen as it is now. Its pixels are what the other tools' x and y count.",
        inputs: act()
      },
      %{name: "click", description: "Click at a point.", inputs: act(Map.put(@at, :button, %{type: "string", enum: ~w(left right middle), description: "default left"}), ["x", "y"])},
      %{name: "double_click", description: "Double-click at a point.", inputs: act(@at, ["x", "y"])},
      %{name: "move", description: "Move the pointer to a point, as hovering does.", inputs: act(@at, ["x", "y"])},
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
          )
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
          )
      },
      %{name: "type", description: "Type text where the focus is.", inputs: act(%{text: %{type: "string"}}, ["text"])},
      %{
        name: "key",
        description: "Press keys: ctrl+l, Return, ctrl+shift+t, BackSpace; several, space between, one after another.",
        inputs: act(%{keys: %{type: "string", description: "the keys, as xdotool names them"}}, ["keys"])
      },
      %{
        name: "wait",
        read_only: true,
        description: "Wait for the screen to settle, then give a screenshot.",
        inputs: act(%{seconds: %{type: "number", minimum: 0.1, maximum: 10, description: "default 1"}})
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
    screens = screens(call)
    used = used(call, screens)

    for s <- screens do
      Map.merge(Map.take(s, ~w(name description width height plugin)), %{"used" => used[key(call, s)]})
    end
  end

  def call("wait", inputs, call) do
    Process.sleep(round(Map.get(inputs, "seconds", 1) * 1000))
    call("screenshot", inputs, call)
  end

  def call(action, inputs, call) when action in @actions do
    s = screen(call, inputs["screen"])
    if action != "screenshot" or inputs["doing"], do: note(call, s, inputs["doing"])
    asked = Map.drop(inputs, ~w(screen doing)) |> Map.merge(s["inputs"]) |> Map.put("action", action)
    Call.tool(call, s["plugin"], s["tool"], asked)
  end

  # Every screen the caller may use, each with the plugin it's from.
  defp screens(call) do
    case Call.run_hook(call, "screens", %{}) do
      {:ok, lists, by} -> for {name, list} <- lists, s <- List.wrap(list), do: Map.put(s, "plugin", by[name])
      {:refuse, why} -> raise why
    end
  end

  # A screen by its name, or by its plugin's name and its own (`containers/proof`) where two are called the same.
  defp screen(call, name) do
    case Enum.filter(screens(call), &(&1["name"] == name or "#{&1["plugin"]}/#{&1["name"]}" == name)) do
      [s] -> s
      [] -> raise "You have no screen called #{name}: `screens` lists yours"
      two -> raise "#{Enum.map_join(two, " and ", &"#{&1["plugin"]}/#{name}")} are both called #{name}: name one so"
    end
  end

  # What a person's screen was last used for is theirs, so it's kept by whom it's for and the screen.
  defp key(call, s), do: "used/" <> Base.url_encode64(:crypto.hash(:sha256, "#{call.subject} #{s["plugin"]} #{s["name"]}"), padding: false)

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
