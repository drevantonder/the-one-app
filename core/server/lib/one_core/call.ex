defmodule OneCore.Call do
  @moduledoc """
  What a tool is called with besides its inputs.

  - `own`: the app's own `OneCore.Artifact` for whoever this call runs for, or for an app that says its data is
    whoever it was installed for's (`own/0`), theirs (`OneCore.Apps.Calls.whose/3`). Only this app reaches it.
  - `installed`: who the app was installed for, whose `own` is: a user, a group or the network. It's who the app
    runs for with nobody there, as a scheduled job and a service do.
  - `folder`: a folder on the machine running this call that is the app's own, as `own` is, for what it keeps there. It may not be there yet, and another server has another.
  - `server`: the key of the server running this call.
  - `subject`: who this call runs for, as relation tuples name them: the user who asked, or a sharing link's holder,
    or the group or the network they asked for. What it makes is theirs, and it reaches what they may. A
    scheduled job runs for whoever its app was installed for.
  - `actor`: who asked: a user, or a sharing link's holder. They're a member of whoever the call is for. A scheduled
    job is called by the clock, so by nobody: `nil`.
  - `caller`: the app that called this tool (`tool/5`), as its artifact's name, or the app that one acts on
    behalf of. One Core sets it from whose call made the call, wherever it runs, never from what's asked. A tool
    called by a person, an app or an agent has none: `nil`.

  The rest is One Core's: the app's name where it was installed and the app as its artifact's name (`app_artifact`), what was asked of it, the
  apps it requires whose code runs inside it (`loaded`), whether another server passed it on, and the state it's in
  (`OneCore.Call.State`).

  A call is passed between servers at most twice, so none goes round. A server that can't run it has one that can
  run it instead (`forward/2`): `forwarded` is then `:to`. That one may still hand it to the one server that holds
  what it needs (`forward_to/2`): `forwarded` is then `true`, and it goes no further.
  """
  alias OneCore.{Artifact, Bucket, Hooks, Apps, Server, Servers, Tasks, User, Writes}
  alias OneCore.Call.State
  alias OneCore.Apps.{Calls, Version}

  @enforce_keys [:own, :installed, :folder, :server, :subject, :actor, :app, :app_artifact, :request, :loaded, :forwarded]
  defstruct [
    :own,
    :installed,
    :folder,
    :server,
    :subject,
    :actor,
    :caller,
    :app,
    :app_artifact,
    :request,
    :loaded,
    :forwarded,
    :state
  ]

  @type t :: %__MODULE__{own: OneCore.Artifact.t(), folder: String.t(), server: String.t(), subject: String.t()}

  defmodule Refused do
    @moduledoc """
    What a tool raises when what it was asked can't be done and nothing failed: what it was given is wrong in a way
    its inputs' schema can't say (`raise OneCore.Call.Refused, "Too long: 619 bytes, limit 280"`), or the device it
    needs is off, as a Mac that's asleep. Whoever asked is told the message, as for `raise "..."`, and the call's
    outcome is `refused`, as one refused for its inputs before it ran: not an error, as OpenTelemetry has a
    server's span for an RPC refused for its arguments.
    """
    defexception [:message]
  end

  @doc """
  Which client asked for what this call is part of: `%{"device" => key, "web" => web, "name" => name, "title" =>
  title}`, the device it came from (`OneCore.User.device/2`), whose kind and name the apps that sign devices in keep;
  whether it came in at the network's address over the web, as a browser and an assistant somewhere else do, rather
  than over a connection of a device's own; and the name and title its MCP client says it goes by (its
  `clientInfo`), where it says. `nil` for what no client asked, such
  as a scheduled job. What a client says of itself is only what it says: who it acts for is `actor`.
  """
  def client(%__MODULE__{}), do: State.client()

  @doc """
  An app this call's app requires, or that one of those requires, whose module provides functions
  (`OneCore.App`): its module, whose functions take this call and run as this call's app, for whoever it runs
  for.

      database = OneCore.Call.app(call, "database")
  """
  def app(%__MODULE__{loaded: loaded, app: app}, name) do
    case loaded do
      %{^name => {_artifact, module}} ->
        module

      _ ->
        raise "#{name}'s code doesn't run inside #{app}: it does where #{app} requires it (requires/0), it's " <>
                "installed for whoever #{app} runs for, it's trusted, and its module provides functions. Otherwise call its " <>
               "tools."
    end
  end

  @doc """
  Calls a tool of another app, installed for whoever this call runs for or for the network, as they would call it
  themselves: the same inputs are checked, the same hooks asked, and it runs as that app, with its own artifact.
  Gives its answer as it would reach a client over JSON, a file it answers as the file (`{:blob, kind, data}`), or
  raises with what that app said.

  The tool is told this call's app called it, as its `caller`. A tool that another app called can call one in
  turn on behalf of that app, as OAuth's on-behalf-of flow does, with `on_behalf_of: call.caller`: then that
  one is its `caller`, so an app that does something for others, as `google-account` does, uses the Vault for
  whichever called it.

      OneCore.Call.tool(call, "notifications", "send", %{"title" => "Lunch is here"})
  """
  def tool(%__MODULE__{actor: actor, subject: subject} = call, app, tool, inputs, options \\ []) do
    caller =
      case Keyword.fetch(options, :on_behalf_of) do
        :error -> call.app_artifact
        {:ok, caller} when caller == call.caller and caller != nil -> caller
        {:ok, _} -> raise "#{call.app} acts on behalf only of the app that called it, and #{inspect(call.caller)} did"
      end

    case Calls.call(app, tool, inputs, {actor, subject}, false, caller) do
      {:ok, result} when is_binary(result) ->
        result

      {:ok, {:blob, _kind, _data} = file} ->
        file

      {:ok, result} ->
        OneCore.JSON.normalize(result)

      {:forwarded, %{"isError" => true, "content" => [%{"text" => why} | _]}} ->
        raise why

      {:forwarded, %{"content" => [%{"type" => "image", "mimeType" => kind, "data" => data}]}} ->
        {:blob, kind, Base.decode64!(data)}

      {:forwarded, %{"content" => [%{"resource" => %{"mimeType" => kind, "blob" => data}}]}} ->
        {:blob, kind, Base.decode64!(data)}

      {:forwarded, %{"structuredContent" => structured}} ->
        structured

      {:forwarded, %{"content" => [%{"text" => text} | _]}} ->
        decoded(text)

      {:error, why} ->
        raise why
    end
  end

  @doc """
  Runs `hook`, one this call's app offers (`offers/0`), for whoever this call runs for, as One Core runs its own:
  the apps installed for them and for the network that hook into `<app>.<hook>` are asked. What it gives is
  the kind's: an observe hook gives `:ok` at once, a mutate hook `{:ok, changes, by}` or `{:refuse, why}`, a
  validate hook `:ok` or `{:refuse, why}`, and a collect hook `{:ok, parts, failed}`: each app's part by its name,
  and why each that couldn't answer couldn't. An app runs only the hooks it offers.

      OneCore.Call.run_hook(call, "removed", %{"artifact" => name, "what" => "restored"})
  """
  def run_hook(%__MODULE__{installed: at, app: app, actor: actor, subject: subject}, hook, about) do
    name = "#{app}.#{hook}"

    case Hooks.offered(name, at) do
      :observe -> Hooks.observe(name, about, {actor, subject})
      :mutate -> Hooks.mutate(name, about, {actor, subject})
      :validate -> Hooks.validate(name, about, {actor, subject})
      :collect -> Hooks.collect(name, about, {actor, subject})
      nil -> raise "#{app} doesn't offer #{hook}. Its offers/0 names the hooks it offers."
    end
  end

  # What another server answered is text: JSON where the tool answered more than text.
  defp decoded(text) do
    case JSON.decode(text) do
      {:ok, result} -> result
      {:error, _} -> text
    end
  end

  @doc """
  An artifact this call's app makes for whoever it runs for, under an `id` the app chooses: letters, digits,
  `_` and `-`. It's theirs, not the app's, and the app reaches it for them without asking. The same `id`
  is the same artifact, so a call that starts again makes it once.

      note = OneCore.Call.make(call, "7")
      OneCore.Artifact.write(note, "text", "Buy milk")
      OneCore.Artifact.name(note)
      #=> "users/alice/made/notes/7"
  """
  def make(%__MODULE__{subject: subject, actor: actor, app: app} = call, id) do
    if User.link?(subject), do: raise("A sharing link's holder makes nothing: nothing in a network is theirs")

    if not (is_binary(id) and id =~ ~r/^[A-Za-z0-9_-]+\z/),
      do: raise(ArgumentError, "an id is letters, digits, _ and -")

    %Artifact{prefix: "#{subject}/made/#{app}/#{id}/", reached: {call.app_artifact, {actor, subject}, "owner"}}
  end

  @doc """
  The artifact named `name`, for a call that may reach it as `relation`: a `viewer` reads it, an `editor` changes
  it too, and an `owner` says who else may. Raises with the reason if it may not.

  An app reaches an artifact only if both it and whoever asked have that relation to it, which is for the apps that
  validate `reach` to say: with none, only the network's first owner reaches it. Nothing is asked about what the app
  made for whoever it runs for (a member asking for a group or the network was asked before the tool ran), and nothing about the app where it made the artifact for someone
  else, or the artifact is its own. Another app's own artifact is never reached: it's always that one app's.

      note = OneCore.Call.reach(call, "users/alice/made/notes/7")
      {:ok, text, _tag} = OneCore.Artifact.read(note, "text")
  """
  def reach(%__MODULE__{subject: subject, actor: actor} = call, name, relation \\ "viewer") do
    if why = refused(call, name, relation), do: raise(why)
    # Its blobs are whoever's it is, whom this server may not act for. One that does runs the call.
    owner = User.owner(name) || "network"
    if not OneCore.Apps.here?(owner), do: forward(call, owner)
    %Artifact{prefix: name <> "/", reached: {call.app_artifact, {actor, subject}, relation}}
  end

  @doc "Whether `reach/3` would give the artifact: whether this call may reach it as `relation`."
  def reaches?(%__MODULE__{} = call, name, relation \\ "viewer"), do: refused(call, name, relation) == nil

  @doc "Why this call may not reach the artifact `name` as `relation`, or `nil` if it may."
  def refused(%__MODULE__{subject: subject, actor: actor, app: app} = call, name, relation)
      when relation in ["viewer", "editor", "owner"] do
    parts = String.split(Bucket.key!(name), "/")

    # What's someone's sits under their prefix by what it is: what an app made for them, or an app's own.
    kind = if hd(parts) == "network", do: Enum.slice(parts, 1, 2), else: Enum.slice(parts, 2, 2)

    # Whoever asked is asked about, so a member asking for a group or the network reaches only what they may
    # themselves. With nobody asking, as for a scheduled job, it's whoever the call runs for.
    asking = actor || subject

    about =
      cond do
        name == Artifact.name(call.own) -> [asking]
        match?(["own" | _], kind) -> nil
        kind == ["made", app] and String.starts_with?(name, subject <> "/") -> []
        kind == ["made", app] -> [asking]
        true -> [asking, call.app_artifact]
      end

    if about == nil do
      "#{name} is an app's own artifact, which only that app reaches"
    else
      Enum.find_value(about, fn about ->
        case Hooks.validate("reach", %{"who" => about, "relation" => relation, "artifact" => name}, {actor, subject}) do
          :ok -> nil
          {:refuse, why} -> why
        end
      end)
    end
  end

  @doc """
  Whether whoever asked is a member of `of`, a group or the network, as the apps that validate `reach` say. With
  nobody asking, as for a scheduled job, it's whoever the call runs for. It's what an app asks before telling
  someone what only a group's members, or the network's, may know.
  """
  def member?(%__MODULE__{subject: subject, actor: actor}, of) do
    asking = actor || subject

    asking == of or
      Hooks.validate("reach", %{"who" => asking, "relation" => "member", "artifact" => of}, {actor, subject}) == :ok
  end

  @doc """
  Whether whoever asked owns whoever this call runs for: themselves, or the group or the network they asked for,
  as its `owner`. It's who changes what's installed for them, so an app asks it before changing what's a group's
  or the network's as a whole, where being a member isn't enough.
  """
  def owns?(%__MODULE__{subject: subject, actor: subject}), do: true
  def owns?(%__MODULE__{actor: nil}), do: false

  def owns?(%__MODULE__{subject: subject, actor: actor}),
    do:
      Hooks.validate("reach", %{"who" => actor, "relation" => "owner", "artifact" => subject}, {actor, subject}) == :ok

  @doc """
  The apps whoever this call runs for finds, theirs and then the network's: each one's `name` there, the
  `app` it is, as its artifact's name, who it was `installed` for, its `title`, `icon`, `subtitle`, `about` and
  `category`, its `entrypoints`, its `widgets` and its `tools`' names and pages (`ui`). A Home Screen page is drawn from
  these.
  """
  def apps(%__MODULE__{subject: subject}) do
    for at <- Enum.reverse(Apps.chain(subject)), {name, version} <- Apps.versions([at]), into: %{} do
      tools = for tool <- version.tools, do: Map.take(tool, [:name, :ui])

      {name,
       Map.merge(OneCore.Apps.Version.listing(version), %{
         name: name,
         app: version.artifact,
         installed: at,
         entrypoints: version.entrypoints,
         widgets: version.widgets,
         tools: tools
       })}
    end
    |> Map.values()
  end

  @doc """
  Every app anyone in the network has deployed, whether or not it's installed for anyone now: each one's `app`,
  as its artifact's name, what a person reads of it where it's listed (`OneCore.Apps.Version.listing/1`: its
  `title`, `icon`, `subtitle`, `about`, `category`, `entrypoints` and `widgets`) as its latest version has it, and the
  git repo it was deployed from (`repo`), the same for each copy of the same app. An app store is drawn from it. Whoever may see one reads and installs it, and so does everyone in a group or
  the network it's in use for (`OneCore.Tools`).
  """
  def deployed(%__MODULE__{}), do: OneCore.Apps.Deploy.deployed()

  @doc """
  Tells every app watching that something changed, so it asks again, as each call that changes something does once
  it's over: for a call that takes a while, as it moves from one state to the next, so its apps show each one.

      OneCore.Call.changed(call)
  """
  def changed(%__MODULE__{app: app}), do: OneCore.Apps.changed([], app)

  @doc """
  Says what the call is doing, a stage of the app's own, such as `"Copying 3 of 12"`: an MCP task's
  `statusMessage`, and the progress a caller waiting on it is told. Only the latest is said. A call that was cancelled
  raises here, so its `try … after` cleans up.

      OneCore.Call.stage(call, "Copying \#{done} of \#{length(files)}")
  """
  def stage(%__MODULE__{state: state}, text) when is_binary(text), do: State.stage(state, text)

  @doc """
  Asks the person the call is for `question`, and gives their answer: the call is `input_required` until it comes,
  and its caller asks them through MCP elicitation. `question` is elicitation's params, a `message` and the
  `requestedSchema` of what to answer, or only a message, to say yes or no to. The answer is elicitation's result:
  `%{"action" => "accept", "content" => %{...}}`, or an `action` of `"decline"` or `"cancel"`. Only a call made as an MCP
  task asks (`task_support`), and one that isn't raises saying so. A call that was cancelled raises here too.

      %{"action" => "accept"} = OneCore.Call.ask(call, "Replace the backup from Tuesday?")
  """
  def ask(%__MODULE__{state: state}, question), do: State.ask(state, Tasks.question(question))

  @doc """
  Makes a web request from the server running this call: its `url`, and its `method`, `headers` and `body` if it
  has them, by atoms or by text. Gives the answer's `status`, `headers` and `body`, as it came, without following a
  redirect. An untrusted app reaches no website (`OneCore.Sandbox`).

      OneCore.Call.fetch(call, %{url: "https://api.example.com/notes", method: "POST", body: JSON.encode!(note)})
  """
  def fetch(_call, request) do
    request = Map.new(request, fn {key, value} -> {to_string(key), value} end)
    method = String.upcase(Map.get(request, "method", "GET"))

    if method not in ~w(GET HEAD POST PUT PATCH DELETE OPTIONS),
      do: raise(ArgumentError, "a request's method is GET, HEAD, POST, PUT, PATCH, DELETE or OPTIONS")

    response =
      Req.request!(
        method: method |> String.downcase() |> String.to_existing_atom(),
        url: request["url"],
        headers: for({name, value} <- Map.get(request, "headers", %{}), do: {to_string(name), to_string(value)}),
        body: request["body"],
        decode_body: false,
        redirect: false,
        retry: false,
        receive_timeout: 30_000
      )

    headers = Map.new(response.headers, fn {name, values} -> {name, Enum.join(values, ", ")} end)
    %{status: response.status, headers: headers, body: response.body}
  end

  @doc """
  A process that keeps running on this server between calls: the GenServer or `:gen_statem` `module`, which is one of
  this call's app's or of an app whose code runs inside it. There's one for each `key`, started when first asked
  for, with `argument` given to its `init/1`, and found again by every call after. It carries on through its app's
  updates as a service does, and one that stopped is started by the next call that asks for it.

      counter = OneCore.Call.process(call, Hello.Counter, call.subject, 0)
  """
  def process(%__MODULE__{loaded: loaded, app_artifact: app}, module, key, argument) do
    under = Version.under(module)
    app = Enum.find_value(loaded, app, fn {_name, {app, other}} -> Version.under(other) == under && app end)

    Apps.Process.of(app, Version.own_name(module), key, argument) ||
      raise "#{inspect(module)} isn't a module of #{app} as it runs now"
  end

  @doc """
  Has a server acting for `owner` run this call instead, since this one doesn't act for them. The call ends here
  with that server's answer, and raises if none of them is up, or none acts for whoever the app was installed for
  too.

  With servers' keys in `owner`'s place, it's one of those that runs it, for an app that knows which can: the
  ones that hold a key, say. `:no_response` then means none of them ran it.
  """
  def forward(%__MODULE__{forwarded: false} = call, servers) when is_list(servers),
    do: ask_each(call.request, {call.actor, call.subject}, servers -- [Server.key()], "to")

  def forward(%__MODULE__{}, servers) when is_list(servers), do: throw({__MODULE__, :forwarding_loop})
  def forward(%__MODULE__{} = call, owner), do: forward(call.request, {call.actor, call.subject}, call.forwarded, owner)

  @doc """
  Has one of `servers`, the ones that run this call (`OneCore.Apps.servers/3`), run it, since this one doesn't
  (`OneCore.Apps.runs?/3`). It ends here with that server's answer, or raises saying whose servers run it and that
  none is on.
  """
  def run_elsewhere(%__MODULE__{} = call, servers),
    do: run_elsewhere(call.request, {call.actor, call.subject}, call.forwarded, {call.installed, servers}, call.app)

  @doc """
  Has one of `servers`, the ones that run the apps installed for `owner`, run what was asked of this one, `what`, as
  `run_elsewhere/2` does.
  """
  def run_elsewhere(ask, principal, forwarded, {owner, servers}, what) do
    if forwarded, do: throw({__MODULE__, :forwarding_loop})
    others = servers -- [Server.key()]
    elsewhere(ask, principal, others)
    raise Servers.off(what, owner, others)
  end

  @doc """
  Has a server acting for `owner` run what was asked of this one instead, as `forward/2` does for a call: `ask` is
  its method and what it was asked with, for `asked`.
  """
  def forward(ask, principal, forwarded, owner) do
    if forwarded, do: throw({__MODULE__, :forwarding_loop})
    servers = OneCore.Apps.covering(owner) -- [Server.key()]
    elsewhere(ask, principal, servers)
    why = Enum.map_join(servers, "; ", &Server.error_message(:unreachable, &1))
    raise "No server acting for #{owner} answered" <> if(why == "", do: "", else: ": " <> why)
  end

  @doc """
  Has the server whose key is `server` run what was asked of this one, `ask`, for `principal`, as `forward/4` has
  one acting for an owner: a tool asked of a particular server. It ends here with that server's answer, and raises
  why where it couldn't be asked. `ready: :any` asks it while it isn't ready too, as long as it runs: one that's
  stuck is how it's updated.
  """
  def run_on(ask, principal, server, opts \\ []) do
    elsewhere(ask, principal, [server], opts)
    raise Server.error_message(:unreachable, server)
  end

  # Asks `servers` to run it, as `ask_each/5` does, and, if none did, the first that's back of those that were
  # starting or stopping.
  defp elsewhere(ask, principal, servers, opts \\ []) do
    ask_each(ask, principal, servers, "to", opts)
    if back = Servers.back(servers), do: asks(ask, principal, back, "to", :infinity, opts)
  end

  @doc """
  Asks another server to run this call instead, for when what it needs is there. If that server runs it, the call
  ends here with its answer, and nothing after `forward_to` runs.

  If it didn't run there, `:no_response` means that server couldn't be reached just now, or went away after it was
  asked something that's safe to ask again; and `:forwarding_loop` that it answered, saying it would pass it on in
  turn. A server that stops answering after it was asked anything else raises, since it may have run.

  With `within`, in milliseconds, a server that hasn't answered within that long what's safe to ask again is taken
  as gone too: one that's there but stuck keeps the call open however long, saying it's still running it. What
  isn't safe to ask again is waited for as long as it takes, since it may be doing it.

  With a list of servers in `server`'s place, one of them runs it, asked as `forward/2` asks them, and `:no_response`
  means none did.
  """
  def forward_to(call, server, within \\ :infinity)
  def forward_to(%__MODULE__{forwarded: true}, _server, _within), do: throw({__MODULE__, :forwarding_loop})

  def forward_to(%__MODULE__{request: ask, subject: subject, actor: actor}, servers, :infinity) when is_list(servers),
    do: ask_each(ask, {actor, subject}, servers, true)

  def forward_to(%__MODULE__{request: ask, subject: subject, actor: actor}, server, within),
    do: asks(ask, {actor, subject}, server, true, within)

  # How long what's safe to ask again waits for a server's answer before it's asked of the next as well: most are answered
  # within a second, and a server that's connected but stuck says it's still running it however long, as a busy one
  # does. The same as a device's client waits (`HEDGE`, `core/client/src/rpc.rs`).
  @hedge 3_000

  # Has one of `servers` run it, with `forwarded` saying how it reached them. What's safe to ask again (`again?/4`) is
  # hedged, as gRPC's hedging policy hedges what's safe to ask twice: asked of the first, and of the next too whenever
  # none has answered within `@hedge` or one couldn't run it, the first answer being its answer. Anything else is asked
  # of each in turn, each waited for as long as it takes, since it may be doing it. `:no_response` if none ran it.
  defp ask_each(ask, {_actor, subject} = principal, servers, forwarded, opts \\ []) do
    {method, params} = passing = passed(ask, principal, forwarded)

    if again?(method, params, subject, servers),
      do: hedged([], servers, passing, opts),
      else: Enum.each(servers, &answered(request(&1, method, params, opts), &1, false))

    given_up()
  end

  defp hedged([], [], _ask, _opts), do: :no_response
  defp hedged([], [server | rest], ask, opts), do: hedged([hedge(server, ask, opts)], rest, ask, opts)

  defp hedged(asking, servers, ask, opts) do
    by_ref = Map.new(asking, &{&1.ref, &1})

    receive do
      {ref, {server, response}} when is_map_key(by_ref, ref) ->
        Process.demonitor(ref, [:flush])
        others = List.delete(asking, by_ref[ref])

        if answer?(response) do
          Enum.each(others, &Task.shutdown(&1, :brutal_kill))
          answered(response, server, true)
        else
          hedged(others, servers, ask, opts)
        end

      {:DOWN, ref, :process, _pid, _reason} when is_map_key(by_ref, ref) ->
        hedged(List.delete(asking, by_ref[ref]), servers, ask, opts)
    after
      if(servers == [], do: :infinity, else: @hedge) ->
        [server | rest] = servers
        hedged([hedge(server, ask, opts) | asking], rest, ask, opts)
    end
  end

  # Asks `server` from a process of its own, in this call's trace, which gives what it answered.
  defp hedge(server, {method, params}, opts) do
    {trace, state} = {OpenTelemetry.Ctx.get_current(), State.id()}

    Task.Supervisor.async_nolink(OneCore.Asks, fn ->
      OpenTelemetry.Ctx.attach(trace)
      State.follow(state)

      try do
        {server, request(server, method, params, opts)}
      catch
        kind, reason -> {server, {:raised, kind, reason, __STACKTRACE__}}
      end
    end)
  end

  # Whether a server ran it, or refused it: anything but couldn't, would pass it on in turn, or is a write another
  # server runs (`OneCore.Writes`).
  defp answer?({:error, code, _why}), do: code not in [Server.forwarding_loop(), Writes.not_run()]
  defp answer?(response), do: match?({:ok, _}, response) or match?({:raised, _, _, _}, response)

  # `forwarded` is what the server asked is told of how the call reached it: `"to"` from one that can't run it, and
  # `true` from one that hands it to the server holding what it needs.
  defp asks(ask, {_actor, subject} = principal, server, forwarded, within \\ :infinity, opts \\ []) do
    {method, params} = passed(ask, principal, forwarded)
    again = again?(method, params, subject, [server])

    response =
      if within != :infinity and again,
        do: request_within(server, method, params, within),
        else: request(server, method, params, opts)

    with :no_response <- answered(response, server, again), do: given_up()
  end

  # Asks `server`, naming it first as the one a write this server holds is passed to, so it takes it over
  # (`OneCore.Writes`).
  defp request(server, method, params, opts \\ []) do
    if method == "tools/call", do: Writes.passing(server)
    Server.request(server, method, params, opts)
  end

  # Nothing that was asked ran it, so a write passed on comes back to this server, unless one took it over meanwhile,
  # which then ends the call.
  defp given_up do
    case Writes.kept() do
      :ok -> :no_response
      {:done, result} -> throw({__MODULE__, {:forwarded, result}})
      {:not_run, why} -> throw({__MODULE__, {:not_run, why}})
    end
  end

  defp passed({method, params}, {actor, subject}, forwarded) do
    meta = %{forwarded: forwarded, actor: actor, subject: subject}
    meta = if client = State.client(), do: Map.put(meta, :client, client), else: meta
    # A call whose caller asked for a task that isn't made yet is made one where it runs, which then answers it.
    meta = if method == "tools/call", do: Map.merge(meta, Tasks.asking()), else: meta
    # A write's idempotency key goes with it, so it's run once wherever it's asked (`OneCore.Writes`).
    write = with {_id, row} <- State.current(), do: row[:write]
    meta = if method == "tools/call" and write, do: Map.put(meta, :idempotencyKey, write.key), else: meta
    {method, Map.update(params, :_meta, meta, &Map.merge(&1, meta))}
  end

  # The call ends with what `server` answered, if it ran it; `again` is whether it was safe to ask again.
  defp answered(response, server, again) do
    forwarding_loop = Server.forwarding_loop()

    case response do
      {:ok, result} -> throw({__MODULE__, {:forwarded, result}})
      {:error, ^forwarding_loop, _why} -> :forwarding_loop
      {:error, _code, why} -> raise why
      {:raised, kind, reason, stacktrace} -> :erlang.raise(kind, reason, stacktrace)
      :unreachable -> :no_response
      :stopped -> if again, do: :no_response, else: raise(Server.error_message(:stopped, server))
    end
  end

  # Asks `server` from a process of its own, in this call's trace, and gives it `within` to answer: one that hasn't
  # is taken as gone after it was asked, and its answer, if it ever comes, goes to nobody.
  defp request_within(server, method, params, within) do
    trace = OpenTelemetry.Ctx.get_current()
    state = State.id()

    asking =
      Task.async(fn ->
        OpenTelemetry.Ctx.attach(trace)
        State.follow(state)

        try do
          {:returned, request(server, method, params)}
        catch
          kind, reason -> {kind, reason, __STACKTRACE__}
        end
      end)

    case Task.yield(asking, within) || Task.shutdown(asking, :brutal_kill) do
      {:ok, {:returned, response}} -> response
      {:ok, {kind, reason, stacktrace}} -> :erlang.raise(kind, reason, stacktrace)
      nil -> :stopped
    end
  end

  # What a server went away from after it was asked is asked of the next one only where asking twice does no harm:
  # a hook, which may be asked more than once for one moment, a tool that only reads, and a write by its idempotency
  # key, of servers that keep to it (`OneCore.Writes`).
  defp again?("hooks/call", _params, _who, _servers), do: true

  defp again?("tools/call", %{name: name} = params, who, servers),
    do: Calls.reads?(name, who) or (is_map_key(params[:_meta], :idempotencyKey) and Enum.all?(servers, &Writes.keeps?/1))

  defp again?(_method, _params, _who, _servers), do: false
end
