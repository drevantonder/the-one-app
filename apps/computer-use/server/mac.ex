defmodule ComputerUse.Mac do
  @moduledoc """
  A Mac's own screen, as Computer Use's: each Mac whose server runs this service is a screen, called what the Mac
  is called, which an agent sees and uses as its person would, while they watch. It's the Mac's person's: only they,
  the user its server is for, or the network's owner use it.

  On the Mac, it's a small app it builds there from `server/helper/`, with swiftc, and runs once for each call,
  which takes a picture of the main display or posts the mouse and keyboard's events to it.

  The helper is a macOS app of this app's own, Computer Use for The One App, so that macOS asks the Mac's person
  for Screen Recording and Accessibility in its name and keeps what they allow it through the server's updates. It's
  built again only when its source changes, and macOS then asks again, as it does for any changed app that isn't
  signed by a developer. Its process keeps which permissions it has asked macOS for, so the Mac's person is asked
  at most once each while it runs.
  """
  use GenServer
  @behaviour OneCore.Service

  @app "Computer Use for The One App.app"

  # Where in System Settings → Privacy & Security each permission is turned on.
  @panes %{"Screen Recording" => "Screen & System Audio Recording", "Accessibility" => "Accessibility"}
  @uses %{"Screen Recording" => "see this Mac's screen", "Accessibility" => "use this Mac's mouse and keyboard"}

  @doc "The caller's Macs that are on, each a screen as Computer Use lists them, with the Mac's key as `mac`."
  def screens(call) do
    for server <- OneCore.Service.servers(call, __MODULE__), not is_integer(OneCore.Service.seen(server)), may?(call, server) do
      status = OneCore.Service.call(call, __MODULE__, :status, [], server)

      missing =
        for {permission, false} <- [{"Screen Recording", status["screen_recording"]}, {"Accessibility", status["accessibility"]}],
            do: permission

      %{
        "name" => OneCore.Service.name(server) || server,
        "description" =>
          "#{OneCore.Service.name(server)}'s own screen, as its person sees it" <>
            if(missing == [], do: "", else: "; macOS doesn't allow #{Enum.join(missing, " or ")} for #{called()} yet"),
        "width" => status["width"],
        "height" => status["height"],
        "mac" => server
      }
    end
  end

  @doc "Does a Computer Use action on `mac`'s screen: a screenshot as a blob, anything else answered with what it did."
  def use(_call, _mac, "stream", _inputs), do: raise("A Mac's screen has no live video yet: it's shown a picture at a time.")

  def use(call, mac, action, inputs) do
    if not may?(call, mac), do: raise("You may not use that Mac's screen: only its person and the network's owner may.")

    case action do
      "screenshot" ->
        {:blob, "image/png", Base.decode64!(OneCore.Service.call(call, __MODULE__, :screenshot, [], mac))}

      _ ->
        done = did(action, inputs)
        OneCore.Service.call(call, __MODULE__, :act, [action, inputs], mac)
        %{done: done}
    end
  end

  # Only the Mac's person, the first its server acts for, or the network's owner uses its screen. A Mac whose server
  # is a group's or the network's has no person of its own: any member may ask for those, so only the owner uses it.
  defp may?(call, server) do
    person = List.first(OneCore.Servers.members(server))
    (String.starts_with?(person || "", "users/") and call.subject == person) or OneCore.Call.owns?(%{call | subject: "network"})
  end

  defp did("click", %{"x" => x, "y" => y} = inputs), do: "Clicked #{inputs["button"] || "left"} at #{x}, #{y}"
  defp did("double_click", %{"x" => x, "y" => y}), do: "Double-clicked at #{x}, #{y}"
  defp did("move", %{"x" => x, "y" => y}), do: "Moved to #{x}, #{y}"
  defp did("drag", %{"path" => path}), do: "Dragged through #{length(path)} points"
  defp did("scroll", %{"direction" => direction} = inputs), do: "Scrolled #{direction} #{inputs["amount"] || 5}"
  defp did("type", %{"text" => text}), do: "Typed #{String.length(text)} characters"
  defp did("key", %{"keys" => keys}), do: "Pressed #{keys}"
  defp did(action, _inputs), do: raise("#{action} needs more: Computer Use's tools say what each action takes")

  @impl OneCore.Service
  def called, do: "Computer Use for The One App"

  @impl OneCore.Service
  def needs do
    "A Mac with Xcode or its Command Line Tools, whose swiftc builds the app it brings, Computer Use for The One App, " <>
      "which takes pictures of the Mac's screen and uses its mouse and keyboard. The Mac's person allows it in System Settings → Privacy & Security, " <>
      "under Screen & System Audio Recording and under Accessibility; macOS asks them on its screen the first time it's used."
  end

  @doc "The screen's size in pixels, whether macOS lets the helper see it and use it, and where the helper is."
  def status(service) do
    service |> run(["status"]) |> Map.put("app", GenServer.call(service, :app))
  end

  @doc "A PNG of the screen, in base64."
  def screenshot(service), do: run(service, ["screenshot"])

  @doc "Does a Computer Use action other than a screenshot: `inputs` as Computer Use gives them."
  def act(service, action, inputs), do: run(service, ["act", JSON.encode!(Map.put(inputs, "action", action))])

  @impl GenServer
  def init(_) do
    for program <- ["swiftc", "codesign"], System.find_executable(program) == nil do
      raise "This machine has no #{program}: Computer Use for The One App is built on a Mac, with Xcode or its Command Line Tools"
    end

    {:ok, %{app: built(OneCore.Service.folder(self(), "server/helper")), asked: MapSet.new()}}
  end

  @impl GenServer
  def handle_call(:app, _from, state), do: {:reply, state.app, state}

  # Whether to ask macOS for `permission` now: only the first time it's missing while this runs.
  def handle_call({:ask?, permission}, _from, state),
    do: {:reply, permission not in state.asked, %{state | asked: MapSet.put(state.asked, permission)}}

  # A new version of the app whose helper's source changed is built again, in the new folder that gives it.
  @impl GenServer
  def code_change(_version, state, _extra),
    do: {:ok, %{state | app: built(OneCore.Service.folder(self(), "server/helper"))}}

  # Runs the helper once, in the calling process, and gives what it answers.
  defp run(service, arguments) do
    app = GenServer.call(service, :app)
    program = Path.join([app, "Contents", "MacOS", "helper"])
    {output, status} = System.cmd(program, arguments)

    case JSON.decode(output) do
      {:ok, %{"ok" => answer}} -> answer
      {:ok, %{"error" => why}} -> raise why
      {:ok, %{"missing" => permission}} -> raise missing(service, program, permission)
      _ -> raise "Computer Use for The One App stopped, with status #{status}: #{String.slice(output, 0, 500)}"
    end
  end

  # What's missing, said for the Mac's person, after asking macOS for it the first time, which adds the helper to
  # its list in System Settings and asks there.
  defp missing(service, program, permission) do
    asked = GenServer.call(service, {:ask?, permission})
    if asked, do: System.cmd(program, ["ask", JSON.encode!(%{permission: permission})])

    "macOS doesn't let Computer Use for The One App #{@uses[permission]} yet. " <>
      if(asked, do: "It's asking on the Mac now. ", else: "") <>
      "On the Mac, in System Settings → Privacy & Security → #{@panes[permission]}, turn on Computer Use for The One App." <>
      "\n\nOnly the Mac's person can allow it: ask them, then try again."
  end

  # The helper's app, built in `folder` if it isn't there already: under another name and then renamed, so an app
  # that's there is a whole one. Signed for this Mac alone, which macOS needs of an app before it keeps a permission
  # for it.
  defp built(folder) do
    app = Path.join(folder, @app)

    if not File.exists?(app) do
      building = app <> ".building"
      File.rm_rf!(building)
      File.mkdir_p!(Path.join([building, "Contents", "MacOS"]))
      File.cp!(Path.join(folder, "Info.plist"), Path.join([building, "Contents", "Info.plist"]))
      build(folder, "swiftc", ["-O", "-o", Path.join([building, "Contents", "MacOS", "helper"]), "main.swift"])
      build(folder, "codesign", ["--force", "--sign", "-", building])
      File.rename!(building, app)
    end

    app
  end

  defp build(folder, program, arguments) do
    case System.cmd(System.find_executable(program), arguments, cd: folder, stderr_to_stdout: true) do
      {_, 0} -> :ok
      {output, _} -> raise "Computer Use for The One App didn't build: #{program} said\n" <> output
    end
  end
end
