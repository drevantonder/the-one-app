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

  A hook may be asked more than once for one moment, as a tool may run more than once: a call passed on to
  another server is decided on both.
  """
  require Logger
  alias OneCore.Plugins

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
     A tool is about to be called. It's about the tool's `plugin`, which is `nil` for one of One Core's own tools,
     its `tool` name and the `inputs` it was asked with. The answer is the inputs to set, by name. The tool is called only if they are still what it takes.
     """},
    {"call", :decides,
     """
     A tool is about to be called. It's about the tool's `plugin`, which is `nil` for one of One Core's own tools,
     its `tool` name, the `inputs` it will be called with, and `changed`: the plugin that set each input that was changed, by the input's name.
     """},
    {"reach", :decides,
     """
     A plugin is about to reach an artifact that isn't its own. It's about `who` would reach it, the `relation`
     they need to it (`viewer`, `editor` or `owner`) and the `artifact`'s name. It's asked about whoever the
     plugin runs for, and about the plugin, by its artifact's name, where the plugin didn't make the artifact.
     It's also asked whether whoever asks is a `member` of the group or the network they ask for: then `who` is
     them, the `relation` is `member` and the `artifact` is the group, or `network`.
     """},
    {"called", :hears,
     """
     A tool was called. It's about what `call` is about, how long the tool `took` in microseconds, and the `error`
     it raised, which is `nil` if it answered.
     """},
    {"changed", :hears,
     """
     A blob of an artifact that a plugin made or reached was written or removed. It's about the `artifact`'s
     name, the `blob`'s name in it, and `by`, the plugin that did it, as a who. A plugin is told only if it could already
     reach the artifact for whoever changed it, and isn't told what it changed itself.
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
  def guide do
    kinds = for {kind, how} <- @kinds, do: "- A hook that #{kind} #{flat(how)}"
    hooks = for {hook, kind, about} <- @hooks, do: "- `#{hook}` #{kind}. #{flat(about)}"
    Enum.join(kinds ++ ["", "The hooks:", ""] ++ hooks, "\n")
  end

  defp flat(text), do: text |> String.split() |> Enum.join(" ")

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
        do: Task.Supervisor.start_child(__MODULE__, fn -> Plugins.hook(at, name, hook, about, asked) end)

    :ok
  end

  @doc """
  Asks each plugin that changes `hook` what it changes of what it's about, for `asked`. Gives `{:ok, changes, by}`:
  what's changed by name, and the plugin that changed each. `{:refuse, why}` if two don't agree, or one couldn't
  say.
  """
  def change(hook, about, {_by, who} = asked) do
    Enum.reduce_while(hooking(hook, who), {:ok, %{}, %{}}, fn {_at, plugin} = added, {:ok, changes, by} ->
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

  @doc "Asks each plugin that decides `hook` about it, for `asked`. Gives `:ok`, or `{:refuse, why}` from the first to refuse."
  def decide(hook, about, {_by, who} = asked) do
    Enum.find_value(hooking(hook, who), :ok, fn {_at, plugin} = added ->
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

  # A hook whose plugin keeps what it needs in a database another server writes is answered there.
  defp answered({at, plugin}, hook, about, asked) do
    case Plugins.hook(at, plugin, hook, about, asked) do
      {:ok, answer} -> {:ok, answer}
      {:passed_on, %{"answer" => answer}} -> {:ok, answer}
      {:error, why} -> {:error, why}
    end
  end
end
