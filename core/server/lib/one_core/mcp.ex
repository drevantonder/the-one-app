defmodule OneCore.Mcp do
  @moduledoc """
  Answers what a client asks, in MCP's JSON-RPC: a message in, and its answer out. So a server is an MCP server to
  whatever reaches it, and the tools it lists are One Core's own and those of the plugins installed for whoever asks
  and for the network.

  Every request is from someone and for someone, which `OneCore.User.asking/2` says, and a plugin's tool runs for
  them.

  Servers ask each other the same way, and four things only a server acting for whoever it asks for may ask: to call a
  function of a plugin's service running here (`services/call`), and to run what was passed on of a service's
  call into its plugin (`plugins/call`), of a plugin's hook (`hooks/call`) or of having a plugin's scheduled jobs
  called (`plugins/schedule`).

  What of a plugin runs in a client, its folder `client/`, is MCP resources: `ui://<plugin>/<code>/<path>`, where
  `code` names the set of files a version runs, so a file at an address never changes. What a page loads is listed,
  a plugin's apps among it as an MCP App is, and the rest of `client/` is read by name. The UI library every app
  loads is One Core's own, the one this server ships with, at `ui://ui/<its code>/client/<path>` (`OneCore.App`).

  Skills are served as MCP's Skills Extension says (SEP-2640), in the shapes it gives for the protocol versions a
  server speaks: `skills/list` lists them, `skills/get` gives one by its address, and each of a skill's files is
  a resource too, `skill://<path>/<file>`, which `resources/read` reads (`OneCore.Skills`).

  A tool its plugin makes a task (`task_support`) is answered as MCP's Tasks have it, to a client that says it takes
  them (`OneCore.Tasks`), and every call is in a state while it runs (`OneCore.Call.State`): a client cancels one
  that isn't a task with `notifications/cancelled`, and is told what it's doing in its progress notifications.

  An agent's client chooses how it calls tools, by what its requests say (`_meta.mode`), which the CLI says for
  `one mcp --code` and the network's address for `/mcp?code`. With none, every tool is listed, and `skills`, and the
  client finds tools itself, as Claude Code's tool search and the model APIs' deferred tools do. In code mode, as
  Executor's MCP server has it, only `search`, which finds them (`OneCore.Search`), `execute`, which runs a script that
  calls them, `call`, which calls them without one (`OneCore.CodeMode`), and `skills` (`OneCore.Skills`). `one search`
  says `cli`, for calls written as commands.
  """
  require Logger

  alias OneCore.{
    App,
    Artifact,
    Builds,
    CodeMode,
    Edge,
    Hooks,
    Plugins,
    Search,
    Server,
    Servers,
    Services,
    Skills,
    Tasks,
    Tools,
    Trace,
    User
  }

  alias OneCore.Call.State
  alias OneCore.Plugins.{Calls, Deploy, Store, Version}

  # The rules every agent is told first, here and in the network's `/llms.txt` (`core/edge/mcp/mcp.ts`).
  @external_resource Path.expand("../../priv/rules.md", __DIR__)
  @rules File.read!(Path.expand("../../priv/rules.md", __DIR__))

  # Claude Code keeps only the first 4,096 characters of a server's instructions, so they stay under that.
  @instructions_limit 4000

  # What a server speaks of MCP is the same in each of these. A client of the versions since 2026-07-28 says its
  # version in each request (`_meta`), and one of the earlier ones in `initialize`.
  @versions ~w(2026-07-28)
  @initialized ~w(2025-11-25 2025-06-18 2025-03-26 2024-11-05)

  @capabilities %{
    tools: %{},
    resources: %{},
    extensions: Map.merge(%{"io.modelcontextprotocol/skills" => %{}}, Tasks.extension())
  }

  # What a client keeps a list or a file of these for, before it asks again: none of it, since who's asking and
  # what's installed for them changes what's there.
  @cacheable ~w(server/discover tools/list resources/list resources/read)

  @doc """
  The answer to one message from whoever holds the key `from`, as JSON. A notification's is empty. `ask` is the
  message as this server took it, which says what a call it asks for is doing while it runs (`OneCore.Iroh`).
  """
  def handle(message, from, ask \\ nil) do
    case JSON.decode(message) do
      {:ok, %{"method" => method, "id" => id} = request} ->
        params = Map.get(request, "params", %{})
        # What a call it asks for is cancelled by, and where its progress is told.
        Process.put(:"$one_request", %{request: {from, User.device(from, meta(params)), id}, ask: ask})

        version = meta(params)["io.modelcontextprotocol/protocolVersion"]

        try do
          Trace.follow(meta(params)["traceparent"])

          with :ok <- speaks(version),
               :ok <- talks(method, params),
               {:ok, principal, forwarded} <- User.asking(from, asking(method, meta(params)), method == "hooks/call"),
               {:ok, result} <- result(method, params, from, principal, forwarded) do
            JSON.encode!(%{jsonrpc: "2.0", id: id, result: complete(result, method, version)})
          else
            {:error, why} -> JSON.encode!(%{jsonrpc: "2.0", id: id, error: %{code: -32000, message: why}})
            {:error, code, why} -> JSON.encode!(%{jsonrpc: "2.0", id: id, error: %{code: code, message: why}})
            {:error, code, why, data} -> JSON.encode!(%{jsonrpc: "2.0", id: id, error: %{code: code, message: why, data: data}})
          end
        rescue
          error ->
            Logger.error(OneCore.Redact.format(:error, error, __STACKTRACE__))
            message = Exception.message(OneCore.Redact.redact(error))
            JSON.encode!(%{jsonrpc: "2.0", id: id, error: %{code: -32603, message: message}})
        catch
          # What exits, as a call that took too long does, is answered too: only a server that went away answers
          # nothing.
          :exit, why ->
            Logger.error(OneCore.Redact.format(:exit, why, __STACKTRACE__))
            message = Exception.format_exit(OneCore.Redact.redact(why))
            JSON.encode!(%{jsonrpc: "2.0", id: id, error: %{code: -32603, message: message}})
        end

      # A client saying an app of its is on screen, and where a server reaches it, or with no address, that none is.
      {:ok, %{"method" => "notifications/one/watch"} = notification} when from != nil ->
        Server.watch(from, notification["params"]["address"])
        ""

      # A client saying what it measured of itself, as from the device and person its request proves it is, which
      # is all that's believed of who said it.
      {:ok, %{"method" => "notifications/one/measured", "params" => %{} = params}} ->
        with {:ok, {by, _for}, false} <- User.asking(from, meta(params)) do
          about = %{"records" => List.wrap(params["records"]), "device" => User.device(from, meta(params)), "who" => by}
          Hooks.observe("measured", about, {nil, "network"})
        end

        ""

      # The relay at the network's address saying one of its servers came or went.
      {:ok, %{"method" => "notifications/one/seen"}} ->
        Servers.seen_again()
        ""

      # Another server, or a plugin's edge function, saying that what this one kept of some hooks
      # was decided from what has since changed.
      {:ok, %{"method" => "notifications/one/invalidate"} = notification} ->
        Hooks.invalidate(for hook <- List.wrap(notification["params"]["hooks"]), is_binary(hook), do: hook)
        ""

      # Another server saying there's something new in the bucket for some owners, or with none, this server saying
      # it can reach its network's address again.
      {:ok, %{"method" => "notifications/one/reload"} = notification} ->
        case notification["params"]["owners"] do
          nil -> Server.reconnected()
          owners -> for owner <- List.wrap(owners), User.owner?(owner), do: Server.reload(owner)
        end

        ""

      # Another server saying where a plugin is in its life for someone moved, which this one's clients ask again.
      {:ok, %{"method" => "notifications/one/changed"}} ->
        Server.changed()
        ""

      # A client cancelling a call it asked for that isn't a task, by the request's id, as only it may.
      {:ok, %{"method" => "notifications/cancelled", "params" => %{"requestId" => id} = params}} ->
        State.cancel_request({from, User.device(from, meta(params)), id})
        ""

      {:ok, %{"method" => _notification}} ->
        ""

      _ ->
        JSON.encode!(%{jsonrpc: "2.0", id: nil, error: %{code: -32700, message: "That isn't a JSON-RPC request"}})
    end
  end

  # What a request says of itself.
  defp meta(%{"_meta" => %{} = meta}), do: meta
  defp meta(_params), do: %{}

  # Who a request is from and for, as it says. A task is only whoever started it's, so a client checking on one, or
  # answering or cancelling it, asks as themselves, whoever it runs for: that's what the task says, not the request.
  defp asking("tasks/" <> _method, %{"actor" => _actor} = meta), do: meta
  defp asking("tasks/" <> _method, meta), do: Map.delete(meta, "subject")
  defp asking(_method, meta), do: meta

  # A request with no version is of a client that said its version in `initialize`, or of a server.
  defp speaks(nil), do: :ok
  defp speaks(version) when version in @versions, do: :ok

  defp speaks(version),
    do: {:error, -32022, "Unsupported protocol version", %{supported: @versions, requested: version}}

  # Parts of the platform talk only on versions that talk with each other (`OneCore.Builds.talk?/2`): a server, a
  # client and the edge each say their build as `_meta.version`, and what one of another says is refused before
  # anything of it is read, as MCP refuses a protocol version it doesn't speak (-32022). One that says none is from
  # before builds had versions, 0.15's. What updates a part that's behind, the builds it downloads and a server's
  # update, is answered whatever it's on, since that's how it catches up, and so are the list of tools a client
  # reads before it calls one and which builds run, by which the program keeping a server running, still the build
  # before, tells a build it tries answers (`clients/cli/src/serving.rs`).
  defp talks("tools/list", _params), do: :ok
  defp talks("tools/call", %{"name" => name}) when name in ["builds", "download", "update"], do: :ok

  defp talks(_method, params) do
    theirs = meta(params)["version"] || "0.15.0"

    if Builds.talk?(theirs, Builds.build()) do
      :ok
    else
      here = Servers.name(Server.key()) || "This server"

      why =
        if Builds.older?(theirs, Builds.build()),
          do:
            "#{here} is on #{Builds.build()}, and what asked on #{theirs}, which doesn't talk with it until it's updated (`one update`)",
          else: "#{here} is on #{Builds.build()}, behind what asked on #{theirs}: it's asked once it's updated"

      {:error, -32022, why, %{version: Builds.build(), requested: theirs}}
    end
  end

  # A result to a client of a version that says what kind of result it is, and how long it may keep it.
  defp complete(result, _method, nil), do: result

  defp complete(result, method, _version) when is_map(result) do
    result = put_new(result, :resultType, "complete")
    if method in @cacheable, do: result |> put_new(:ttlMs, 0) |> put_new(:cacheScope, "private"), else: result
  end

  defp complete(result, _method, _version), do: result

  # A result is this server's, keyed by atoms, or another server's that it passed on, keyed by strings.
  defp put_new(result, key, value) do
    if Map.has_key?(result, key) or Map.has_key?(result, Atom.to_string(key)),
      do: result,
      else: Map.put(result, key, value)
  end

  defp result("services/call", %{"plugin" => plugin, "service" => service} = params, _from, _principal, forwarded) do
    between(forwarded, fn -> Services.run(plugin, service, params["function"], params["arguments"]) end)
  end

  defp result("plugins/call", %{"installed" => at, "plugin" => plugin} = params, _from, principal, forwarded) do
    function = fn -> Calls.function(at, plugin, params["function"], params["arguments"], principal, true) end

    with {:ok, result} <- between(forwarded, function) do
      Server.changed()
      {:ok, result}
    end
  end

  defp result("plugins/schedule", %{"installed" => at, "plugin" => plugin}, _from, _principal, forwarded) do
    between(forwarded, fn -> Calls.schedule(at, plugin, true) end)
  end

  defp result("hooks/call", %{"installed" => at, "plugin" => plugin, "kind" => kind, "hook" => hook} = params, _from, principal, forwarded)
       when kind in ["observe", "mutate", "validate", "collect"] do
    between(forwarded, fn -> Calls.hook(at, plugin, String.to_existing_atom(kind), hook, params["about"], principal, true) end)
  end

  # `search`, `execute`, `call` and `skills` are this server's own, and a sharing link's holder has only the tools
  # it's given.
  defp result("tools/call", %{"name" => name} = params, from, {actor, _subject} = principal, forwarded)
       when name in ["search", "execute", "call", "skills"] do
    tool = Enum.find([Search.tool(), Skills.tool() | CodeMode.tools()], &(&1.name == name))
    inputs = Map.get(params, "arguments") || %{}

    cond do
      holder?(actor) ->
        {:error, -32602, "There's no tool #{name} for a sharing link's holder"}

      not ExJsonSchema.Validator.valid?(
        ExJsonSchema.Schema.resolve(JSON.decode!(JSON.encode!(tool.inputSchema))),
        inputs
      ) ->
        {:ok, %{content: [%{type: "text", text: "#{name} takes #{JSON.encode!(tool.inputSchema)}"}], isError: true}}

      true ->
        own(name, inputs, params, from, principal, forwarded)
    end
  end

  # Which plugin called a tool is said only by a server, which `OneCore.User.asking/3` checked acts for whoever it
  # asks for (a request that says its `actor`). A client calls as nobody's plugin. A tool its plugin makes a task is
  # one for a person's client that takes tasks, and refused to one that doesn't if it needs one. A script's calls
  # are never tasks, since it waits on each.
  defp result("tools/call", %{"name" => name} = params, _from, {actor, subject} = principal, forwarded) do
    caller = if Map.has_key?(meta(params), "actor"), do: meta(params)["caller"]

    run = fn ->
      rendered(name, call(name, Map.get(params, "arguments", %{}), principal, forwarded, caller), principal)
    end

    case Tasks.support(tool(name, subject)) do
      support when support != nil and is_binary(actor) and caller == nil and not is_map_key(params, :script) ->
        cond do
          Tasks.capable?(params) and not User.link?(actor) -> Tasks.start(run, name, principal)
          support == :required -> Tasks.required()
          true -> plainly(run)
        end

      _never ->
        plainly(run)
    end
  end

  defp result("tasks/" <> method, params, _from, principal, forwarded) when method in ["get", "update", "cancel"] do
    cond do
      not Tasks.capable?(params) -> Tasks.required()
      true -> with({:forwarded, result} <- Tasks.handle(method, params, principal, forwarded), do: {:ok, result})
    end
  end

  defp result("initialize", params, _from, {actor, subject}, _forwarded) do
    requested = params["protocolVersion"]

    {:ok,
     %{
       protocolVersion: if(requested in @initialized, do: requested, else: hd(@initialized)),
       capabilities: @capabilities,
       serverInfo: server_info(),
       instructions: instructions(if(holder?(actor), do: :holder, else: mode(params)), subject)
     }}
  end

  # What a client of a version since 2026-07-28 may ask first, or to tell whether a server speaks one at all.
  defp result("server/discover", params, _from, {actor, subject}, _forwarded) do
    {:ok,
     %{
       supportedVersions: @versions,
       capabilities: @capabilities,
       _meta: %{"io.modelcontextprotocol/serverInfo" => server_info()},
       instructions: instructions(if(holder?(actor), do: :holder, else: mode(params)), subject)
     }}
  end

  defp result("ping", _params, _from, _principal, _forwarded), do: {:ok, %{}}

  # A user's device asks for its token to the relay at the network's address, and for the next before that one
  # ends, and is told whose it is: the device a run of it proved here, or the edge did there. It says which
  # build it is as it asks, which is kept in the bucket after it's answered: a device waits a few seconds for its
  # token, and a slow bucket would otherwise keep it from joining at all.
  defp result("one/token", params, from, {"users/" <> _ = by, _for}, _forwarded) do
    device = User.device(from, meta(params))

    if not User.link?(by) and is_binary(device) and device =~ ~r/^[0-9a-f]{64}$/ do
      Task.Supervisor.start_child(OneCore.Asks, fn -> Builds.seen(by, device, params["build"]) end)
      {:ok, %{token: Edge.token(device), who: by}}
    else
      {:error, "A token is for a user's device, by its key"}
    end
  end

  # A sharing link's holder is told of the tools they may call, and of nothing else that's in the network.
  defp result("tools/list", params, _from, {actor, subject}, _forwarded) do
    listed = listed(subject, holder?(actor))

    tools =
      cond do
        holder?(actor) -> listed
        mode(params) == :code -> [Search.tool() | CodeMode.tools()] ++ [Skills.tool()]
        true -> [Skills.tool() | listed]
      end

    {:ok, %{tools: tools}}
  end

  defp result("skills/list", _params, _from, principal, _forwarded),
    do: {:ok, %{resultType: "complete", skills: skills(principal)}}

  defp result("skills/get", params, _from, principal, _forwarded) do
    case Enum.find(skills(principal), &(&1.uri == params["uri"])) do
      nil -> {:error, -32602, "There's no skill at #{params["uri"]}. skills/list says what there is."}
      skill -> {:ok, %{resultType: "complete", skill: skill}}
    end
  end

  defp result("resources/" <> _ = method, params, _from, {actor, subject}, _forwarded) do
    if holder?(actor),
      do: {:error, -32002, "A sharing link's holder reads nothing of a network but what a tool gives them"},
      else: resources(method, params, subject)
  end

  defp result(method, _params, _from, _principal, _forwarded),
    do: {:error, -32601, "This server has no method #{method}"}

  # A skill is listed here too, for an agent whose client lists resources and not yet skills.
  defp resources("resources/list", _params, who) do
    skills =
      for %{uri: uri, frontmatter: %{"name" => name, "description" => description}} <- Skills.list(who),
          do: %{uri: uri, name: name, description: description, mimeType: "text/markdown"}

    running = Enum.sort(Plugins.running(who))

    uis =
      Map.new(for {plugin, version} <- running, %{ui: ui} when is_binary(ui) <- version.tools, do: {{plugin, ui}, true})

    %{files: files, code: code} = App.library()

    library =
      for path <- Enum.sort(Map.keys(files)), Version.loaded?(path),
          do: %{uri: "ui://ui/#{code}/#{path}", name: path, mimeType: Version.kind(path)}

    resources =
      for {plugin, version} <- running, path <- Version.files(version) do
        resource = %{uri: "ui://#{plugin}/#{version.code}/#{path}", name: path, mimeType: Version.kind(path)}

        # An app's category goes as it is: a client opens the launcher first, and what the rest mean is the
        # launcher's. A page that's a tool's UI is an MCP App's page too.
        case Enum.find(version.apps, &(&1.page == path)) do
          nil when is_map_key(uis, {plugin, path}) ->
            resource

          nil ->
            %{resource | mimeType: String.replace_suffix(resource.mimeType, ";profile=mcp-app", "")}

          %{category: nil} = app ->
            Map.merge(resource, %{name: app.name, icons: [%{src: app.icon}]})

          app ->
            Map.merge(resource, %{name: app.name, icons: [%{src: app.icon}], _meta: %{category: app.category}})
        end
      end

    {:ok, %{resources: skills ++ library ++ resources}}
  end

  defp resources("resources/read", %{"uri" => "skill://" <> _ = uri}, who) do
    case Skills.text(who, uri) do
      {:ok, text} -> {:ok, %{contents: [%{uri: uri, mimeType: "text/markdown", text: text}]}}
      :absent -> {:error, -32602, "There's no skill at #{uri}. skills/list says what there is."}
    end
  end

  defp resources("resources/read", %{"uri" => "ui://ui/" <> _ = uri}, _who) do
    library = App.library()

    with ["ui:", "", "ui", code, path] when code == library.code <- String.split(uri, "/", parts: 5),
         true <- Version.client?(path),
         %{^path => data} <- library.files do
      {:ok, %{contents: [Map.merge(%{uri: uri, mimeType: Version.kind(path)}, content(data))]}}
    else
      _ -> {:error, -32002, "There's nothing at #{uri}"}
    end
  end

  defp resources("resources/read", %{"uri" => uri}, who) do
    with ["ui:", "", plugin, code, path] <- String.split(uri, "/", parts: 5),
         true <- Version.client?(path),
         %{^plugin => version} <- Plugins.running(who),
         {:ok, data} <- Store.file(version.artifact, code, path) do
      data = if Path.extname(path) == ".html", do: mapped(data, path, [plugin | Map.keys(version.requires)]), else: data
      {:ok, %{contents: [Map.merge(%{uri: uri, mimeType: Version.kind(path)}, content(data))]}}
    else
      _ -> {:error, -32002, "There's nothing at #{uri}"}
    end
  end

  defp resources(method, _params, _who), do: {:error, -32601, "This server has no method #{method}"}

  # A page, with the import map its modules are resolved by in its head, before anything loads: what every app shares
  # by name, the UI library's one copy, which an app's build already imports by where it's served (`core/ui/app.js`),
  # and as `<plugin>/` the client of its own plugin and of each plugin it requires, which a client serves beside it, at
  # <its address>/<plugin>/client/, for what's imported by name as it runs. So separately built plugins share one
  # Solid. It depends on nothing but the page's own version, so the page is still read once.
  @shared %{
    "solid-js" => "/ui/client/solid.js",
    "@solidjs/web" => "/ui/client/web.js",
    "@zag-js/solid" => "/ui/client/zag.js",
    "ui" => "/ui/client/ui.js"
  }
  defp mapped(page, path, plugins) do
    up = String.duplicate("../", length(Path.split(path)))
    imports = Map.merge(@shared, Map.new(plugins, &{"#{&1}/", "#{up}#{&1}/client/"}))
    map = ~s(<script type="importmap">#{JSON.encode!(%{imports: imports})}</script>)

    case Regex.split(~r/<!doctype html>|<head(?:\s[^>]*)?>/i, page, parts: 2, include_captures: true) do
      [before, tag, rest] -> before <> tag <> map <> rest
      [_page] -> map <> page
    end
  end

  # A file that isn't text, such as a picture or a font, is base64 as MCP's `blob`.
  defp content(data), do: if(String.valid?(data), do: %{text: data}, else: %{blob: Base.encode64(data)})

  # What one server asks of another is run only for a server that says who it asks for, which only one acting
  # for them is believed about. This server runs it if it acts for whoever it's for too.
  defp between(forwarded, run) when forwarded in [true, :to] do
    case run.() do
      {:ok, result} -> {:ok, %{answer: result}}
      {:error, why} -> {:error, -32000, why}
      {:forwarded, result} -> {:ok, result}
      :forwarding_loop -> {:error, Server.forwarding_loop(), "This server would pass that on in turn"}
    end
  end

  defp between(_forwarded, _run), do: {:error, -32000, "Only a server acting for whoever it asks for may ask that"}

  # What anyone has deployed is read from the bucket, meanwhile.
  defp own("search", %{"query" => query} = inputs, params, _from, {_actor, subject}, _forwarded) do
    deployed = Task.async(&Deploy.deployed/0)
    tools = scripted(subject)
    deployed = Task.await(deployed, 30_000)
    text = Search.run(query, Map.get(inputs, "limit", 8), tools, Plugins.running(subject), deployed, mode(params))
    {:ok, %{content: [%{type: "text", text: text}], structuredContent: %{text: text}}}
  end

  defp own("skills", inputs, _params, _from, {_actor, subject}, _forwarded) do
    case Skills.run(inputs, subject) do
      {:ok, %{text: text} = read} -> {:ok, %{content: [%{type: "text", text: text}], structuredContent: read}}
      {:ok, listed} -> {:ok, %{content: [%{type: "text", text: JSON.encode!(listed)}], structuredContent: listed}}
      {:error, why} -> {:ok, %{content: [%{type: "text", text: why}], isError: true}}
    end
  end

  # A script's calls are each answered here as that call of the tool would be, for whoever asked, and so are
  # `call`'s. A plugin's tools in a script are in a namespace of its name, which says what the plugin is for.
  defp own("execute", %{"code" => code}, _params, from, {_actor, subject} = principal, forwarded) do
    tools =
      for tool <- scripted(subject),
          do: %{
            name: tool.name,
            plugin: tool[:_meta][:plugin],
            description: tool.description,
            input: tool.inputSchema,
            output: tool[:outputSchema]
          }

    namespaces = Map.new(Plugins.running(subject), fn {name, version} -> {name, Search.described(version)} end)
    {done, text} = CodeMode.run(code, tools, namespaces, called(from, principal, forwarded))
    {:ok, %{content: [%{type: "text", text: text}], structuredContent: %{text: text}, isError: done == :error}}
  end

  defp own("call", %{"calls" => calls}, _params, from, {_actor, subject} = principal, forwarded) do
    # A path as search gives it, or without its `tools.`, as a script's `search()` gives it.
    named =
      Map.new(
        for tool <- scripted(subject),
            dotted = dotted(tool),
            path <- [Search.path(dotted), dotted],
            do: {path, tool.name}
      )

    {:ok, CodeMode.calls(calls, named, called(from, principal, forwarded))}
  end

  # A script waits on each call it makes, so none is a task.
  defp called(from, principal, forwarded),
    do: fn name, inputs ->
      result("tools/call", %{"name" => name, "arguments" => inputs, script: true}, from, principal, forwarded)
    end

  # A tool's path in a script, `gmail.search` or `deploy`.
  defp dotted(%{_meta: %{plugin: plugin}} = tool), do: plugin <> "." <> String.replace_prefix(tool.name, plugin <> "_", "")
  defp dotted(tool), do: tool.name

  # The tools a script has, and that `search` finds among: those every client is listed but code mode's own.
  defp scripted(who), do: [Skills.tool() | listed(who, false)]

  # How whoever asks calls tools: as MCP's tools, in a code mode script, or as commands.
  defp mode(params) do
    case meta(params)["mode"] do
      "code" -> :code
      "cli" -> :cli
      _ -> :mcp
    end
  end

  # Every tool there is for `who`, One Core's own first, or only those a sharing link's holder may call. A plugin's
  # says whose it is, and one with a page of its own names it as an MCP App's tool names its UI.
  defp listed(who, holder) do
    open = fn tools -> if holder, do: Enum.filter(tools, &(&1[:by_link] == true)), else: tools end

    listed = fn name, tool ->
      listed = %{
        name: name,
        description: tool.description,
        inputSchema: tool.inputs,
        annotations: %{readOnlyHint: tool[:read_only] == true}
      }

      if tool[:outputs], do: Map.put(listed, :outputSchema, tool.outputs), else: listed
    end

    own = for tool <- open.(Tools.tools()), do: listed.(tool.name, tool)

    plugins =
      for {plugin, version} <- Enum.sort(Plugins.running(who)), tool <- open.(version.tools) do
        ui = if tool[:ui], do: %{ui: %{resourceUri: "ui://#{plugin}/#{version.code}/#{tool.ui}"}}, else: %{}
        Map.put(listed.("#{plugin}_#{tool.name}", tool), :_meta, Map.put(ui, :plugin, plugin))
      end

    own ++ plugins
  end

  # A call that isn't a task, run in the process answering it, which its client cancels by the request's id. One
  # cancelled is answered as MCP has a cancelled request: it's said, and its client has stopped listening.
  defp plainly(run) do
    asked = Process.get(:"$one_request", %{})
    id = State.begin(Map.take(asked, [:request, :ask]))

    try do
      run.()
    after
      State.finish(id)
    end
  end

  # What MCP's `tools/call` answers of a call: its answer, the error it ended with, or another server's answer.
  defp rendered(name, outcome, principal) do
    case outcome do
      {:ok, result} -> changed(name, answer(name, result), principal)
      {:error, why} -> {:ok, %{content: [%{type: "text", text: why}], isError: true}}
      # Another server ran it, and told its own clients.
      {:forwarded, %{"isError" => true} = result} -> {:ok, result}
      {:forwarded, result} -> changed(name, result, principal)
      :forwarding_loop -> {:error, Server.forwarding_loop(), "This server would pass #{name} on in turn"}
    end
  end

  # The tool `name` is for `who`: One Core's own, or a plugin's by its name and the plugin's, or `nil`.
  defp tool(name, who) do
    Enum.find(Tools.tools(), &(&1.name == name)) ||
      with [plugin, tool] <- String.split(name, "_", parts: 2),
           %{^plugin => version} <- Plugins.running(who) do
        Enum.find(version.tools, &(&1.name == tool))
      else
        _ -> nil
      end
  end

  # A tool is one of One Core's own, or a plugin's by its name and the plugin's.
  defp call(name, inputs, principal, forwarded, caller) do
    with :none <- Tools.call(name, inputs, principal, forwarded),
         [plugin, tool] <- String.split(name, "_", parts: 2),
         {:ok, result} <- Calls.call(plugin, tool, inputs, principal, forwarded, caller) do
      {:ok, result}
    else
      [_] -> {:error, "#{name} isn't a tool. tools/list says what there is."}
      other -> other
    end
  end

  # An answer as MCP's result: what it is as JSON, `structuredContent`, with the same as text for a client from before
  # that, as MCP says. A file is content of its kind, and its `structuredContent` says what kind of file it is. A
  # tool of a version from before tools said what they answer may answer text, which is only text.
  defp answer(_name, text) when is_binary(text), do: %{content: [%{type: "text", text: text}]}

  defp answer(name, file) when is_tuple(file), do: %{content: file(name, file), structuredContent: Calls.structured(file)}

  defp answer(_name, answer) do
    structured = Calls.structured(answer)
    result = %{content: [%{type: "text", text: JSON.encode!(structured)}]}
    if is_map(structured), do: Map.put(result, :structuredContent, structured), else: result
  end

  # A picture is an image, which an agent that sees pictures sees, and any other file is a resource inside the answer,
  # unless it's large.
  defp file(_name, {:blob, "image/" <> _ = kind, data}),
    do: [%{type: "image", mimeType: kind, data: Base.encode64(data)}]

  defp file(name, {:blob, kind, data}),
    do: [%{type: "resource", resource: %{uri: "one://#{name}", mimeType: kind, blob: Base.encode64(data)}}]

  # A large file is a short-lived link to its blob in the bucket, under the blob's key, which never names another.
  defp file(_name, {:file, kind, %Artifact{prefix: prefix} = artifact, blob}) do
    [%{type: "resource_link", uri: Artifact.link(artifact, blob, :read).url, name: prefix <> blob, mimeType: kind}]
  end

  # After a tool that doesn't only read, every client connected here is told that something changed.
  defp changed(name, result, {_actor, subject}) do
    if not reads?(name, subject), do: Server.changed()
    {:ok, result}
  end

  @doc "Whether calling the tool `name` for `who` only reads: one of One Core's own that says so, or a plugin's."
  def reads?(name, who) do
    with [plugin, tool] <- String.split(name, "_", parts: 2),
         %{^plugin => version} <- Plugins.running(who) do
      Enum.any?(version.tools, &(&1.name == tool and &1[:read_only] == true))
    else
      _ -> Enum.any?(Tools.tools(), &(&1.name == name and &1[:read_only] == true))
    end
  end

  defp holder?(by), do: by != nil and User.link?(by)

  defp server_info, do: %{name: "one", version: Builds.build()}

  # A sharing link's holder sees no skill.
  defp skills({actor, subject}), do: if(holder?(actor), do: [], else: Skills.list(subject))

  # What an agent is told when it connects: the rules first, then how it finds tools as it calls them, then where
  # the skills are, which say the rest, and the plugins there are, as many as fit. Not every client lists skills yet,
  # and any reads one that it has the address of.
  defp instructions(mode, who) do
    network = """
    This is a One App network: what one user, family or workplace keeps and makes, which you use as whoever you act \
    for. Its tools are its plugins', each named <plugin>_<tool>, and One Core's own, which add and remove plugins, \
    servers and services.

    Rules, before anything else:
    #{String.trim(@rules)}\
    """

    finding =
      case mode do
        :code ->
          """
          `search` finds the tools for a task across every plugin, with each one's exact call in a script and its \
          plugin's skill: search before you guess, to learn a plugin, and before saying you can't, since it offers \
          plugins you could install. `call` calls one or several at once, by their paths, and `execute` runs a short \
          script where one call's answer feeds the next, `const r = await tools.<plugin>.<tool>({ ... }); return r`: \
          what it returns comes back to you. Skills name tools <plugin>_<tool>, which in a script is \
          tools.<plugin>.<tool>, and One Core's own are tools.<tool>.\
          """

        _ ->
          """
          Every tool is listed, and your client's own tool search finds them. Where `one` runs, every tool is also a \
          command: gmail_search is `one gmail search`, and `one search <words>` finds the tools for a task and offers \
          plugins you could install: use it before saying you can't.\
          """
      end

    skills = """
    A plugin's skill says when and how to use its tools, and the skill building-plugins how to make a plugin: when \
    the person asks for an app, tracker or tool, make it here, where it's on all their devices. `skills` lists them \
    and reads one, as do skills/list and resources/read: a plugin's are under skill://<plugin>/, and building-plugins \
    is skill://building-plugins/SKILL.md.\
    """

    case mode do
      :holder -> network
      _ -> fitted(Enum.join([network, finding, skills], "\n\n"), Enum.sort(Map.keys(Plugins.running(who))))
    end
  end

  # The instructions, with the plugins there are, as many as fit under the limit.
  defp fitted(text, []), do: text

  defp fitted(text, plugins) do
    more = ", and more: `skills` lists them."
    instructions = text <> "\n\nThe plugins here, whose tools start with their names: "

    Enum.reduce_while(plugins, {instructions, ""}, fn plugin, {instructions, sep} ->
      if length16(instructions <> sep <> plugin <> more) > @instructions_limit,
        do: {:halt, {instructions <> more, :full}},
        else: {:cont, {instructions <> sep <> plugin, ", "}}
    end)
    |> case do
      {instructions, :full} -> instructions
      {instructions, _sep} -> instructions <> "."
    end
  end

  # What Claude Code counts a length in: UTF-16's units.
  defp length16(text), do: div(byte_size(:unicode.characters_to_binary(text, :utf8, :utf16)), 2)
end
