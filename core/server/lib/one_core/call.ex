defmodule OneCore.Call do
  @moduledoc """
  What a tool is called with besides its inputs.

  - `own`: the plugin's own `OneCore.Artifact` for whoever it was installed for. Only this plugin reaches it.
  - `installed`: who the plugin was installed for, whose `own` is: a user, a group or the network. It's who the plugin
    runs for with nobody there, as a scheduled job and a service do.
  - `folder`: a folder on the machine running this call that is the plugin's own for whoever it was installed for, for what
    it keeps there. It may not be there yet, and another server has another.
  - `server`: the key of the server running this call.
  - `subject`: who this call runs for, as relation tuples name them: the user who asked, or a sharing link's holder,
    or the group or the network they asked for. What it makes is theirs, and it reaches what they may. A
    scheduled job runs for whoever its plugin was installed for.
  - `actor`: who asked: a user, or a sharing link's holder. They're a member of whoever the call is for. A scheduled
    job is called by the clock, so by nobody: `nil`.
  - `caller`: the plugin that called this tool (`tool/5`), as its artifact's name, or the plugin that one acts on
    behalf of. One Core sets it from whose call made the call, wherever it runs, never from what's asked. A tool
    called by a person, an app or an agent has none: `nil`.

  The rest is One Core's: the plugin's name where it was installed and the plugin as a who, what was asked of it, the
  plugins it uses, and whether another server passed it on.

  A call is passed between servers at most twice, so none goes round. A server that can't run it has one that can
  run it instead (`forward/2`): `forwarded` is then `:to`. That one may still hand it to the one server that holds
  what it needs (`forward_to/2`): `forwarded` is then `true`, and it goes no further.
  """
  alias OneCore.{Artifact, Hooks, Mcp, Plugins, Server, Servers, User}
  alias OneCore.Plugins.{Calls, Version}

  @enforce_keys [:own, :installed, :folder, :server, :subject, :actor, :plugin, :who, :request, :uses, :forwarded]
  defstruct [:own, :installed, :folder, :server, :subject, :actor, :caller, :plugin, :who, :request, :uses, :forwarded]

  @type t :: %__MODULE__{own: OneCore.Artifact.t(), folder: String.t(), server: String.t(), subject: String.t()}

  @doc """
  A plugin this call's plugin uses, or that one of those uses: its module, whose functions take this call and run
  as this call's plugin, for whoever it runs for.

      database = OneCore.Call.plugin(call, "database")
  """
  def plugin(%__MODULE__{uses: uses, plugin: plugin}, name) do
    case uses do
      %{^name => {_artifact, module}} -> module
      _ -> raise "#{plugin} doesn't say it uses #{name}. Its uses/0 names the plugins it uses."
    end
  end

  @doc """
  Calls a tool of another plugin, installed for whoever this call runs for or for the network, as they would call it
  themselves: the same inputs are checked, the same hooks asked, and it runs as that plugin, with its own artifact.
  Gives its answer as it would reach a client over JSON, a file it answers as the file (`{:blob, kind, data}`), or
  raises with what that plugin said.

  The tool is told this call's plugin called it, as its `caller`. A tool that another plugin called can call one in
  turn on behalf of that plugin, as OAuth's on-behalf-of flow does, with `on_behalf_of: call.caller`: then that
  one is its `caller`, so a plugin that does something for others, as `google-account` does, uses the Vault for
  whichever called it.

      OneCore.Call.tool(call, "notifications", "send", %{"title" => "Lunch is here"})
  """
  def tool(%__MODULE__{actor: by, subject: who} = call, plugin, tool, inputs, options \\ []) do
    caller =
      case Keyword.fetch(options, :on_behalf_of) do
        :error -> call.who
        {:ok, caller} when caller == call.caller and caller != nil -> caller
        {:ok, _} -> raise "#{call.plugin} acts on behalf only of the plugin that called it, and #{inspect(call.caller)} did"
      end

    case Calls.call(plugin, tool, inputs, {by, who}, false, caller) do
      {:ok, result} when is_binary(result) ->
        result

      {:ok, {:blob, _kind, _data} = file} ->
        file

      {:ok, result} ->
        JSON.decode!(JSON.encode!(result))

      {:forwarded, %{"isError" => true, "content" => [%{"text" => why} | _]}} ->
        raise why

      {:forwarded, %{"content" => [%{"type" => "image", "mimeType" => kind, "data" => data}]}} ->
        {:blob, kind, Base.decode64!(data)}

      {:forwarded, %{"content" => [%{"resource" => %{"mimeType" => kind, "blob" => data}}]}} ->
        {:blob, kind, Base.decode64!(data)}

      {:forwarded, %{"content" => [%{"text" => text} | _]}} ->
        decoded(text)

      {:error, why} ->
        raise why
    end
  end

  @doc """
  Runs `hook`, one this call's plugin offers (`offers/0`), for whoever this call runs for, as One Core runs its own:
  the plugins installed for them and for the network that hook into `<plugin>.<hook>` are asked. What it gives is
  the kind's: an observe hook gives `:ok` at once, a mutate hook `{:ok, changes, by}` or `{:refuse, why}`, a
  validate hook `:ok` or `{:refuse, why}`, and a collect hook `{:ok, parts, failed}`: each plugin's part by its name,
  and why each that couldn't answer couldn't. A plugin runs only the hooks it offers.

      OneCore.Call.run_hook(call, "removed", %{"artifact" => name, "what" => "restored"})
  """
  def run_hook(%__MODULE__{installed: at, plugin: plugin, actor: by, subject: who}, hook, about) do
    name = "#{plugin}.#{hook}"

    case Hooks.offered(name, at) do
      :observe -> Hooks.observe(name, about, {by, who})
      :mutate -> Hooks.mutate(name, about, {by, who})
      :validate -> Hooks.validate(name, about, {by, who})
      :collect -> Hooks.collect(name, about, {by, who})
      nil -> raise "#{plugin} doesn't offer #{hook}. Its offers/0 names the hooks it offers."
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
  An artifact this call's plugin makes for whoever it runs for, under an `id` the plugin chooses: letters, digits,
  `_` and `-`. It's theirs, not the plugin's, and the plugin reaches it for them without asking. The same `id`
  is the same artifact, so a call that starts again makes it once.

      note = OneCore.Call.make(call, "7")
      OneCore.Artifact.write(note, "text", "Buy milk")
      OneCore.Artifact.name(note)
      #=> "users/alice/made/notes/7"
  """
  def make(%__MODULE__{subject: who, actor: by, plugin: plugin} = call, id) do
    if User.link?(who), do: raise("A sharing link's holder makes nothing: nothing in a network is theirs")
    if not (is_binary(id) and id =~ ~r/^[A-Za-z0-9_-]+$/), do: raise(ArgumentError, "an id is letters, digits, _ and -")
    %Artifact{prefix: "#{who}/made/#{plugin}/#{id}/", reached: {call.who, {by, who}, "owner"}}
  end

  @doc """
  The artifact named `name`, for a call that may reach it as `relation`: a `viewer` reads it, an `editor` changes
  it too, and an `owner` says who else may. Raises with the reason if it may not.

  A plugin reaches an artifact only if both it and whoever it runs for have that relation to it, which is for the
  plugins that validate `reach` to say: with none, only the network's first owner reaches it. Nothing is asked about what
  the plugin made for whoever it runs for, and nothing about the plugin where it made the artifact for someone
  else, or the artifact is its own. Another plugin's own artifact is never reached: it's always that one plugin's.

      note = OneCore.Call.reach(call, "users/alice/made/notes/7")
      {:ok, text, _tag} = OneCore.Artifact.read(note, "text")
  """
  def reach(%__MODULE__{subject: who, actor: by} = call, name, relation \\ "viewer") do
    if why = refused(call, name, relation), do: raise(why)
    # Its blobs are whoever's it is, whom this server may not act for. One that does runs the call.
    owner = User.owner(name) || "network"
    if not Servers.here?(owner), do: forward(call, owner)
    %Artifact{prefix: name <> "/", reached: {call.who, {by, who}, relation}}
  end

  @doc "Whether `reach/3` would give the artifact: whether this call may reach it as `relation`."
  def reaches?(%__MODULE__{} = call, name, relation \\ "viewer"), do: refused(call, name, relation) == nil

  @doc "Why this call may not reach the artifact `name` as `relation`, or `nil` if it may."
  def refused(%__MODULE__{subject: who, actor: by, plugin: plugin} = call, name, relation)
      when relation in ["viewer", "editor", "owner"] do
    parts = String.split(name, "/")

    if not Enum.all?(parts, &(&1 =~ ~r/^[^.]/)),
      do: raise(ArgumentError, "an artifact's name is parts joined by /, none empty or starting with a dot")

    # What's someone's sits under their prefix by what it is: what a plugin made for them, or a plugin's own.
    kind = if hd(parts) == "network", do: Enum.slice(parts, 1, 2), else: Enum.slice(parts, 2, 2)

    about =
      cond do
        name == Artifact.name(call.own) -> [who]
        match?(["own" | _], kind) -> nil
        kind == ["made", plugin] and String.starts_with?(name, who <> "/") -> []
        kind == ["made", plugin] -> [who]
        true -> [who, call.who]
      end

    if about == nil do
      "#{name} is a plugin's own artifact, which only that plugin reaches"
    else
      Enum.find_value(about, fn about ->
        case Hooks.validate("reach", %{"who" => about, "relation" => relation, "artifact" => name}, {by, who}) do
          :ok -> nil
          {:refuse, why} -> why
        end
      end)
    end
  end

  @doc """
  Whether whoever asked is a member of `of`, a group or the network, as the plugins that validate `reach` say. With
  nobody asking, as for a scheduled job, it's whoever the call runs for. It's what a plugin asks before telling
  someone what only a group's members, or the network's, may know.
  """
  def member?(%__MODULE__{subject: who, actor: by}, of) do
    asking = by || who

    asking == of or
      Hooks.validate("reach", %{"who" => asking, "relation" => "member", "artifact" => of}, {by, who}) == :ok
  end

  @doc """
  Whether whoever asked owns whoever this call runs for: themselves, or the group or the network they asked for,
  as its `owner`. It's who changes what's installed for them, so a plugin asks it before changing what's a group's
  or the network's as a whole, where being a member isn't enough.
  """
  def owns?(%__MODULE__{subject: who, actor: who}), do: true
  def owns?(%__MODULE__{actor: nil}), do: false

  def owns?(%__MODULE__{subject: who, actor: by}),
    do: Hooks.validate("reach", %{"who" => by, "relation" => "owner", "artifact" => who}, {by, who}) == :ok

  @doc """
  The plugins whoever this call runs for finds, theirs and then the network's: each one's `name` there, the
  `plugin` it is, as its artifact's name, who it was `installed` for, its `title`, `icon`, `subtitle` and `about`, its
  `apps`, its `widgets` and its `tools`' names and pages (`ui`). A Home Screen page is drawn from these.
  """
  def plugins(%__MODULE__{subject: who}) do
    for at <- Enum.reverse(Plugins.chain(who)), {name, version} <- Plugins.versions([at]), into: %{} do
      tools = for tool <- version.tools, do: Map.take(tool, [:name, :ui])

      {name,
       Map.merge(
         %{
           name: name,
           plugin: version.artifact,
           installed: at,
           title: version.title,
           icon: version.icon,
           apps: version.apps,
           widgets: version.widgets,
           tools: tools
         },
         OneCore.Plugins.Version.listing(version)
       )}
    end
    |> Map.values()
  end

  @doc """
  Every plugin anyone in the network has deployed, whether or not it's installed for anyone now: each one's `plugin`,
  as its artifact's name, its `title`, `icon`, `subtitle` and `about`, and its `apps` as its latest version has
  them. An app store is drawn from it. Whoever may see one reads and installs it, and so does everyone in a group or
  the network it's in use for (`OneCore.Tools`).
  """
  def deployed(%__MODULE__{}), do: OneCore.Plugins.Deploy.deployed()

  @doc """
  Makes a web request from the server running this call: its `url`, and its `method`, `headers` and `body` if it
  has them, by atoms or by text. Gives the answer's `status`, `headers` and `body`, as it came, without following a
  redirect. In sandbox mode, only to a host the plugin's `permissions/0` names (`OneCore.Sandbox`).

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
  A process that keeps running on this server between calls: the GenServer `module`, which is one of this call's
  plugin's or of a plugin it uses. There's one for each `key`, started when first asked for, with `argument` given
  to its `init/1`, and found again by every call after. It carries on through its plugin's updates as a service
  does, and one that stopped is started by the next call that asks for it.

      counter = OneCore.Call.process(call, Hello.Counter, call.subject, 0)
  """
  def process(%__MODULE__{uses: uses, who: plugin}, module, key, argument) do
    under = Version.under(module)
    plugin = Enum.find_value(uses, plugin, fn {_name, {plugin, used}} -> Version.under(used) == under && plugin end)

    Plugins.Process.of(plugin, Version.own_name(module), key, argument) ||
      raise "#{inspect(module)} isn't a module of #{plugin} as it runs now"
  end

  @doc """
  Has a server acting for `owner` run this call instead, since this one doesn't act for them. The call ends here
  with that server's answer, and raises if none of them is up, or none acts for whoever the plugin was installed for
  too.

  With servers' keys in `owner`'s place, it's one of those that runs it, for a plugin that knows which can: the
  ones that hold a key, say. `:no_answer` then means none of them ran it.
  """
  def forward(%__MODULE__{forwarded: false} = call, servers) when is_list(servers) do
    for server <- servers -- [Server.key()], do: asks(call.request, {call.actor, call.subject}, server, "to")
    :no_response
  end

  def forward(%__MODULE__{}, servers) when is_list(servers), do: throw({__MODULE__, :forwarding_loop})
  def forward(%__MODULE__{} = call, owner), do: forward(call.request, {call.actor, call.subject}, call.forwarded, owner)

  @doc """
  Has a server that runs the plugins installed for `owner` run this call, since this one doesn't
  (`OneCore.Servers.runs?/1`). It ends here with that server's answer, or raises saying whose servers run it and
  that none is on.
  """
  def run_elsewhere(%__MODULE__{} = call, owner),
    do: run_elsewhere(call.request, {call.actor, call.subject}, call.forwarded, owner, call.plugin)

  @doc "Has a server that runs the plugins installed for `owner` run what was asked of this one, `what`, as `run_elsewhere/2` does."
  def run_elsewhere(ask, principal, forwarded, owner, what) do
    if forwarded, do: throw({__MODULE__, :forwarding_loop})
    servers = Servers.running(owner)
    for server <- servers -- [Server.key()], do: asks(ask, principal, server, "to")
    raise Servers.off(what, owner, servers)
  end

  @doc """
  Has a server acting for `owner` run what was asked of this one instead, as `forward/2` does for a call: `ask` is
  its method and what it was asked with, for `asked`.
  """
  def forward(ask, principal, forwarded, owner) do
    if forwarded, do: throw({__MODULE__, :forwarding_loop})
    for server <- Servers.acting(owner) -- [Server.key()], do: asks(ask, principal, server, "to")
    raise "No server acting for #{owner} answered"
  end

  @doc """
  Asks another server to run this call instead, for when what it needs is there. If that server runs it, the call
  ends here with its answer, and nothing after `forward_to` runs.

  If it didn't run there, `:no_answer` means that server couldn't be reached just now, or went away after it was
  asked something that's safe to ask again; and `:forwarding_loop` that it answered, saying it would pass it on in
  turn. A server that stops answering after it was asked anything else raises, since it may have run.
  """
  def forward_to(%__MODULE__{forwarded: true}, _server), do: throw({__MODULE__, :forwarding_loop})

  def forward_to(%__MODULE__{request: ask, subject: who, actor: by}, server), do: asks(ask, {by, who}, server, true)

  # `forwarded` is what the server asked is told of how the call reached it: `"to"` from one that can't run it, and
  # `true` from one that hands it to the server holding what it needs.
  defp asks({method, params}, {by, who}, server, forwarded) do
    forwarding_loop = Server.forwarding_loop()

    meta = %{forwarded: forwarded, who: by, for: who}

    case Server.request(server, method, Map.update(params, :_meta, meta, &Map.merge(&1, meta))) do
      {:ok, result} -> throw({__MODULE__, {:forwarded, result}})
      {:error, ^forwarding_loop, _why} -> :forwarding_loop
      {:error, _code, why} -> raise why
      :unreachable -> :no_response
      :stopped -> if again?(method, params, who), do: :no_response, else: raise(Server.error_message(:stopped, server))
    end
  end

  # What a server went away from after it was asked is asked of the next one only where asking twice does no harm:
  # a hook, which may be asked more than once for one moment, and a tool that only reads.
  defp again?("hooks/call", _params, _who), do: true
  defp again?("tools/call", %{name: name}, who), do: Mcp.reads?(name, who)
  defp again?(_method, _params, _who), do: false
end
