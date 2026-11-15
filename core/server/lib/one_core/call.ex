defmodule OneCore.Call do
  @moduledoc """
  What a tool is called with besides its inputs.

  - `own`: the plugin's own `OneCore.Artifact` for whoever it was installed for. Only this plugin reaches it.
  - `installed`: who the plugin was installed for, whose `own` is: a user, a group or the network. It's who the plugin
    runs for with nobody there, as a scheduled job and a service do.
  - `folder`: a folder on the machine running this call that is the plugin's own for whoever it was installed for, for what
    it keeps there. It may not be there yet, and another server has another.
  - `server`: the key of the server running this call.
  - `for`: who this call runs for, as relation tuples name them: the user who asked, or a sharing link's holder,
    or the group or the network they asked for. What it makes is theirs, and it reaches what they may. A
    scheduled job runs for whoever its plugin was installed for.
  - `by`: who asked: a user, or a sharing link's holder. They're a member of whoever the call is for. A scheduled
    job is called by the clock, so by nobody: `nil`.

  The rest is One Core's: the plugin's name where it was installed and the plugin as a who, what was asked of it, the
  plugins it uses, and whether another server passed it on.

  A call is passed between servers at most twice, so none goes round. A server that can't run it has one that can
  run it instead (`forward/2`): `forwarded` is then `:to`. That one may still hand it to the one server that holds
  what it needs (`forward_to/2`): `forwarded` is then `true`, and it goes no further.
  """
  alias OneCore.{Artifact, Hooks, Plugins, Server, Servers, User}
  alias OneCore.Plugins.{Calls, Version}

  @enforce_keys [:own, :installed, :folder, :server, :for, :by, :plugin, :who, :ask, :uses, :forwarded]
  defstruct [:own, :installed, :folder, :server, :for, :by, :plugin, :who, :ask, :uses, :forwarded]

  @type t :: %__MODULE__{own: OneCore.Artifact.t(), folder: String.t(), server: String.t(), for: String.t()}

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
  Gives its answer as it would reach a client over JSON, or raises with what that plugin said.

      OneCore.Call.tool(call, "notifications", "send", %{"title" => "Lunch is here"})
  """
  def tool(%__MODULE__{by: by, for: who}, plugin, tool, inputs) do
    case Calls.call(plugin, tool, inputs, {by, who}) do
      {:ok, answer} when is_binary(answer) -> answer
      {:ok, answer} -> JSON.decode!(JSON.encode!(answer))
      {:forwarded, %{"isError" => true, "content" => [%{"text" => why} | _]}} -> raise why
      {:forwarded, %{"content" => [%{"text" => text} | _]}} -> decoded(text)
      {:error, why} -> raise why
    end
  end

  # What another server answered is text: JSON where the tool answered more than text.
  defp decoded(text) do
    case JSON.decode(text) do
      {:ok, answer} -> answer
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
  def make(%__MODULE__{for: who, by: by, plugin: plugin} = call, id) do
    if User.link?(who), do: raise("A sharing link's holder makes nothing: nothing in a network is theirs")
    if not (is_binary(id) and id =~ ~r/^[A-Za-z0-9_-]+$/), do: raise(ArgumentError, "an id is letters, digits, _ and -")
    %Artifact{prefix: "#{who}/made/#{plugin}/#{id}/", reached: {call.who, {by, who}, "owner"}}
  end

  @doc """
  The artifact named `name`, for a call that may reach it as `relation`: a `viewer` reads it, an `editor` changes
  it too, and an `owner` says who else may. Raises with the reason if it may not.

  A plugin reaches an artifact only if both it and whoever it runs for have that relation to it, which is for the
  plugins that decide `reach` to say: with none, only the network's first owner reaches it. Nothing is asked about what
  the plugin made for whoever it runs for, and nothing about the plugin where it made the artifact for someone
  else, or the artifact is its own. Another plugin's own artifact is never reached: it's always that one plugin's.

      note = OneCore.Call.reach(call, "users/alice/made/notes/7")
      {:ok, text, _tag} = OneCore.Artifact.read(note, "text")
  """
  def reach(%__MODULE__{for: who, by: by} = call, name, relation \\ "viewer") do
    if why = refused(call, name, relation), do: raise(why)
    # Its blobs are whoever's it is, whom this server may not act for. One that does runs the call.
    owner = User.owner(name) || "network"
    if not Servers.here?(owner), do: forward(call, owner)
    %Artifact{prefix: name <> "/", reached: {call.who, {by, who}, relation}}
  end

  @doc "Whether `reach/3` would give the artifact: whether this call may reach it as `relation`."
  def reaches?(%__MODULE__{} = call, name, relation \\ "viewer"), do: refused(call, name, relation) == nil

  @doc "Why this call may not reach the artifact `name` as `relation`, or `nil` if it may."
  def refused(%__MODULE__{for: who, by: by, plugin: plugin} = call, name, relation)
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
        case Hooks.decide("reach", %{"who" => about, "relation" => relation, "artifact" => name}, {by, who}) do
          :ok -> nil
          {:refuse, why} -> why
        end
      end)
    end
  end

  @doc """
  Whether whoever asked is a member of `of`, a group or the network, as the plugins that decide `reach` say. With
  nobody asking, as for a scheduled job, it's whoever the call runs for. It's what a plugin asks before telling
  someone what only a group's members, or the network's, may know.
  """
  def member?(%__MODULE__{for: who, by: by}, of) do
    asking = by || who

    asking == of or
      Hooks.decide("reach", %{"who" => asking, "relation" => "member", "artifact" => of}, {by, who}) == :ok
  end

  @doc """
  Whether whoever asked owns whoever this call runs for: themselves, or the group or the network they asked for,
  as its `owner`. It's who changes what's installed for them, so a plugin asks it before changing what's a group's
  or the network's as a whole, where being a member isn't enough.
  """
  def owns?(%__MODULE__{for: who, by: who}), do: true
  def owns?(%__MODULE__{by: nil}), do: false

  def owns?(%__MODULE__{for: who, by: by}),
    do: Hooks.decide("reach", %{"who" => by, "relation" => "owner", "artifact" => who}, {by, who}) == :ok

  @doc """
  The plugins whoever this call runs for finds, theirs and then the network's: each one's `name` there, the
  `plugin` it is, as its artifact's name, who it was `installed` for, its `apps` and its `widgets`. A home page is
  drawn from these.
  """
  def plugins(%__MODULE__{for: who}) do
    for at <- Enum.reverse(Plugins.chain(who)), {name, version} <- Plugins.versions([at]), into: %{} do
      {name, %{name: name, plugin: version.artifact, installed: at, apps: version.apps, widgets: version.widgets}}
    end
    |> Map.values()
  end

  @doc """
  Every plugin anyone in the network has deployed, whether or not it's installed for anyone now: each one's `plugin`,
  as its artifact's name, and its `apps` as its newest version has them. An app store is drawn from it. Whoever
  may see one reads and installs it, and so does everyone in a group or the network it's in use for (`OneCore.Tools`).
  """
  def deployed(%__MODULE__{}), do: OneCore.Plugins.Deploy.deployed()

  @doc """
  A process that keeps running on this server between calls: the GenServer `module`, which is one of this call's
  plugin's or of a plugin it uses. There's one for each `key`, started when first asked for, with `argument` given
  to its `init/1`, and found again by every call after. It carries on through its plugin's updates as a service
  does, and one that stopped is started by the next call that asks for it.

      counter = OneCore.Call.process(call, Hello.Counter, call.for, 0)
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
    for server <- servers -- [Server.key()], do: :no_answer = asks(call.ask, {call.by, call.for}, server, "to")
    :no_answer
  end

  def forward(%__MODULE__{}, servers) when is_list(servers), do: throw({__MODULE__, :forwarding_loop})
  def forward(%__MODULE__{} = call, owner), do: forward(call.ask, {call.by, call.for}, call.forwarded, owner)

  @doc """
  Has a server acting for `owner` run what was asked of this one instead, as `forward/2` does for a call: `ask` is
  its method and what it was asked with, for `asked`.
  """
  def forward(ask, asked, forwarded, owner) do
    if forwarded, do: throw({__MODULE__, :forwarding_loop})
    for server <- Servers.acting(owner) -- [Server.key()], do: :no_answer = asks(ask, asked, server, "to")
    raise "No server acting for #{owner} answered"
  end

  @doc """
  Asks another server to run this call instead, for when what it needs is there. If that server runs it, the call
  ends here with its answer, and nothing after `forward_to` runs.

  `:no_answer` means it didn't run there: that server couldn't be reached, or would have passed it on in turn. A
  server that stops answering after it was asked raises, since it may have run.
  """
  def forward_to(%__MODULE__{forwarded: true}, _server), do: throw({__MODULE__, :forwarding_loop})

  def forward_to(%__MODULE__{ask: ask, for: who, by: by}, server), do: asks(ask, {by, who}, server, true)

  # `forwarded` is what the server asked is told of how the call reached it: `"to"` from one that can't run it, and
  # `true` from one that hands it to the server holding what it needs.
  defp asks({method, params}, {by, who}, server, forwarded) do
    forwarding_loop = Server.forwarding_loop()

    case Server.request(server, method, Map.put(params, :_meta, %{forwarded: forwarded, who: by, for: who})) do
      {:ok, result} -> throw({__MODULE__, {:forwarded, result}})
      {:error, ^forwarding_loop, _why} -> :no_answer
      {:error, _code, why} -> raise why
      :unreachable -> :no_answer
      # A hook may be asked more than once for one moment, so one a server stopped answering is asked again.
      :stopped when method == "hooks/call" -> :no_answer
      :stopped -> raise Server.said(:stopped, server)
    end
  end
end
