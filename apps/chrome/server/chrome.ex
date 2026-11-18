defmodule Chrome do
  @moduledoc """
  Chrome: Google Chrome in a container of the Containers app, of its image `desktop`, headed on that container's
  display, and driven by Microsoft's Playwright MCP server, which runs beside it and reaches it over its DevTools
  Protocol. Its tools are that server's, named as it names them without `browser_`, and each takes the browser's
  `session`.

  A session is a container, `chrome-<session>` or `chrome.2-<session>`, created through Containers' tools for whoever
  opened it, so whose it is and who reaches it is what Containers says, and Containers stops it once it's gone unused
  for its `idle_hours`. Which of the two it is, and the MCP server's own session id, are a record in the app's own
  artifact, `browsers/<whose>/<session>`; with none it's the first.

  Opening a stopped one carries its user data dir on into a new container, under its other name, so on the image as
  it is now. That's steps, each saved in its record before it's taken, so a call that fails carries on from there the
  next time it's opened: `packing` (its Chrome started and quit, its user data dir packed in its container),
  `moving` (a new container made, and the user data dir copied into it), `starting` (Chrome started in the new one,
  and the old one removed), then `running`. The old container is removed only once the new one has the user data
  dir, so nothing that fails loses it.

  A browser's profile is its user data dir, `/root/.chromium` in its container. As one is created it offers
  `chrome.starting`, for an app to hand it a user data dir to start from: a blob in the bucket, by a link and the key
  it's sealed with, which Chrome fetches into its container with Containers' `download`. An app that keeps one
  takes it with `quit`, which leaves a gzipped tarball of it in the container for the app to send with Containers'
  `upload`, and as a browser closes it offers `chrome.closing`, for that app to refuse while it hasn't. No hook moves
  a profile's bytes, which take longer than a hook has, and no tool's result carries them.

  A running browser is a screen in Computer Use, as every container with a display is, and `screen` acts on it for an
  app that lets its person use the browser: to sign in, or to get an agent past a captcha.

  A secret on a page, or in a file the browser downloaded, goes to whoever keeps it by `upload`, as a profile does:
  what each element holds is written to a file in the container by Playwright's `browser_evaluate` (its `filename`),
  so it's never in the server's answer, gathered into one JSON file there, and sent sealed by Containers' `upload`. The
  file goes once it's sent, and the download once it's taken from. `snapshot` blots out what password fields and the
  elements taken hold, so an agent doesn't read them back.
  """
  @behaviour OneCore.App
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
  # What Playwright's MCP server answered, as its text.
  @said %{
    type: "object",
    properties: %{text: %{type: "string", description: "What Playwright said, the page's state and the result among it"}},
    required: ["text"]
  }

  # Where `upload` gathers what it sends: Playwright's MCP server writes a file only under its own folder.
  @saving "/tmp/.playwright-mcp"
  # Where Chrome saves what it downloads.
  @downloads "/root/Downloads"
  # What an element holds, as `upload` takes it, marking it as taken so `snapshot` blots it out from then on.
  @take """
  (e) => {
    e.setAttribute("data-one-saved", "")
    return (e instanceof HTMLInputElement || e instanceof HTMLTextAreaElement || e instanceof HTMLSelectElement ? e.value : e.innerText).trim()
  }
  """
  # What `snapshot` blots out: what password fields and the elements taken hold, but what's too short to be a secret.
  @hidden """
  () => [...document.querySelectorAll("input[type=password], [data-one-saved]")]
    .map((e) => (e instanceof HTMLInputElement || e instanceof HTMLTextAreaElement ? e.value : e.innerText).trim())
    .filter((value) => value.length >= 4)
  """
  # Gathers what `upload` sends, given its nonce and what to take, into one JSON file, {site, values}: each field from
  # the file Playwright wrote for it, or from the download, the whole of it or a value in its JSON at a dotted path. It
  # says only what's wrong in its own words, never what it read, and leaves only the gathered file.
  @gather """
  const fs = require("fs")
  const [nonce, spec] = process.argv.slice(1)
  const { download, fields } = JSON.parse(spec)
  const piece = (name) => `#{@saving}/one-save-${nonce}-${name}`
  class Said extends Error {}
  try {
    const read = (name) => JSON.parse(fs.readFileSync(piece(name), "utf8"))
    const file = download && `#{@downloads}/${download}`
    if (file && !fs.existsSync(file)) {
      const there = fs.existsSync("#{@downloads}") ? fs.readdirSync("#{@downloads}") : []
      throw new Said(`There's no ${download} in the browser's downloads. ${there.length ? `There's ${there.join(", ")}.` : "It has none."}`)
    }
    const whole = file && fs.readFileSync(file, "utf8")
    const values = {}
    for (const [name, { json }] of Object.entries(fields)) {
      let value = file ? whole : read(`f-${name}`)
      if (file && json) {
        try { value = JSON.parse(whole) } catch { throw new Said(`${download} isn't JSON.`) }
        for (const key of json.split(".")) value = value?.[key]
        if (value === undefined || value === null) throw new Said(`${download} has no ${json}.`)
        if (typeof value !== "string") value = JSON.stringify(value)
      }
      if (typeof value !== "string" || value.trim() === "") throw new Said(`What's there for ${name} is empty.`)
      values[name] = value
    }
    const all = JSON.stringify({ site: read("site"), values })
    if (Buffer.byteLength(all) > 65536) throw new Said("That's more than 64 KB, more than a secret is.")
    fs.writeFileSync(piece("all"), all, { mode: 0o600 })
  } catch (e) {
    console.error(e instanceof Said ? e.message : `Couldn't gather it (${e.code ?? e.name}).`)
    process.exitCode = 1
  } finally {
    for (const name of fs.readdirSync("#{@saving}"))
      if (name.startsWith(`one-save-${nonce}-`) && name !== `one-save-${nonce}-all`) fs.rmSync(`#{@saving}/${name}`, { force: true })
  }
  """

  @impl true
  def icon, do: "icon.svg"

  # Its data is everyone's together, each person's browsers kept apart by who they're for (`browsers/<who>/`).
  @impl true
  def own, do: :installed

  @impl true
  def requires, do: %{"core" => ">= 0.22.18", "containers" => "to run each browser in a container"}

  @impl true
  def offers do
    [
      %{
        name: "starting",
        kind: :mutate,
        description:
          "A browser is about to start for the first time, for whoever the call is for: its `session`, `description`, " <>
            "the `url` it opens, its `container`, as Containers' tools name it, and `user_data`, nil. An app that has " <>
            "a user data dir for it answers %{\"user_data\" => %{\"link\" => link, \"key\" => key}}: a link that reads a " <>
            "blob of a gzipped tarball of the folder's contents, as Containers' `upload` sent it, and its key. Chrome " <>
            "fetches it (Containers' `download`) and the browser starts from it."
      },
      %{
        name: "closing",
        kind: :validate,
        description:
          "A browser is about to be closed with `close`, its container removed with its user data dir: its " <>
            "`session`, `description` and `container`. An app that keeps the user data dir and hasn't yet (`quit`, " <>
            "then Containers' `upload`) refuses, saying how it's kept: the browser then stays as it was, and `close` " <>
            "says why."
      }
    ]
  end

  @impl true
  def tools do
    [
      %{
        name: "open",
        task_support: :optional,
        description:
          "Open a browser: a Google Chrome of yours in a container, signed in to nothing. Opening one that's open " <>
            "already, or that was stopped after going unused, carries on with it. It takes a few seconds, or about a " <>
            "minute in the minute after Containers starts on a machine, which builds its image then.",
        inputs:
          object(%{description: %{type: "string", description: "what it's for, in a few words"}, url: %{type: "string", description: "the page to open"}}, [
            "description"
          ]),
        outputs: %{
          type: "object",
          properties: %{session: @session, text: %{type: "string", description: "What navigating to url said, if it was given"}},
          required: ["session"]
        }
      },
      %{
        name: "list",
        read_only: true,
        description: "Your browsers: each one's session, what it's for, its state (running or exited), its machine, and when it was created and last used (ms since 1970).",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            browsers: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  session: @session,
                  description: %{type: ["string", "null"], description: "What it's for"},
                  state: %{type: "string", description: "running or exited"},
                  machine: %{type: ["string", "null"]},
                  created: %{type: "integer", description: "In milliseconds since 1970"},
                  last_used: %{type: "integer", description: "In milliseconds since 1970"}
                },
                required: ["session", "description", "state", "machine", "created", "last_used"]
              }
            }
          },
          required: ["browsers"]
        }
      },
      %{
        name: "quit",
        description:
          "For an app that keeps a browser's user data dir: quit its Chrome as a person would, so it writes everything " <>
            "down, and pack its user data dir without caches into a gzipped tarball in its container, for Containers' " <>
            "`upload`. The container stays, with Chrome stopped, until `close`.",
        inputs: object(),
        outputs: %{
          type: "object",
          properties: %{
            session: @session,
            container: %{type: "string", description: "Its container, as Containers' tools name it"},
            user_data: %{type: "string", description: "The tarball's path in the container"},
            size: %{type: "integer", description: "The tarball's size in bytes"}
          },
          required: ["session", "container", "user_data", "size"]
        }
      },
      %{
        name: "upload",
        description:
          "For an app's code, which keeps secrets: send what elements on the page hold, or what's in a file the " <>
            "browser downloaded, to the bucket by `link`, which writes one blob, sealed with `key`, as one JSON " <>
            "object: {site, values}, `site` the page's host and `values` each field's, by its name. Never through a " <>
            "tool's result. The download is deleted from the browser once it's sent.",
        inputs:
          object(
            %{
              link: %{type: "object", description: "Where it goes: a link that writes one blob"},
              key: %{type: "string", contentEncoding: "base64", description: "The 32-byte key it's sealed with, made for this one blob"},
              download: %{type: "string", description: "A file the browser downloaded, by its name, to take the fields from"},
              fields: %{
                type: "object",
                description: "What to take, by the field's name: {\"client_secret\": {\"target\": \"e12\"}}, or with `download`, {\"client_secret\": {\"json\": \"web.client_secret\"}}",
                minProperties: 1,
                additionalProperties: %{
                  type: "object",
                  properties: %{
                    target: Map.put(@target, :description, "the element that holds it: its ref from the snapshot, like e12, or a unique selector"),
                    json: %{type: "string", description: "with `download`, where it is in the file's JSON, like web.client_secret; the whole file if left out"}
                  }
                }
              }
            },
            ["link", "key", "fields"]
          ),
        outputs: %{
          type: "object",
          properties: %{session: @session, size: %{type: "integer", description: "What was sent, in bytes"}},
          required: ["session", "size"]
        }
      },
      %{
        name: "close",
        description: "Close a browser and remove its container, for good. One that's gone already is closed.",
        inputs: object(),
        outputs: %{type: "object", properties: %{session: @session}, required: ["session"]}
      },
      %{
        name: "navigate",
        description: "Navigate to a URL.",
        inputs: object(%{url: %{type: "string"}}, ["url"]),
        outputs: @said
      },
      %{
        name: "navigate_back",
        description: "Go back to the previous page in the history.",
        inputs: object(),
        outputs: @said
      },
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
          }),
        outputs: @said
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
          ),
        outputs: @said
      },
      %{
        name: "hover",
        description: "Hover over an element.",
        inputs: object(%{target: @target, element: @element}, ["target"]),
        outputs: @said
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
          ),
        outputs: @said
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
          ),
        outputs: @said
      },
      %{
        name: "select_option",
        description: "Select options in a dropdown.",
        inputs: object(%{target: @target, element: @element, values: %{type: "array", items: %{type: "string"}}}, ["target", "values"]),
        outputs: @said
      },
      %{
        name: "press_key",
        description: "Press a key, like ArrowLeft, Enter or a.",
        inputs: object(%{key: %{type: "string"}}, ["key"]),
        outputs: @said
      },
      %{
        name: "wait_for",
        description: "Wait for text to appear or to go, or for some seconds to pass (at most 30), whichever is given.",
        inputs: object(%{text: %{type: "string"}, textGone: %{type: "string"}, time: %{type: "number"}}),
        outputs: @said
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
          ),
        outputs: @said
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
          }),
        outputs: %{
          type: "object",
          description: "The picture, which an agent sees",
          properties: %{mimeType: %{type: "string"}, size: %{type: "integer"}},
          required: ["mimeType"]
        }
      },
      %{
        name: "tabs",
        description: "List, open (new), close or select a tab. close and select take its index; new takes a url.",
        inputs:
          object(%{action: %{type: "string", enum: ~w(list new close select)}, index: %{type: "number"}, url: %{type: "string"}}, [
            "action"
          ]),
        outputs: @said
      },
      %{
        name: "handle_dialog",
        description: "Accept or dismiss the alert, confirm or prompt the page opened.",
        inputs: object(%{accept: %{type: "boolean"}, promptText: %{type: "string"}}, ["accept"]),
        outputs: @said
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
          ),
        outputs: %{
          type: "object",
          description: "As Computer Use says: a screenshot, what it did, or for stream, the ICE servers or the screen's answer",
          properties: %{
            mimeType: %{type: "string"},
            size: %{type: "integer"},
            done: %{type: "string", description: "What it did, in a few words"},
            iceServers: %{type: "array", items: %{type: "object"}},
            answer: %{type: "string", description: "The screen's SDP answer"}
          }
        }
      }
    ]
  end

  defp object(properties \\ %{}, required \\ []),
    do: %{type: "object", required: ["session" | required], properties: Map.put(properties, :session, @session)}

  @impl true
  def call("list", _inputs, call) do
    # One being carried on into its other container is listed once, by the one that's running if one is.
    browsers =
      for c <- containers(call) |> Enum.sort_by(&(&1["state"] != "running")) |> Enum.uniq_by(&session(&1["name"])) do
        %{
          session: session(c["name"]),
          description: c["description"],
          state: c["state"],
          machine: c["machine"],
          created: c["created"],
          last_used: c["last_used"]
        }
      end

    %{browsers: browsers}
  end

  def call("open", %{"session" => session} = inputs, call) do
    record = record(call, session)
    name = container(record, session)

    case {record["state"], Enum.find(containers(call), &(&1["name"] == name))} do
      # A carry that didn't finish carries on from its step.
      {state, _} when state in ~w(packing moving starting) ->
        carry(call, inputs, record)

      {_, %{"state" => "running"}} ->
        if inputs["url"],
          do: Map.put(mcp(call, session, "browser_navigate", %{"url" => inputs["url"]}), :session, session),
          else: %{session: session}

      # A browser that was stopped carries on in a new container, so on the image as it is now: with the user data
      # dir it stopped with, and a new MCP server, which knows none of the sessions kept from before.
      {_, %{}} ->
        carry(call, inputs, %{"state" => "packing", "from" => name, "container" => other(name, session)})

      {_, nil} ->
        record(call, session, fn _ -> %{"container" => name, "state" => "running"} end)
        create(call, inputs)
    end
  end

  def call("close", %{"session" => session}, call) do
    case browser(call, session) do
      # Containers lists only the machines that are on, so a browser it doesn't list is gone only if none is off.
      nil ->
        off = for %{"on" => false, "machine" => machine} <- Call.tool(call, "containers", "machines", %{})["machines"], do: machine
        if off != [], do: raise("#{session} isn't on a machine that's on: #{Enum.join(off, ", ")} is off. Close it once that's back")

      %{"name" => name} = c ->
        run_hook(call, "closing", %{"session" => session, "description" => c["description"], "container" => name})
        remove(call, session)
    end

    Artifact.remove(call.own, key(call, session))
    %{session: session}
  end

  # A stopped one is started for its Chrome to be quit as a person would, as a carry's is.
  def call("quit", %{"session" => session}, call) do
    c = browser(call, session) || raise("You have no browser called #{session}")
    if c["state"] != "running", do: on(call, "start", c["name"])
    packed = on(call, "exec", c["name"], %{"command" => @pack <> "test $? = 0 && stat -c %s #{@tarball}\n", "timeout" => 120})
    if packed["exit"] != 0, do: raise("Couldn't pack #{session}'s user data dir: #{String.trim(packed["stderr"])}")
    %{session: session, container: c["name"], user_data: @tarball, size: packed["stdout"] |> String.trim() |> String.to_integer()}
  end

  def call("screen", %{"session" => session} = inputs, call), do: containers(call, "screen", session, Map.delete(inputs, "session"))

  def call("upload", _inputs, %Call{caller: nil}),
    do: raise("chrome_upload is for an app's code, which says where what it takes goes and what it's sealed with.")

  # Each piece is written in the container, and only the gathered file leaves it, sealed (the moduledoc).
  def call("upload", %{"session" => session, "link" => link, "key" => key, "fields" => fields} = inputs, call) do
    download = inputs["download"]
    nonce = Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)
    piece = &"#{@saving}/one-save-#{nonce}-#{&1}"

    for {name, _field} <- fields, not (name =~ ~r/^[A-Za-z0-9_]+$/), do: raise("A field's name is letters, digits and _.")

    if download && (download != Path.basename(download) or download in [".", ".."]),
      do: raise("Name the download as Chrome saved it, like client_secret_123.json.")

    mcp(call, session, "browser_evaluate", %{"function" => "() => location.host", "filename" => piece.("site")})

    if !download do
      for {name, field} <- fields do
        target = field["target"] || raise("Say which element holds #{name}, as its `target`, or take it from a `download`.")
        mcp(call, session, "browser_evaluate", %{"function" => @take, "target" => target, "filename" => piece.("f-" <> name)})
      end
    end

    spec = JSON.encode!(%{download: download, fields: Map.new(fields, fn {name, field} -> {name, %{json: field["json"]}} end)})
    gathered = containers(call, "exec", session, %{"command" => "node -e #{shell(@gather)} #{nonce} #{shell(spec)}"})
    if gathered["exit"] != 0, do: raise(String.trim(gathered["stderr"]))

    try do
      sent = containers(call, "upload", session, %{"path" => piece.("all"), "link" => link, "key" => key})
      if download, do: containers(call, "exec", session, %{"command" => "rm -f #{shell("#{@downloads}/#{download}")}"})
      %{session: session, size: sent["size"]}
    after
      containers(call, "exec", session, %{"command" => "rm -f #{shell(piece.("all"))}"})
    end
  end

  # The page as its accessibility tree, with what password fields and the elements taken hold blotted out.
  def call("snapshot", %{"session" => session} = inputs, call) do
    hidden = mcp(call, session, "browser_evaluate", %{"function" => @hidden})

    hidden =
      with [_, values] <- Regex.run(~r/\A### Result\n(.*?)\n###/s, hidden.text),
           {:ok, values} when is_list(values) <- JSON.decode(values) do
        values
      else
        _ -> raise "Couldn't find what the page's password fields hold, to hide it."
      end

    snapshot = mcp(call, session, "browser_snapshot", Map.delete(inputs, "session"))
    %{snapshot | text: Enum.reduce(hidden, snapshot.text, &String.replace(&2, &1, "••••••"))}
  end

  def call(tool, %{"session" => session} = inputs, call) do
    mcp(call, session, "browser_" <> tool, Map.delete(inputs, "session"))
  end

  # Creates a new browser's container, with its display and no Chrome yet, puts in the user data dir an app hands
  # it, if one does, and starts Chrome on it.
  defp create(call, %{"session" => session} = inputs) do
    name = make(call, session, inputs)

    about = %{
      "session" => session,
      "description" => inputs["description"],
      "url" => inputs["url"],
      "container" => name,
      "user_data" => nil
    }

    with %{"link" => link, "key" => key} <- run_hook(call, "starting", about)["user_data"] do
      Call.stage(call, "Fetching its profile")
      containers(call, "download", session, %{"path" => @tarball, "link" => link, "key" => key})
      unpack!(call, session, @tarball)
    end

    launch(call, session, inputs["url"])
  end

  # The container the record names, made: the image `desktop`, with its display and no Chrome yet.
  defp make(call, session, inputs) do
    name = container(record(call, session), session)
    Call.stage(call, "Creating its container")

    Call.tool(call, "containers", "create", %{
      "image" => "desktop",
      "name" => name,
      "description" => inputs["description"],
      "size" => "medium",
      "ports" => [@port],
      "command" => ["desktop", "--no-browser"]
    })

    name
  end

  # Carries a stopped browser on into its other container, a step at a time from the one `record` is at, each saved
  # before it's taken (the moduledoc).
  defp carry(call, %{"session" => session} = inputs, %{"state" => "packing", "from" => from} = record) do
    record(call, session, fn _ -> record end)
    Call.stage(call, "Packing what it kept")

    if not match?(%{"state" => "running"}, Enum.find(containers(call), &(&1["name"] == from))),
      do: on(call, "start", from)

    packed = on(call, "exec", from, %{"command" => @pack, "timeout" => 120})
    if packed["exit"] != 0, do: raise("Couldn't pack #{session}'s user data dir: #{String.trim(packed["stderr"])}")
    carry(call, inputs, %{record | "state" => "moving"})
  end

  defp carry(call, %{"session" => session} = inputs, %{"state" => "moving", "from" => from, "container" => to} = record) do
    record(call, session, fn _ -> record end)
    Call.stage(call, "Moving what it kept to a new container")
    # One a try before this made holds nothing the old one doesn't.
    if Enum.any?(containers(call), &(&1["name"] == to)), do: on(call, "remove", to)
    make(call, session, inputs)
    # The user data dir goes by the bucket, sealed under a key made for it here that only this call holds, a chunk
    # at a time, so one of any size moves and none passes through a tool's result. The blob goes once it's in.
    blob = "moving/#{call.subject}/#{session}"
    key = Base.encode64(:crypto.strong_rand_bytes(32))
    link = &(call.own |> Artifact.link(blob, &1) |> JSON.encode!() |> JSON.decode!())
    Artifact.remove(call.own, blob)
    on(call, "upload", from, %{"path" => @tarball, "link" => link.(:write), "key" => key})
    on(call, "download", to, %{"path" => @tarball, "link" => link.(:read), "key" => key})
    Artifact.remove(call.own, blob)
    unpack!(call, session, @tarball)
    carry(call, inputs, %{record | "state" => "starting"})
  end

  defp carry(call, %{"session" => session} = inputs, %{"state" => "starting", "from" => from, "container" => to}) do
    record(call, session, fn _ -> %{"state" => "starting", "from" => from, "container" => to} end)
    launched = launch(call, session, inputs["url"])
    if Enum.any?(containers(call), &(&1["name"] == from)), do: on(call, "remove", from)
    record(call, session, fn _ -> %{"state" => "running", "container" => to} end)
    launched
  end

  # Opens the user data dir handed over as `tarball` in the session's container.
  defp unpack!(call, session, tarball) do
    unpack = "mkdir -p #{@user_data} && tar -xzf #{shell(tarball)} -C #{@user_data} && rm #{shell(tarball)}"
    unpacked = containers(call, "exec", session, %{"command" => unpack})

    if unpacked["exit"] != 0,
      do: raise("Couldn't open the profile #{session} was handed: #{String.trim(unpacked["stderr"])}")
  end

  defp launch(call, session, url) do
    Call.stage(call, "Starting Chrome")
    containers(call, "exec", session, %{"command" => "sh -c #{shell(@launch)} sh #{if url, do: shell(url)}"})
    %{session: session}
  end

  defp shell(word), do: "'" <> String.replace(word, "'", ~S('\'')) <> "'"

  # Runs one of the hooks Chrome offers, and gives what's changed: nothing on a server from before apps could
  # offer hooks (One Core 7929556e6), where no app can join one.
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
  # its picture, or else its text, as `@said`.
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
        record(call, session, &Map.delete(&1, "mcp"))
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
      [] -> %{text: text}
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
    record(call, session, &Map.put(&1, "mcp", id))
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

  defp session_id(call, session), do: record(call, session)["mcp"]

  # The session's record (the moduledoc): which container it is, its MCP session, and how far a carry got.
  defp record(call, session) do
    case Artifact.read(call.own, key(call, session)) do
      {:ok, data, _tag} -> JSON.decode!(data)
      :absent -> %{}
    end
  end

  # Changes the session's record in one step: `change` is given it as it is now.
  defp record(call, session, change) do
    Artifact.update(call.own, key(call, session), fn was -> JSON.encode!(change.(if(was, do: JSON.decode!(was), else: %{}))) end)
  end

  defp key(call, session), do: "browsers/#{call.subject}/#{session}"

  # A session's container, by its record, and the other name it's carried on into.
  defp container(record, session), do: record["container"] || "chrome-" <> session
  defp other("chrome-" <> session, _session), do: "chrome.2-" <> session
  defp other(_name, session), do: "chrome-" <> session

  # Removes every container of the session's.
  defp remove(call, session),
    do: for(%{"name" => name} <- containers(call), session(name) == session, do: on(call, "remove", name))

  defp on(call, tool, name, inputs \\ %{}), do: Call.tool(call, "containers", tool, Map.put(inputs, "container", name))

  # Calls a tool of Containers on the session's container. What went wrong is said of the browser where it's that
  # there's none, or that it was stopped.
  defp containers(call, tool, session, inputs) do
    on(call, tool, container(record(call, session), session), inputs)
  rescue
    e ->
      case browser(call, session) do
        nil -> raise "You have no browser called #{session}"
        %{"state" => "running"} -> reraise e, __STACKTRACE__
        %{} -> raise "The browser was stopped after going unused.\n\n`open` starts #{session} again, afresh."
      end
  end

  # The session's container, as Containers lists it: the one its record names, else any of its two.
  defp browser(call, session) do
    mine = for c <- containers(call), session(c["name"]) == session, do: c
    name = container(record(call, session), session)
    Enum.find(mine, &(&1["name"] == name)) || List.first(mine)
  end

  # The browsers whoever this call is for may use: their containers called chrome-<session> or chrome.2-<session>,
  # of the image desktop.
  defp containers(call),
    do:
      Enum.filter(
        Call.tool(call, "containers", "list", %{})["containers"],
        &(session(&1["name"]) && &1["image"] =~ "containers-desktop")
      )

  defp session("chrome-" <> session), do: session
  defp session("chrome.2-" <> session), do: session
  defp session(_name), do: nil
end
