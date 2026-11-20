defmodule ComputerUse.Phone do
  @moduledoc """
  A phone's own screen, as Computer Use's: each Android phone whose app has answered is a screen, called what its
  person calls it where they let it in (sign-in's `devices`). A phone runs no server, so its app answers: a server
  asks it each action as a tool of the device's own (`OneCore.Server.ask_device/3`), which the app does through
  Android's accessibility service (`client/native`, and `ComputerUse.kt` in the Android app), once its person has
  turned it on in Settings → Accessibility. A phone whose app isn't running is woken as it's asked, by a push, so
  each one that answered once is kept in this app's own artifact (`phones/<device>`) and listed while it sleeps.
  It's the phone's person's: only they, or the network's owner, use it.
  """
  alias OneCore.{Artifact, Call, Hooks, Server}

  @doc """
  The phones the caller may use, each with what it is and its screenshot's size, awake or not, and why each device
  whose app is watching that didn't answer in time couldn't, by its name, all asked at once (`OneCore.Asks.each/3`).
  One that didn't answer is listed as it was kept, as one that's asleep is.
  """
  def screens(call) do
    kept = Map.new(Artifact.list(call.own, "phones/"), &{String.replace_prefix(&1, "phones/", ""), nil})
    devices = Map.keys(kept) |> Enum.concat(Server.devices()) |> Enum.uniq() |> Enum.filter(&may?(call, &1))
    {answers, failed} = OneCore.Asks.each(Enum.filter(Server.devices(), &(&1 in devices)), &ask(&1, "screen", %{}, false))

    awake =
      Map.new(answers, fn
        {device, {:ok, %{"structuredContent" => %{"name" => _} = screen}}} -> {device, kept(call, device, screen)}
        # A device that isn't a phone, whose app has no screen to give, or a phone whose app went away.
        {device, _} -> {device, nil}
      end)

    screens =
      for device <- devices, phone = awake[device] || asleep(call, device, kept), phone != nil do
        %{
          "name" => phone["name"],
          "description" =>
            "#{phone["name"]}'s own screen, " <>
              if(phone["tv"], do: "an Android TV, which its remote's D-pad drives: `key` Up, Down, Left and Right move the focus, and Return presses what has it", else: "an Android phone") <>
              ", as its person sees it" <>
              cond do
                awake[device] == nil -> "; its app isn't running, and it's woken as it's used"
                not phone["on"] -> "; its person hasn't turned on the One App in Settings → Accessibility yet"
                true -> ""
              end,
          "width" => phone["width"],
          "height" => phone["height"],
          "phone" => device
        }
      end

    {screens, Map.new(failed, fn {device, why} -> {(asleep(call, device, kept) || %{})["name"] || device, why} end)}
  end

  def use(_call, _phone, "stream", _inputs), do: raise("A phone's screen has no live video yet: it's shown a picture at a time.")

  def use(call, phone, action, inputs) do
    if not may?(call, phone), do: raise("You may not use that phone's screen: only its person and the network's owner may.")

    case ask(phone, action, inputs, true) do
      {:ok, %{"isError" => true, "content" => content}} -> raise Enum.map_join(content, "\n", & &1["text"])
      {:ok, %{"content" => [%{"type" => "image", "data" => data}]}} -> {:blob, "image/png", Base.decode64!(data)}
      {:ok, %{"structuredContent" => %{"done" => done}}} -> %{done: done}
      {:error, why} -> raise why
    end
  end

  # Only the phone's person, or the network's owner, uses its screen.
  defp may?(call, phone),
    do: Hooks.whose(phone) == call.subject or Call.owns?(%{call | subject: "network"})

  # A phone that answered, as it's kept: its size and whether Computer Use is on, under the name its person let it in
  # by, or else the name it gives itself. It's written again only when that changed.
  defp kept(call, device, screen) do
    blob = "phones/" <> device

    Artifact.update(call.own, blob, fn stored ->
      was = if stored, do: JSON.decode!(stored), else: %{}
      named = was["named"] || sign_in_name(call, device)
      now = %{"named" => named, "name" => named || screen["name"], "width" => screen["width"], "height" => screen["height"], "on" => screen["on"], "tv" => screen["tv"]}
      if now == was, do: stored, else: JSON.encode!(now)
    end)
    |> JSON.decode!()
  end

  # A phone kept from when it last answered, while it's still its person's: one that's nobody's now is forgotten.
  defp asleep(call, device, kept) do
    cond do
      not is_map_key(kept, device) ->
        nil

      Hooks.whose(device) == nil ->
        Artifact.remove(call.own, "phones/" <> device)
        nil

      true ->
        case Artifact.read(call.own, "phones/" <> device) do
          {:ok, phone, _tag} -> JSON.decode!(phone)
          :absent -> nil
        end
    end
  end

  defp sign_in_name(call, device) do
    Call.app(call, "sign-in").device_name(call, device)
  rescue
    # Sign-in couldn't say now: it's asked again the next time the phone answers.
    _ -> nil
  end

  # Asks the phone's app to do `action` on its screen, as Computer Use's tool of the device's own: a phone that's
  # asleep is woken first where `wake`, which listing screens doesn't do.
  defp ask(phone, action, inputs, wake) do
    request = %{jsonrpc: "2.0", id: 1, method: "tools/call", params: %{name: "computer-use_" <> action, arguments: inputs}}
    message = JSON.encode!(request)

    case Server.ask_device(phone, message, wake: wake) do
      {:ok, answer} ->
        case JSON.decode(answer) do
          {:ok, %{"result" => result}} -> {:ok, result}
          {:ok, %{"error" => %{"message" => why}}} -> {:error, why}
          _ -> {:error, "The phone's app answered what isn't a tool's result"}
        end

      :unreachable ->
        {:error,
         "The phone didn't answer, and didn't wake when asked: it's woken by a push once the One App has been opened " <>
           "on it, so its person has it on, online, and with the One App allowed to notify them."}

      :stopped ->
        {:error, "The phone's app went away before it said it had done that, so it may or may not have. Take a screenshot to see."}
    end
  end
end
