defmodule OneCore.Call do
  @moduledoc """
  What a tool is called with besides its inputs.

  - `own`: the plugin's own `OneCore.Artifact` for whoever it runs for. Only this plugin reaches it.
  - `folder`: a folder on the machine running this call that is the plugin's own for whoever it runs for, for what
    it keeps there. It may not be there yet, and another server has another.
  - `server`: the key of the server running this call.
  - `for`: who this call runs for, as relation tuples name them: the user who asked, or a sharing link's holder,
    or the group or the network they asked for. What it makes is theirs, and it reaches what they may. A
    scheduled job runs for whoever its plugin was added for.
  - `by`: who asked: a user, or a sharing link's holder. They're a member of whoever the call is for. A scheduled
    job is called by the clock, so by nobody: `nil`.

  The rest is One Core's: which plugin this is, what was asked of it, the plugins it uses, and whether another
  server passed it on.
  """
  alias OneCore.{Artifact, Hooks, Plugins, Server, User}
  alias OneCore.Plugins.Version

  @enforce_keys [:own, :folder, :server, :for, :by, :plugin, :ask, :uses, :passed_on]
  defstruct [:own, :folder, :server, :for, :by, :plugin, :ask, :uses, :passed_on]

  @type t :: %__MODULE__{own: OneCore.Artifact.t(), folder: String.t(), server: String.t(), for: String.t()}

  @doc """
  A plugin this call's plugin uses, or that one of those uses: its module, whose functions take this call and run
  as this call's plugin, for whoever it runs for.

      database = OneCore.Call.plugin(call, "database")
  """
  def plugin(%__MODULE__{uses: uses, plugin: plugin}, name) do
    case uses do
      %{^name => module} -> module
      _ -> raise "#{plugin} doesn't say it uses #{name}. Its uses/0 names the plugins it uses."
    end
  end

  @doc """
  An artifact this call's plugin makes for whoever it runs for, under an `id` the plugin chooses: letters, digits,
  `_` and `-`. It's theirs, not the plugin's, and the plugin reaches it for them without asking. The same `id`
  is the same artifact, so a call that starts again makes it once.

      note = OneCore.Call.make(call, "7")
      OneCore.Artifact.write(note, "text", "Buy milk")
      OneCore.Artifact.name(note)
      #=> "users/anyone/made/notes/7"
  """
  def make(%__MODULE__{for: who, by: by, plugin: plugin}, id) do
    if User.link?(who), do: raise("A sharing link's holder makes nothing: nothing in a network is theirs")
    if not (is_binary(id) and id =~ ~r/^[A-Za-z0-9_-]+$/), do: raise(ArgumentError, "an id is letters, digits, _ and -")
    %Artifact{prefix: "#{who}/made/#{plugin}/#{id}/", reached: {plugin, {by, who}, "owner"}}
  end

  @doc """
  The artifact named `name`, for a call that may reach it as `relation`: a `viewer` reads it, an `editor` changes
  it too, and an `owner` says who else may. Raises with the reason if it may not.

  A plugin reaches an artifact only if both it and whoever it runs for have that relation to it, which is for the
  plugins that decide `reach` to say: with none, everything in a network is shared. Nothing is asked about what
  the plugin made for whoever it runs for, and nothing about the plugin where it made the artifact for someone
  else, or the artifact is its own.

      note = OneCore.Call.reach(call, "users/alice/made/notes/7")
      {:ok, text, _tag} = OneCore.Artifact.read(note, "text")
  """
  def reach(%__MODULE__{for: who, by: by, plugin: plugin}, name, relation \\ "viewer") do
    case refused(plugin, {by, who}, name, relation) do
      nil -> %Artifact{prefix: name <> "/", reached: {plugin, {by, who}, relation}}
      why -> raise why
    end
  end

  @doc "Whether `reach/3` would give the artifact: whether this call may reach it as `relation`."
  def reaches?(%__MODULE__{for: who, by: by, plugin: plugin}, name, relation \\ "viewer"),
    do: refused(plugin, {by, who}, name, relation) == nil

  @doc """
  Why a plugin in a call asked by `by`, running for `who`, may not reach the artifact `name` as `relation`, or
  `nil` if it may.
  """
  def refused(plugin, {_by, who} = asked, name, relation) when relation in ["viewer", "editor", "owner"] do
    if not Enum.all?(String.split(name, "/"), &(&1 =~ ~r/^[^.]/)),
      do: raise(ArgumentError, "an artifact's name is parts joined by /, none empty or starting with a dot")

    made = match?([_, _, "made", ^plugin | _], String.split(name, "/"))

    about =
      cond do
        made and String.starts_with?(name, who <> "/") -> []
        made or name == Plugins.own(plugin) -> [who]
        true -> [who, Plugins.who(plugin)]
      end

    Enum.find_value(about, fn about ->
      case Hooks.decide("reach", %{"who" => about, "relation" => relation, "artifact" => name}, asked) do
        :ok -> nil
        {:refuse, why} -> why
      end
    end)
  end

  @doc """
  A process that keeps running on this server between calls: the GenServer `module`, which is one of this call's
  plugin's or of a plugin it uses. There's one for each `key`, started when first asked for, with `argument` given
  to its `init/1`, and found again by every call after. It carries on through its plugin's updates as a service
  does, and one that stopped is started by the next call that asks for it.

      counter = OneCore.Call.process(call, Hello.Counter, call.for, 0)
  """
  def process(%__MODULE__{uses: uses, plugin: plugin}, module, key, argument) do
    under = Version.under(module)
    plugin = Enum.find_value(uses, plugin, fn {name, used} -> Version.under(used) == under && name end)

    Plugins.Process.of(plugin, Version.own_name(module), key, argument) ||
      raise "#{inspect(module)} isn't a module of #{plugin} as it runs now"
  end

  @doc """
  Asks another server to run this call instead, for when what it needs is there: a database's lease, say. If that
  server runs it, the call ends here with its answer, and nothing after `pass_on` runs.

  `:no_answer` means it didn't run there: that server couldn't be reached, or would have passed it on in turn. A
  server that stops answering after it was asked raises, since it may have run.
  """
  def pass_on(%__MODULE__{passed_on: true}, _server), do: throw({__MODULE__, :passed_back})

  def pass_on(%__MODULE__{ask: {method, params}, for: who, by: by}, server) do
    passed_back = Server.passed_back()

    case Server.request(server, method, Map.put(params, :_meta, %{passed_on: true, who: by, for: who})) do
      {:ok, result} -> throw({__MODULE__, {:passed_on, result}})
      {:error, ^passed_back, _why} -> :no_answer
      {:error, _code, why} -> raise why
      :unreachable -> :no_answer
      :stopped -> raise Server.said(:stopped)
    end
  end
end
