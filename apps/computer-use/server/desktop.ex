defmodule ComputerUse.Desktop do
  @moduledoc """
  A computer's own screen, as Computer Use's: each Mac, Linux or Windows desktop whose server runs this service is a screen,
  called what the computer is called, which an agent sees and uses as its person would, while they watch. It's the
  computer's person's: only they, the user its server is for, or the network's owner use it.

  It runs Cua Driver (github.com/trycua/cua, MIT), the version pinned here, downloaded from its release and checked,
  in the person's session, kept running by the system as their server is (`OneCore.System`): on a Mac a LaunchAgent,
  on Linux a systemd user unit of the graphical session, on Windows a scheduled task of theirs that starts as they
  log on. Its daemon (`cua-driver serve`) answers
  MCP over HTTP on a port of the loopback of its own, with a bearer token kept in this service's folder, the same on
  every kind of computer. Each action is one of its tools, called there either with the whole display as its target
  (`scope: desktop`) and the person's own pointer and keyboard (`delivery_mode: foreground`), as a person uses them,
  or aimed at one window (`pid`, `window_id`) or one of its elements (`element_token`), which Cua Driver does in the
  background: the window stays where it is, behind whatever the person has in front, and their pointer and keyboard
  stay theirs.

  macOS lets a program see the screen only with Screen Recording and use the mouse and keyboard only with
  Accessibility, and asks for and keeps them in the name of the program it holds responsible, which for a launchd
  agent is the program it starts. So on a Mac the agent starts this app's helper app, Computer Use for the One App
  (`server/bin/helper`, built by `tooling/mac-helper`), which runs Cua Driver as its child in its embedded mode:
  macOS asks for both in the helper's name, with its icon, and as the helper is signed with a Developer ID, keeps
  them through its updates and Cua Driver's.
  """
  use GenServer
  @behaviour OneCore.Service

  # Cua Driver's release, and the build of it for each kind of machine with its SHA-256, from the release's
  # SHA256SUMS. A new version is a new pin here.
  @cua "0.53.0"
  @builds %{
    "macos" => {"darwin-universal-binary.tar.gz", "2dae2548618c2223461bcd17d211c6e8c0409fef27923b7917755739f0e66a12"},
    "linux-x86_64" => {"linux-x86_64-binary.tar.gz", "fdee86c42b3f1f43916887f05e38cc6b79883668cde55c29ca862abb09db1908"},
    "linux-aarch64" => {"linux-arm64-binary.tar.gz", "4dcb9e1cfb3cebc995b99d85ee218978682520eae7dfa552555016e2ea9afbc4"},
    "windows-x86_64" => {"windows-x86_64-binary.zip", "5a0cb66baaeb0dbd5ac9efb135d3a3fecbf6383796d556f25122b37cbc58e713"},
    "windows-aarch64" => {"windows-arm64-binary.zip", "93cef9716856da3c416fe4097716e8d009e2064b16dacb7b62118944d9f60ea1"}
  }

  @helper "Computer Use for the One App.app"
  @id "app.theone.computer-use"
  # What the system knows it by, with its server's key (`OneCore.System`).
  @name "computer-use"

  # Where in System Settings → Privacy & Security each permission is turned on, the link that opens it there, and
  # what it's for.
  @panes %{"Screen Recording" => "Screen & System Audio Recording", "Accessibility" => "Accessibility"}
  @links %{
    "Screen Recording" => "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
    "Accessibility" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
  }
  @uses %{"Screen Recording" => "see this Mac's screen", "Accessibility" => "use this Mac's mouse and keyboard"}
  @needed %{"screenshot" => "Screen Recording", "windows" => "Screen Recording"}

  @doc "The caller's computers that are on, each a screen as Computer Use lists them, with its server's key as `desktop`."
  def screens(call) do
    for server <- OneCore.Service.servers(call, __MODULE__), not is_integer(OneCore.Service.seen(server)), may?(call, server) do
      status = OneCore.Service.call(call, __MODULE__, :status, [], server)
      name = OneCore.Service.name(server)

      %{
        "name" => name || server,
        "description" =>
          "#{name}'s own screen, as its person sees it" <>
            if(status["missing"] == [], do: "", else: "; macOS doesn't allow #{Enum.join(status["missing"], " or ")} for #{called()} yet"),
        "width" => status["width"],
        "height" => status["height"],
        "desktop" => server
      }
    end
  end

  @doc """
  Does a Computer Use action on `desktop`'s screen: a screenshot as a blob, `windows` and `elements` as Cua Driver
  answers them, anything else answered with what it did.
  """
  def use(_call, _desktop, "stream", _inputs), do: raise("A computer's own screen has no live video yet: it's shown a picture at a time.")

  def use(call, desktop, action, inputs) do
    if not may?(call, desktop), do: raise("You may not use that computer's screen: only its person and the network's owner may.")

    case action do
      "screenshot" ->
        {:blob, "image/png", Base.decode64!(OneCore.Service.call(call, __MODULE__, :screenshot, [inputs], desktop))}

      "windows" ->
        OneCore.Service.call(call, __MODULE__, :windows, [inputs], desktop)

      "elements" ->
        OneCore.Service.call(call, __MODULE__, :elements, [inputs], desktop)

      _ ->
        done = did(action, inputs)
        Map.put(OneCore.Service.call(call, __MODULE__, :act, [action, inputs], desktop), "done", done)
    end
  end

  # Only the computer's person, the first its server acts for, or the network's owner uses its screen. One whose
  # server is a group's or the network's has no person of its own: any member may ask for those, so only the owner
  # uses it.
  defp may?(call, server) do
    person = List.first(OneCore.Servers.members(server))
    (String.starts_with?(person || "", "users/") and call.subject == person) or OneCore.Call.owns?(%{call | subject: "network"})
  end

  defp did("click", %{"element_token" => token} = inputs), do: "Clicked #{inputs["button"] || "left"} on #{token}"
  defp did("click", %{"x" => x, "y" => y} = inputs), do: "Clicked #{inputs["button"] || "left"} at #{x}, #{y}#{window(inputs)}"
  defp did("double_click", %{"element_token" => token}), do: "Double-clicked on #{token}"
  defp did("double_click", %{"x" => x, "y" => y} = inputs), do: "Double-clicked at #{x}, #{y}#{window(inputs)}"
  defp did("move", %{"x" => x, "y" => y}), do: "Moved to #{x}, #{y}"
  defp did("drag", %{"path" => path} = inputs), do: "Dragged through #{length(path)} points#{window(inputs)}"
  defp did("scroll", %{"direction" => direction} = inputs), do: "Scrolled #{direction} #{inputs["amount"] || 5}#{window(inputs)}"
  defp did("type", %{"text" => text} = inputs), do: "Typed #{String.length(text)} characters#{window(inputs)}"
  defp did("key", %{"keys" => keys} = inputs), do: "Pressed #{keys}#{window(inputs)}"
  defp did("set_value", %{"element_token" => token, "value" => _}), do: "Set the value of #{token}"
  defp did(action, _inputs), do: raise("#{action} needs more: Computer Use's tools say what each action takes")

  defp window(%{"element_token" => token}), do: " into #{token}"
  defp window(%{"window_id" => id}), do: " in window #{id}"
  defp window(%{"pid" => pid}), do: " in #{pid}'s window"
  defp window(_inputs), do: ""

  @impl OneCore.Service
  def called, do: "Computer Use for the One App"

  @impl OneCore.Service
  def needs do
    "A Mac with macOS 14 or later, a Linux desktop on X11 (or XWayland) with AT-SPI, or Windows 10 or 11, whose person uses it. It downloads " <>
      "Cua Driver #{@cua} from GitHub (trycua/cua) and runs it in their session, which takes pictures of the screen and uses its " <>
      "mouse and keyboard. On a Mac it runs inside Computer Use for the One App, which the Mac's person allows in System Settings → " <>
      "Privacy & Security, under Screen & System Audio Recording and under Accessibility; macOS asks them on its screen the first " <>
      "time it's used."
  end

  @doc "The screen's picture's size, and which of macOS's permissions Cua Driver lacks on a Mac, asking macOS for none."
  def status(service) do
    {width, height} = picture(frame(service))
    %{"width" => width, "height" => height, "missing" => missing(service)}
  end

  defp missing(service) do
    if :os.type() == {:unix, :darwin} do
      allowed = tool(service, "check_permissions", %{})["structuredContent"] || %{}
      for {key, permission} <- [{"screen_recording", "Screen Recording"}, {"accessibility", "Accessibility"}], allowed[key] == false, do: permission
    else
      []
    end
  end

  @doc """
  A PNG of the screen, or of one window of it (`pid` and `window_id`), in base64, scaled down to what a model sees
  whole. A window's is taken in the background, as it is behind whatever is in front of it, and its pixels are what
  actions aimed at it count.
  """
  def screenshot(service, inputs) do
    allowed!(service, "screenshot")

    result =
      if Map.has_key?(inputs, "pid") or Map.has_key?(inputs, "window_id") do
        # No more than about 1.15 megapixels whatever the window's shape, as `picture/1` does for the display.
        window(service, "get_window_state", Map.merge(inputs, %{"include_accessibility_tree" => false, "max_image_dimension" => 1072}))
      else
        {width, height} = picture(frame(service))
        tool(service, "get_desktop_state", %{"max_image_dimension" => max(width, height)})
      end

    # A window it can't picture (a minimized one, on Windows) it says why of.
    Enum.find_value(result["content"], &(&1["type"] == "image" && &1["data"])) || raise "Cua Driver gave no picture: #{text(result)}"
  end

  @doc "The computer's top-level windows, as Cua Driver's `list_windows` gives them: of one app (`app`, `pid`) or all."
  def windows(service, inputs) do
    allowed!(service, "windows")
    tool(service, "list_windows", inputs)["structuredContent"]
  end

  @doc """
  One window's elements, read through accessibility in the background, as Cua Driver's `get_window_state` gives
  them: a tree of rows, each `[N]` an element whose `element_token` is `<snapshot_id>:N`, for actions aimed at it.
  """
  def elements(service, inputs) do
    allowed!(service, "elements")
    window(service, "get_window_state", Map.put(inputs, "include_screenshot", false))["structuredContent"]
  end

  @doc """
  Does a Computer Use action other than a screenshot: `inputs` as Computer Use gives them. Answers with what Cua
  Driver says of its effect, where it does: `effect` (confirmed, unverifiable or suspected_noop) and the `escalation`
  it recommends.
  """
  def act(service, action, inputs) do
    allowed!(service, action)
    desktop? = not Enum.any?(~w(pid window_id element_token), &Map.has_key?(inputs, &1))

    {send, target, at} =
      if desktop? do
        # As a person does it: anywhere on the display, with their own pointer and keyboard.
        frame = frame(service)
        picture = picture(frame)
        {&tool(service, &1, &2), %{"scope" => "desktop", "delivery_mode" => "foreground"}, &at(&1, frame, picture)}
      else
        # Aimed at a window or an element, in the background: x and y are pixels of that window's own screenshot,
        # which Cua Driver counts itself.
        {&window(service, &1, &2), Map.take(inputs, ~w(pid window_id element_token)), &Map.take(&1, ~w(x y))}
      end

    result =
      case action do
        "click" ->
          send.("click", Map.merge(target, %{"button" => inputs["button"] || "left"}) |> Map.merge(at.(inputs)))

        # Cua Driver's own double-click is aimed at a window or an element; on the display it's a click of two.
        "double_click" when desktop? ->
          send.("click", Map.merge(target, %{"count" => 2}) |> Map.merge(at.(inputs)))

        "double_click" ->
          send.("double_click", Map.merge(target, at.(inputs)))

        "move" ->
          if not desktop?, do: raise("`move` moves the person's own pointer, which actions aimed at a window leave alone: it takes no window")
          send.("move_cursor", Map.merge(target, at.(inputs)))

        "drag" ->
          # Cua Driver drags in a straight line, from a point to a point.
          case Enum.map(inputs["path"] || [], at) do
            [from, to] ->
              send.("drag", Map.merge(target, %{"from_x" => from["x"], "from_y" => from["y"], "to_x" => to["x"], "to_y" => to["y"]}))

            _ ->
              raise "On a computer's own screen a drag is a straight line: give its path as two points, where it starts and where it ends"
          end

        "scroll" ->
          send.("scroll", Map.merge(target, %{"direction" => inputs["direction"], "amount" => inputs["amount"] || 5}) |> Map.merge(at.(inputs)))

        "type" ->
          send.("type_text", Map.put(target, "text", inputs["text"] || ""))

        "key" ->
          presses =
            for combo <- String.split(inputs["keys"] || "", " ", trim: true) do
              case keys(combo) do
                [key] -> send.("press_key", Map.put(target, "key", key))
                keys -> send.("hotkey", Map.put(target, "keys", keys))
              end
            end

          List.last(presses, %{})

        "set_value" ->
          if not Map.has_key?(target, "element_token"), do: raise("set_value sets an element's value: give its element_token, from `elements`")
          send.("set_value", Map.put(target, "value", inputs["value"] || ""))

        _ ->
          raise "There's no action #{action}"
      end

    Map.take(result["structuredContent"] || %{}, ~w(effect escalation))
  end

  # The display's whole size in pixels, as Cua Driver's desktop actions count them. It gives a Mac's in points, with
  # their scale, and Windows' in pixels already, with its scale beside them: a Surface at 200% says 2736×1824 and 2.
  defp frame(service) do
    %{"width" => width, "height" => height} = size = tool(service, "get_screen_size", %{})["structuredContent"]
    scale = if :os.type() == {:unix, :darwin}, do: size["scale_factor"] || 1, else: 1
    {round(width * scale), round(height * scale)}
  end

  # The size of the picture of a display: no bigger than a model is shown a picture whole, up to 1568 pixels on its
  # long side and about 1.15 megapixels (Anthropic's vision limits). A bigger one is shrunk before it's seen, and the x
  # and y it answers then miss. So the display is scaled down to fit, never up.
  defp picture({width, height}) do
    scale = Enum.min([1, 1568 / max(width, height), :math.sqrt(1_150_000 / (width * height))])
    {round(width * scale), round(height * scale)}
  end

  # A pixel of the picture, as the middle of the pixels of the display it stands for.
  defp at(%{"x" => x, "y" => y}, {width, height}, {across, down}) when x >= 0 and y >= 0 and x < across and y < down,
    do: %{"x" => (x + 0.5) * width / across, "y" => (y + 0.5) * height / down}

  defp at(%{"x" => x, "y" => y}, _frame, {across, down}), do: raise("#{x}, #{y} is off the screen, which is #{across}×#{down}")
  defp at(_point, _frame, _picture), do: raise("give x and y")

  # A press, such as `ctrl+shift+t`, as xdotool names its keys, as the keys Cua Driver names: its modifiers, then its
  # key. On a Mac `ctrl` with a letter, digit or punctuation is ⌘, as a shortcut written for Linux or Windows means
  # there (ctrl+c copies), and with anything else ⌃ (ctrl+Tab); `control` is always ⌃, and `super`, `cmd` and `meta` ⌘.
  defp keys(combo) do
    names = String.split(combo, "+")
    key = key(List.last(names))
    character? = String.length(key) == 1
    mac? = :os.type() == {:unix, :darwin}

    modifiers =
      for name <- Enum.drop(names, -1) do
        case String.downcase(name) do
          ctrl when ctrl in ~w(ctrl control_l control_r) -> if(mac? and character?, do: "cmd", else: "ctrl")
          "control" -> "ctrl"
          super when super in ~w(super super_l super_r meta meta_l meta_r cmd command) -> if(mac?, do: "cmd", else: "super")
          alt when alt in ~w(alt alt_l alt_r option) -> "alt"
          shift when shift in ~w(shift shift_l shift_r) -> "shift"
          other -> raise "There's no modifier called #{other}: ctrl, shift, alt, super (cmd) or control"
        end
      end

    modifiers ++ [key]
  end

  @named %{
    "return" => "return", "kp_enter" => "enter", "enter" => "enter", "tab" => "tab", "space" => "space",
    "backspace" => "backspace", "delete" => "delete", "escape" => "escape", "esc" => "escape", "home" => "home",
    "end" => "end", "prior" => "pageup", "page_up" => "pageup", "pageup" => "pageup", "next" => "pagedown",
    "page_down" => "pagedown", "pagedown" => "pagedown", "left" => "left", "right" => "right", "up" => "up",
    "down" => "down", "insert" => "insert", "minus" => "-", "equal" => "=", "plus" => "+", "bracketleft" => "[",
    "bracketright" => "]", "braceleft" => "{", "braceright" => "}", "backslash" => "\\", "bar" => "|",
    "semicolon" => ";", "colon" => ":", "apostrophe" => "'", "quotedbl" => "\"", "comma" => ",", "less" => "<",
    "period" => ".", "greater" => ">", "slash" => "/", "question" => "?", "grave" => "`", "asciitilde" => "~",
    "exclam" => "!", "at" => "@", "numbersign" => "#", "dollar" => "$", "percent" => "%", "asciicircum" => "^",
    "ampersand" => "&", "asterisk" => "*", "parenleft" => "(", "parenright" => ")", "underscore" => "_"
  }

  # A key by its name: as xdotool names it, or a character, whose case matters only for a letter.
  defp key(name) do
    cond do
      String.length(name) == 1 -> name
      @named[String.downcase(name)] -> @named[String.downcase(name)]
      String.downcase(name) =~ ~r/^f([1-9]|1[0-2])$/ -> String.downcase(name)
      true -> raise "There's no key called #{name}: name keys as xdotool does (Return, ctrl+l, BackSpace, F5)"
    end
  end

  # What a Mac's person must allow first for an action, said for them, opening System Settings where it's turned on the
  # first time it's missing while this runs (see handle_call({:ask, permission}, ...)). A permission
  # turned on after Cua Driver started is read by it once it starts again: a second action that finds it missing starts
  # it again, so the one after reads it afresh.
  defp allowed!(service, action) do
    permission = @needed[action] || "Accessibility"

    cond do
      permission in missing(service) ->
        where = "System Settings → Privacy & Security → #{@panes[permission]} (#{@links[permission]})"

        how =
          case GenServer.call(service, {:ask, permission}) do
            :opened -> "#{where} is open on the Mac: they turn on Computer Use for the One App there, then try again."
            :before -> "On the Mac, they open #{where} and turn on Computer Use for the One App, then try again."
          end

        raise "macOS doesn't let Computer Use for the One App #{@uses[permission]} yet, and only the Mac's person can allow it. " <> how

      action == "type" and :os.type() == {:unix, :darwin} and secure_input?(service) ->
        raise "A password field has the focus on the Mac. Its person types passwords and codes themselves."

      true ->
        :ok
    end
  end

  defp secure_input?(service) do
    {answer, 0} = System.cmd(Path.join([GenServer.call(service, :helper), "Contents", "MacOS", "helper"]), ["secure-input"])
    String.trim(answer) == "true"
  end

  # Calls one of Cua Driver's tools on the whole display, on a connection of its own, which it closes: the daemon
  # scales the x and y of a connection's later actions by its last screenshot's, and these are the display's own.
  defp tool(service, name, args), do: call(service, [headers: [connection: "close"]], name, args)

  # Calls one of Cua Driver's tools aimed at a window or an element, on the one connection kept open for them, Req's
  # pool of one: the daemon keeps a window's elements and its last screenshot, which element_tokens and x and y count
  # from, for as long as the connection they were read on.
  defp window(service, name, args), do: call(service, [finch: [size: 1, count: 1, pool_timeout: 120_000]], name, args)

  # Calls one of Cua Driver's tools with what of `args` that tool takes on this kind of computer, which differs
  # between them (a Mac's `move_cursor` takes `delivery_mode`, Linux's doesn't), in an MCP session of its own, on
  # `connection` (Req's options for it).
  defp call(service, connection, name, args) do
    session = session(service, connection)
    takes = GenServer.call(service, :takes) || takes(service, connection, session)

    case mcp(service, connection, session, "tools/call", %{name: name, arguments: Map.take(args, takes[name] || [])}) do
      {%{"isError" => true} = result, _} -> raise text(result)
      {result, _} -> result
    end
  end

  # What each of its tools takes, kept until Cua Driver starts again.
  defp takes(service, connection, session) do
    {%{"tools" => tools}, _} = mcp(service, connection, session, "tools/list", %{})
    takes = Map.new(tools, &{&1["name"], Map.keys(&1["inputSchema"]["properties"] || %{})})
    GenServer.cast(service, {:takes, takes})
    takes
  end

  defp session(service, connection) do
    initialize = %{protocolVersion: "2025-06-18", capabilities: %{}, clientInfo: %{name: "computer-use", version: "1"}}
    {_, session} = mcp(service, connection, nil, "initialize", initialize)
    session
  end

  # A JSON-RPC request to Cua Driver's MCP endpoint, answered with its result and the session it's in.
  defp mcp(service, connection, session, method, params) do
    %{"port" => port, "token" => token} = GenServer.call(service, :endpoint)
    headers = [authorization: "Bearer #{token}"] ++ if(session, do: ["mcp-session-id": session], else: [])
    request = %{jsonrpc: "2.0", id: 1, method: method, params: params}
    req = Req.new(url: "http://127.0.0.1:#{port}/mcp", json: request, headers: headers, retry: false, receive_timeout: 120_000)

    case Req.post(Req.merge(req, connection)) do
      {:ok, %{status: 200, body: %{"result" => result}} = response} -> {result, session || List.first(response.headers["mcp-session-id"] || [])}
      {:ok, %{body: %{"error" => %{"message" => why}}}} -> raise "Cua Driver: #{why}"
      {:ok, %{status: status}} -> raise "Cua Driver answered #{status}"
      {:error, %{reason: :econnrefused}} -> raise "Cua Driver isn't running on this computer: it runs in its person's desktop session, once they're signed in"
      {:error, error} -> raise "Cua Driver didn't answer: #{Exception.message(error)}"
    end
  end

  defp text(result), do: Enum.map_join(result["content"] || [], "\n", &(&1["text"] || "")) |> String.trim()

  @impl GenServer
  def init(_) do
    {:ok, start(%{endpoint: nil, takes: nil, helper: nil, program: nil, driver: nil, asked: MapSet.new()})}
  end

  @impl GenServer
  def handle_call(:endpoint, _from, state), do: {:reply, state.endpoint, state}
  def handle_call(:takes, _from, state), do: {:reply, state.takes, state}
  def handle_call(:helper, _from, state), do: {:reply, state.helper, state}

  # The first time `permission` is missing while this runs, asks macOS for it, which lists the helper in its pane of
  # Privacy & Security, and opens System Settings there, where it's turned on: macOS never gives it from its own
  # question, which may or may not show, and whose only button goes there too. After that, starts Cua Driver again,
  # so it reads what macOS allows now.
  def handle_call({:ask, permission}, _from, state) do
    if permission in state.asked do
      restarted(state)
      {:reply, :before, state}
    else
      System.cmd("open", ["-n", "-g", state.helper, "--args", "ask", permission])
      {_, opened} = System.cmd("open", [@links[permission]])
      {:reply, if(opened == 0, do: :opened, else: :before), %{state | asked: MapSet.put(state.asked, permission)}}
    end
  end

  @impl GenServer
  def handle_cast({:takes, takes}, state), do: {:noreply, %{state | takes: takes}}

  # A new version of the app with another Cua Driver or another helper starts them again.
  @impl GenServer
  def code_change(_version, state, _extra), do: {:ok, start(state)}

  # Removed from this computer, or its app uninstalled: Cua Driver stops, and the system forgets it, with its socket
  # and its lock on a Mac or Linux. What's in the service's folder goes with it. Stopped any other way, as with its
  # server, it keeps running.
  @impl GenServer
  def terminate({:shutdown, :removed}, state) do
    stopped(state.driver)
    OneCore.System.remove(@name, key())
    for file <- [socket(), socket() <> ".lock"], do: File.rm(file)
    :ok
  end

  def terminate(_why, _state), do: :ok

  # Cua Driver, downloaded if it isn't here yet, kept running by the system in the person's session: started now, or
  # started again if what it runs changed. On a Mac it's the helper that's run, which runs Cua Driver.
  defp start(state) do
    folder = OneCore.Service.folder(self())
    driver = driver(folder)
    state = %{state | endpoint: endpoint(folder), takes: nil}

    {command, state, new?} =
      case :os.type() do
        {:unix, :darwin} ->
          {helper, new?} = helper(folder)
          {[Path.join([helper, "Contents", "MacOS", "helper"]), "run" | command(driver)], %{state | helper: helper}, new?}

        _ ->
          {command(driver), state, false}
      end

    program = %{
      name: @name,
      key: key(),
      description: "Computer Use for the One App: Cua Driver, on this computer's screen.",
      command: command,
      env: environment(folder, state.endpoint),
      folder: folder,
      desktop: true,
      start_within: nil,
      stop_within: 30_000
    }

    # A version before kept neither in its state, which a new one starts from (`code_change`).
    state = Map.merge(state, %{program: program, driver: driver})

    # Its task on Windows was "the One App Computer Use <key>" until 20 Nov 2026, which would start Cua Driver again
    # each minute beside this: it goes, as each Windows server runs this once.
    old = "the One App Computer Use #{key()}"

    if match?({:win32, _}, :os.type()) and match?({_, 0}, System.cmd("schtasks", ["/query", "/tn", old], stderr_to_stdout: true)) do
      stopped(driver)
      System.cmd("schtasks", ["/end", "/tn", old], stderr_to_stdout: true)
      System.cmd("schtasks", ["/delete", "/tn", old, "/f"], stderr_to_stdout: true)
    end

    if OneCore.System.install(program) or new?, do: restarted(state), else: OneCore.System.start(program)

    state
  end

  # Cua Driver, started again as the system keeps it, asked to stop by its own `stop` first, as on Windows ending its
  # task ends only conhost.
  defp restarted(state) do
    stopped(state.driver)
    OneCore.System.restart(state.program)
  end

  defp stopped(nil), do: :ok
  defp stopped(driver), do: System.cmd(driver, ["stop", "--socket", socket()], stderr_to_stdout: true)

  # The loopback port Cua Driver's MCP endpoint listens on, one the system gave free the first time, and the token it
  # takes, both kept in the service's folder.
  defp endpoint(folder) do
    file = Path.join(folder, "endpoint.json")

    case File.read(file) do
      {:ok, json} ->
        JSON.decode!(json)

      {:error, _} ->
        {:ok, listening} = :gen_tcp.listen(0, ip: {127, 0, 0, 1})
        {:ok, port} = :inet.port(listening)
        :gen_tcp.close(listening)
        endpoint = %{"port" => port, "token" => Base.encode16(:crypto.strong_rand_bytes(32), case: :lower)}
        File.write!(file, JSON.encode!(endpoint))
        File.chmod!(file, 0o600)
        endpoint
    end
  end

  # Cua Driver's program for this kind of machine, from its release, checked against its SHA-256, in a folder named
  # for its version; the folder before goes.
  defp driver(folder) do
    {file, sha256} = @builds[kind()] || raise "Cua Driver #{@cua} has no build for #{kind()}"
    within = Path.join(folder, "cua-driver-#{@cua}")
    name = if match?({:win32, _}, :os.type()), do: "cua-driver.exe", else: "cua-driver"
    program = Path.join(within, name)

    if not File.exists?(program) do
      url = "https://github.com/trycua/cua/releases/download/cua-driver-rs-v#{@cua}/cua-driver-rs-#{@cua}-#{file}"
      %{status: 200, body: archive} = Req.get!(url, decode_body: false, receive_timeout: 300_000)

      if Base.encode16(:crypto.hash(:sha256, archive), case: :lower) != sha256,
        do: raise("Cua Driver from #{url} isn't the build it should be: its SHA-256 isn't #{sha256}")

      # Unpacked beside it and moved in whole, so a folder that's there has all of it.
      writing = within <> ".writing"
      File.rm_rf!(writing)
      File.mkdir_p!(writing)

      if String.ends_with?(file, ".zip"),
        do: {:ok, _} = :zip.extract(archive, cwd: String.to_charlist(writing), file_list: [String.to_charlist(name)]),
        else: :ok = :erl_tar.extract({:binary, archive}, [:compressed, cwd: String.to_charlist(writing), files: [String.to_charlist(name)]])

      File.chmod!(Path.join(writing, name), 0o755)
      File.rename!(writing, within)
    end

    for other <- File.ls!(folder), String.starts_with?(other, "cua-driver-"), other != Path.basename(within),
        do: File.rm_rf!(Path.join(folder, other))

    program
  end

  defp kind do
    arch = :erlang.system_info(:system_architecture) |> to_string() |> String.split("-") |> hd()

    case :os.type() do
      {:unix, :darwin} -> "macos"
      {:unix, :linux} -> "linux-#{arch}"
      # Windows' VM says only win32 of its architecture.
      {:win32, _} -> if(System.get_env("PROCESSOR_ARCHITECTURE") == "ARM64", do: "windows-aarch64", else: "windows-x86_64")
      {_, os} -> "#{os}-#{arch}"
    end
  end

  # The daemon's own socket, or named pipe on Windows, which this doesn't use, apart from any other Cua Driver's: in a
  # short path, as a socket's path is at most 104 bytes on a Mac.
  defp socket do
    case :os.type() do
      {:unix, :darwin} -> Path.expand("~/Library/Caches/#{@id}.#{key()}.sock")
      {:win32, _} -> "\\\\.\\pipe\\one-computer-use-#{key()}"
      _ -> Path.join(System.get_env("XDG_RUNTIME_DIR") || Path.expand("~/.cache"), "one-computer-use-#{key()}.sock")
    end
  end

  defp key, do: binary_part(OneCore.Service.server(), 0, 12)

  # The helper app, from this version's build of it, in the service's folder, where it stays as each version replaces
  # it, so macOS finds it where it was. Unpacked beside it and moved in whole; gives where it is, and whether it's new.
  defp helper(folder) do
    build = Path.join(OneCore.Service.folder(self(), "server/bin/helper"), "macos-aarch64")
    helper = Path.join(folder, @helper)
    from = Path.join(folder, "helper-from")
    new? = File.read(from) != {:ok, build} or not File.dir?(helper)

    if new? do
      writing = helper <> ".writing"
      File.rm_rf!(writing)
      File.mkdir_p!(writing)
      {_, 0} = System.cmd("tar", ["-xzf", build, "-C", writing])
      File.rm_rf!(helper)
      File.rename!(Path.join(writing, @helper), helper)
      File.rm_rf!(writing)
      File.write!(from, build)
    end

    {helper, new?}
  end

  defp command(driver) do
    embedded = if :os.type() == {:unix, :darwin}, do: ["--embedded", "--host-bundle-id", @id], else: []
    # Nothing of its own on the person's screen: no cursor of the agent's drawn over it, and no question from macOS
    # to let it script an app (Finder, as set_value renames a file), which it refuses instead.
    [driver, "serve", "--socket", socket(), "--no-overlay", "--no-automation-prompts" | embedded]
  end

  # Its MCP endpoint, its own state, apart from any other Cua Driver on the computer, and neither telemetry nor update
  # checks: it's the version pinned here.
  defp environment(folder, %{"port" => port, "token" => token}) do
    [
      {"CUA_DRIVER_RS_HOME", Path.join(folder, "cua-driver-home")},
      {"CUA_DRIVER_RS_MCP_HTTP_PORT", "#{port}"},
      {"CUA_DRIVER_RS_MCP_HTTP_TOKEN", token},
      {"CUA_DRIVER_RS_TELEMETRY_ENABLED", "0"},
      {"CUA_DRIVER_RS_UPDATE_CHECK", "0"}
    ]
  end
end
