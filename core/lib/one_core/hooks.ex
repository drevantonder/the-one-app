defmodule OneCore.Hooks do
  @moduledoc """
  The moments a plugin can hook into, and the one way each is run. A plugin names the hooks it hooks into
  (`hooks/0`), and its `hook/3` is called at each, as that plugin, for whoever the moment is about. That's
  `asked` below: who asked for what brought the moment on, and who it runs for.

  A hook is one of three kinds, and how a kind is run is what keeps plugins from getting in each other's way:

  - No hook is handed what happens next to run or to leave out. It answers, and One Core carries on: nothing is
    blocked by a plugin that forgot to pass it on.
  - Nothing depends on the order plugins are asked in. Plugins that hear are each told apart from the others,
    every plugin that changes is handed the same thing, and every plugin that decides has to agree.
  - What a plugin changed is said, with its name, to the plugins that decide and hear the same moment.

  A plugin hooks into the moments of whoever it was added for. So a moment that runs for someone is hooked into
  by the plugins added for them and by the ones added for the network, and by nobody else's: what one user adds
  decides nothing for another.

  A plugin that can't answer, because it raised, its server couldn't be reached or it took longer than ten seconds
  (`within/3`), has refused: nothing is let through for want of an answer. So a plugin that breaks blocks what it changes or decides, for whoever it was
  added for and nobody else, until it's removed or deployed again, and it has no say in either: a call that
  removes a plugin or deploys over it isn't that plugin's to change or decide.

  A hook may be asked more than once for one moment, as a tool may run more than once: a call passed on to
  another server is decided on both.

  That something may be reached is kept by the server that asked, so that it asks once: until a tool that doesn't
  only read is called of a plugin that decides it, which that server and the ones it tells forget at once, and
  for a few seconds anywhere else. Whose a device is is kept the same way, until a server is told to forget it:
  by a tool of a plugin that says whose, or by that plugin's edge part. A refusal is never
  kept, nor that a device is nobody's. So taking access away reaches every server, and giving it is seen at once.

  Who may do what is a plugin's to decide. Where no plugin decides `reach` for a moment, nothing is reached and
  no tool is called but by the network's first owner (`OneCore.User.first_owner/0`), who is how a network whose
  deciding plugin was removed is put right. A network being built, the server's `:development` setting, lets
  everything through instead.
  """
  require Logger
  alias OneCore.Plugins
  alias OneCore.Plugins.Calls

  @kinds [
    hears: """
    is told after it happened, apart from what happened and from every other plugin that hears it. What it answers
    is ignored, and so is its raising.
    """,
    changes: """
    is handed it as it was asked, as every other plugin that changes it is, and answers only what it changes, by
    name: `%{}` changes nothing. Two plugins that change the same thing to something different refuse it, and say
    so. Which plugin changed what is said to the plugins that decide and hear it.
    """,
    decides: """
    answers `:ok` or `{:refuse, why}`. Every plugin that decides it has to answer `:ok`: one refusal refuses it,
    and whoever asked is told why. A plugin that raises, or answers anything else, refuses it too.
    """
  ]

  @hooks [
    {"inputs", :changes,
     """
     A tool is about to be called. It's about the tool's `plugin`, which is `nil` for one of One Core's own tools, its
     `tool` name and the `inputs` it was asked with. The answer is the inputs to set, by name. The tool is called only
     if they are still what it takes.
     """},
    {"call", :decides,
     """
     A tool is about to be called. It's about the tool's `plugin`, which is `nil` for one of One Core's own tools, its
     `tool` name, the `inputs` it will be called with, and `changed`: the plugin that set each input that was changed,
     by the input's name.
     """},
    {"reach", :decides,
     """
     A plugin is about to reach an artifact that isn't its own. It's about `who` would reach it, the `relation`
     they need to it (`viewer`, `editor` or `owner`) and the `artifact`'s name. It's asked about whoever the
     plugin runs for, and about the plugin, by its artifact's name, where the plugin didn't make the artifact.
     An `:ok` is kept for a few seconds, and until a tool of the plugin is called that doesn't only read: so
     what it decides from is what its own tools change.
     It's also asked whether whoever asks is a `member` of the group or the network they ask for: then `who` is
     them, the `relation` is `member` and the `artifact` is the group, or `network`.
     """},
    {"device", :changes,
     """
     A device asks, and One Core is about to say whose it is. It's about the `device`, by its key, and `who`, which
     is nobody: `nil`. The answer is `who`: the user whose device it is, as `users/<name>`. With no plugin
     answering, the device is nobody's, and nobody is refused everything. It's asked of the plugins added for the
     network, since nobody is asking yet, and what's answered is kept until a tool of the plugin is called that
     doesn't only read.
     """},
    {"owner", :decides,
     """
     The network is about to have its first owner. It's about `who` they are. A plugin that keeps who may do what
     writes that down before it answers, and refuses if it can't: nothing else tells it.
     """},
    {"called", :hears,
     """
     A tool was called. It's about what `call` is about, how long the tool `took` in microseconds, and the `error`
     it raised, which is `nil` if it answered.
     """},
    {"changed", :hears,
     """
     A blob of an artifact that a plugin made or reached was written or removed. It's about the `artifact`'s name, the
     `blob`'s name in it, and `by`, the plugin that did it, as a who. A plugin is told only if it could already reach
     the artifact for whoever changed it, and isn't told what it changed itself.
     """},
    {"recorded", :hears,
     """
     A server recorded what it and its clients did, as `OneCore.Telemetry` says, and hands it on about once a
     minute. It's about the `server`'s key, its `machine`'s name, its `build`, the time it's `from` and `to` in
     nanoseconds since 1970, and what it recorded in OpenTelemetry's shapes: `spans`, `logs` and `metrics`. It's
     heard by the plugins added for the network, and nothing is recorded while none hears it.
     """},
    {"joined", :hears,
     """
     A server was let act for someone. It's about the `server`'s key, who it's `for`, and `by`, the key of the
     server that added it.
     """}
  ]

  @doc "Every hook's name, with its kind: `:hears`, `:changes` or `:decides`."
  def kinds, do: Map.new(@hooks, fn {hook, kind, _about} -> {hook, kind} end)

  @doc "What a plugin's author reads about hooks: each kind, and each hook with its kind and what it's about."
  def listed do
    kinds = for {kind, how} <- @kinds, do: "- A hook that #{kind} #{flat(how)}"
    hooks = for {hook, kind, about} <- @hooks, do: "- `#{hook}` #{kind}. #{flat(about)}"
    Enum.join(kinds ++ ["", "The hooks:", ""] ++ hooks, "\n")
  end

  defp flat(text), do: text |> String.split() |> Enum.join(" ")

  # How long a plugin has to answer a hook, in milliseconds. A check is answered in well under a millisecond, and
  # the slowest answer is one whose plugin's server has just gone: finding that out and taking over from it
  # takes about three seconds (`core/test/takeover.sh`), and more where the bucket is far away. This is that
  # with room to spare, and still short of a person giving up on what they asked.
  @answers_within 10_000

  @doc """
  Runs `fun`, a plugin's `hook/3` at `hook`, and gives what it answered, or raises what it raised. One that hasn't
  answered within ten seconds is stopped and has refused: it raises, saying so.
  """
  def within(plugin, hook, fun) do
    task =
      Task.async(fn ->
        try do
          {:answered, fun.()}
        catch
          kind, reason -> {kind, reason, __STACKTRACE__}
        end
      end)

    case Task.yield(task, @answers_within) || Task.shutdown(task, :brutal_kill) do
      {:ok, {:answered, answer}} -> answer
      {:ok, {kind, reason, stacktrace}} -> :erlang.raise(kind, reason, stacktrace)
      _ -> raise "#{plugin} didn't answer #{hook} within #{div(@answers_within, 1000)} seconds"
    end
  end

  # How long a server keeps that something may be reached, in milliseconds, where nothing told it to forget.
  @kept_for 10_000
  @allowed __MODULE__.Allowed

  @doc "Makes where which plugins hook into what is kept, and what was decided. Whatever calls this owns both."
  def keep do
    :ets.new(__MODULE__, [:named_table, read_concurrency: true])
    :ets.new(@allowed, [:named_table, :public, read_concurrency: true])
  end

  @doc "The hooks whose answers a server keeps."
  def kept, do: ["reach", "device"]

  @doc """
  Forgets what `hooks` were answered, so that they're asked again: who may reach what may have changed, or whose
  a device is.
  """
  def forget(hooks \\ kept()) do
    for hook <- hooks, hook in kept() do
      :ets.update_counter(@allowed, {:forgotten, hook}, 1, {{:forgotten, hook}, 0})
      :ets.match_delete(@allowed, {{hook, :_}, :_})
    end

    :ok
  end

  defp forgotten(hook), do: :ets.lookup_element(@allowed, {:forgotten, hook}, 2, 0)

  @doc """
  The plugins that hook into `hook` for a moment that runs for `who`: each as who it was added for, and its name
  there. Which hook into what is kept by `OneCore.Plugins`, by who they were added for, as it looks at them.
  """
  def hooking(hook, who) do
    for at <- Plugins.chain(who), name <- Map.get(added(at), hook, []), do: {at, name}
  end

  # The hooks that the plugins added for an owner hook into, each with those plugins' names. An owner this server
  # hasn't looked at is looked at first.
  defp added(owner) do
    with [] <- :ets.lookup(__MODULE__, owner),
         _ = Plugins.look(owner),
         [] <- :ets.lookup(__MODULE__, owner) do
      %{}
    else
      [{^owner, hooking}] -> hooking
    end
  end

  @doc """
  Whether a plugin added for `owner` hooks into `hook`, as this server last looked at them: asking costs a lookup,
  and never looks.
  """
  def hooked?(hook, owner) do
    case :ets.lookup(__MODULE__, owner) do
      [{^owner, hooking}] -> Map.has_key?(hooking, hook)
      [] -> false
    end
  end

  @doc """
  What a plugin's `hook/3` answered, as it's passed between servers: what JSON holds. Raises if it isn't an answer
  of the hook's kind.
  """
  def answer(hook, answer) do
    case {Map.fetch!(kinds(), hook), answer} do
      {:hears, _} ->
        nil

      {:changes, %{} = changes} ->
        JSON.decode!(JSON.encode!(changes))

      {:decides, :ok} ->
        "ok"

      {:decides, {:refuse, why}} when is_binary(why) ->
        %{"refuse" => why}

      {:changes, _} ->
        raise "It answered #{inspect(answer)} to #{hook}, which is answered with what it changes, by name"

      {:decides, _} ->
        raise "It answered #{inspect(answer)} to #{hook}, which is answered :ok or {:refuse, why}"
    end
  end

  @doc "What a plugin that has nothing to say answers `hook`."
  def nothing(hook) do
    case Map.fetch!(kinds(), hook) do
      :hears -> nil
      :changes -> %{}
      :decides -> :ok
    end
  end

  @doc "Tells each plugin that hears `hook` what it's about, for `asked`."
  def heard(hook, about, {_by, who} = asked) do
    for {at, name} <- hooking(hook, who),
        do: Task.Supervisor.start_child(__MODULE__, fn -> Calls.hook(at, name, hook, about, asked) end)

    :ok
  end

  @doc """
  Asks each plugin that changes `hook` what it changes of what it's about, for `asked`. Gives `{:ok, changes, by}`:
  what's changed by name, and the plugin that changed each. `{:refuse, why}` if two don't agree, or one couldn't
  say. `apart` is a plugin that isn't asked, as who it was added for and its name there.
  """
  def change(hook, about, {_by, who} = asked, apart \\ nil) do
    Enum.reduce_while(hooking(hook, who) -- [apart], {:ok, %{}, %{}}, fn {_at, plugin} = added, {:ok, changes, by} ->
      with {:ok, changed} <- answered(added, hook, about, asked),
           nil <- Enum.find(changed, fn {name, to} -> is_map_key(changes, name) and changes[name] != to end) do
        by = Map.merge(Map.new(changed, fn {name, _to} -> {name, plugin} end), by)
        {:cont, {:ok, Map.merge(changes, changed), by}}
      else
        {:error, why} ->
          {:halt, {:refuse, "#{plugin} couldn't say what it changes: #{why}"}}

        {name, _to} ->
          {:halt, {:refuse, "#{by[name]} and #{plugin} each change #{name} to something else. One has to give way."}}
      end
    end)
  end

  @doc """
  Asks each plugin that decides `hook` about it, for `asked`. Gives `:ok`, or `{:refuse, why}` from the first to
  refuse. `apart` is a plugin that isn't asked, as who it was added for and its name there. Where no plugin
  decides `reach` for them, a call or a reach is the network's first owner's alone.
  """
  def decide(hook, about, {by, who} = asked, apart \\ nil) do
    # Who decides is whoever's plugins they are, so what's kept is what they decided.
    kept = if hook == "reach", do: {"reach", {about, Plugins.chain(who)}}
    forgotten = forgotten("reach")

    if kept != nil and kept?(kept) do
      :ok
    else
      # Who may do what is decided only where something decides who reaches what.
      undecided = hook in ["call", "reach"] and hooking("reach", who) -- [apart] == []

      decided =
        with :ok <- if(undecided, do: undecided(by), else: :ok) do
          Enum.find_value(hooking(hook, who) -- [apart], :ok, fn {_at, plugin} = added ->
            case answered(added, hook, about, asked) do
              {:ok, "ok"} ->
                nil

              {:ok, %{"refuse" => why}} ->
                Logger.info("#{plugin} refused #{hook} for #{who}: #{why}")
                {:refuse, why}

              {:error, why} ->
                {:refuse, "#{plugin} couldn't decide it: #{why}"}
            end
          end)
        end

      # Kept only if nothing was forgotten meanwhile: it may have been decided from what has since changed.
      if kept != nil and decided == :ok and forgotten == forgotten("reach"),
        do: :ets.insert(@allowed, {kept, System.monotonic_time(:millisecond) + @kept_for})

      decided
    end
  end

  # What's decided where no plugin decides who may reach what.
  defp undecided(by) do
    cond do
      Application.get_env(:one_core, :development, false) -> :ok
      by != nil and by == OneCore.User.first_owner() -> :ok
      true -> {:refuse, "Nothing here decides who may do what, so only the network's first owner may do anything"}
    end
  end

  # Whether it's kept that this may be reached, and not for too long. The clock it's kept by starts anywhere.
  defp kept?(kept) do
    case :ets.lookup(@allowed, kept) do
      [{^kept, until}] -> until > System.monotonic_time(:millisecond)
      [] -> false
    end
  end

  @doc """
  Whose the device `device` is, as the plugins that change `device` say: a user, or `nil` for nobody's. Kept
  once it's somebody's, so a call asks nothing.
  """
  def whose(device) do
    kept = {"device", device}

    case :ets.lookup(@allowed, kept) do
      [{^kept, who}] ->
        who

      [] ->
        forgotten = forgotten("device")

        with {:ok, %{"who" => who}, _by} <- change("device", %{"device" => device, "who" => nil}, {nil, "network"}),
             true <- is_binary(who) and who =~ ~r/^users\/[a-z][a-z0-9-]*$/ do
          if forgotten == forgotten("device"), do: :ets.insert(@allowed, {kept, who})
          who
        else
          _ -> nil
        end
    end
  end

  # A hook whose plugin keeps what it needs where another server writes it is answered there.
  defp answered({at, plugin}, hook, about, asked) do
    case Calls.hook(at, plugin, hook, about, asked) do
      {:ok, answer} -> {:ok, answer}
      {:passed_on, %{"answer" => answer}} -> {:ok, answer}
      {:error, why} -> {:error, why}
    end
  end
end
