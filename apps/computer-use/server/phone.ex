defmodule ComputerUse.Phone do
  @moduledoc """
  A phone's own screen, as Computer Use's: each Android phone whose app is running, and so watching its servers, is a
  screen, called what the phone is called. A phone runs no server, so its app answers: a server asks it each action
  as a tool of the device's own (`OneCore.Server.ask_device/2`), which the app does through Android's accessibility
  service (`client/native`, and `ComputerUse.kt` in the Android app), once its person has turned it on in Settings →
  Accessibility. It's the phone's person's: only they, or the network's owner, use it.
  """
  alias OneCore.{Hooks, Server}

  @doc "The phones the caller may use whose app answers now, each with what it is and its screenshot's size."
  def screens(call) do
    Server.devices()
    |> Enum.filter(&may?(call, &1))
    |> Task.async_stream(&{&1, ask(&1, "screen", %{})}, timeout: :infinity, ordered: false)
    |> Enum.flat_map(fn
      {:ok, {device, {:ok, %{"structuredContent" => %{"name" => name} = screen}}}} ->
        [
          %{
            "name" => name,
            "description" =>
              "#{name}'s own screen, an Android phone, as its person sees it" <>
                if(screen["on"], do: "", else: "; its person hasn't turned on The One App in Settings → Accessibility yet"),
            "width" => screen["width"],
            "height" => screen["height"],
            "phone" => device
          }
        ]

      # A device that isn't a phone, whose app has no screen to give, or a phone whose app went away.
      _ ->
        []
    end)
  end

  def use(_call, _phone, "stream", _inputs), do: raise("A phone's screen has no live video yet: it's shown a picture at a time.")

  def use(call, phone, action, inputs) do
    if not may?(call, phone), do: raise("You may not use that phone's screen: only its person and the network's owner may.")

    case ask(phone, action, inputs) do
      {:ok, %{"isError" => true, "content" => content}} -> raise Enum.map_join(content, "\n", & &1["text"])
      {:ok, %{"content" => [%{"type" => "image", "data" => data}]}} -> {:blob, "image/png", Base.decode64!(data)}
      {:ok, %{"structuredContent" => %{"done" => done}}} -> %{done: done}
      {:error, why} -> raise why
    end
  end

  # Only the phone's person, or the network's owner, uses its screen.
  defp may?(call, phone),
    do: Hooks.whose(phone) == call.subject or OneCore.Call.owns?(%{call | subject: "network"})

  # Asks the phone's app to do `action` on its screen, as Computer Use's tool of the device's own.
  defp ask(phone, action, inputs) do
    request = %{jsonrpc: "2.0", id: 1, method: "tools/call", params: %{name: "computer-use_" <> action, arguments: inputs}}

    case Server.ask_device(phone, JSON.encode!(request)) do
      {:ok, answer} ->
        case JSON.decode(answer) do
          {:ok, %{"result" => result}} -> {:ok, result}
          {:ok, %{"error" => %{"message" => why}}} -> {:error, why}
          _ -> {:error, "The phone's app answered what isn't a tool's result"}
        end

      :unreachable ->
        {:error, "The phone's app isn't running, or didn't answer: it's used while The One App is open on it, or was lately."}

      :stopped ->
        {:error, "The phone's app went away before it said it had done that, so it may or may not have. Take a screenshot to see."}
    end
  end
end
