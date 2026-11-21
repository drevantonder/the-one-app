defmodule OneCore.Hooks do
  @moduledoc """
  The moments an app can hook into, and the one way each is run. An app names the hooks it hooks into
  (`hooks/0`), and its `hook/3` is called at each, as that app, for whoever the moment is about. That's
  `asked` below: who asked for what brought the moment on, and who it runs for.

  The hooks are One Core's (`OneCore.App.Manifest`), and the ones apps offer (`offers/0`), each named with the app's name:
  `soft-delete.moved`. An app's hook is run here as One Core's are, and only that app runs it
  (`OneCore.Call.run_hook/3`).

  A hook is one of four kinds, as with Kubernetes' admission webhooks, which are mutating or validating, and its
  observers, and as with a list each app adds its own part to: an app observes it, mutates it, validates it or
  collects it. How a kind is run is what keeps apps from getting in each other's way:

  - No hook is handed what happens next to run or to leave out. It answers, and One Core carries on: nothing is
    blocked by an app that forgot to pass it on.
  - Nothing depends on the order apps are asked in. Apps that observe are each told apart from the others,
    every app that mutates is handed the same thing, every app that validates has to agree, and what each
    app that collects answers is kept apart from the others'.
  - What an app mutated is said, with its name, to the apps that validate and observe the same moment.

  An app hooks into the moments of whoever it was installed for. So a moment that runs for someone is hooked into
  by the apps installed for them and by the ones installed for the network, and by nobody else's: what one user installs
  has no say in another's.

  An app that can't answer, because it raised, its server couldn't be reached or it took longer than ten seconds
  (`within/3`), has refused: nothing is let through for want of an answer. So an app that breaks blocks what it
  mutates or validates, for whoever it was installed for and nobody else, until it's removed or deployed again, and
  it has no say in either: a call of a tool of One Core's that says `exempts_app` (`OneCore.Tools`) isn't the app
  it removes or deploys over's to mutate or validate.
  One that can't answer what it collects is left out, and only its own part is missing.

  A hook may be asked more than once for one moment, as a tool may run more than once: a call passed on to
  another server is validated on both.

  That something may be reached, and whose a device is, is kept by the server that asked, so that it asks once: for
  10 seconds (`@ttl`), or until a tool that doesn't only read is called of an app that validates it or says whose,
  which that server invalidates at once and tells the others to, or that app's edge function says to. A refusal is
  never kept, nor that a device is nobody's. So giving access is seen at once, and taking it away reaches every
  server it's told to at once and every other within 10 seconds: one that's stuck or out of reach as it's told still
  answers from what it kept for no longer than that, so nothing waits on telling it.

  Who may do what is an app's to decide. Where no app validates `reach` for a moment, nothing is reached and
  no tool is called but by the network's first owner (`OneCore.User.first_owner/0`), who is how a network whose
  validating app was removed is put right. A network being built, the server's `:development` setting, lets
  everything through instead.

  A history of who changed what, relation tuples included, is an app's to keep, through app hooks.
  """
  require Logger
  alias OneCore.{Cache, Apps}
  alias OneCore.Apps.Calls
  alias OpenTelemetryAPIExperimental.Counter
  require Counter

  # How long an app has to answer a hook, in milliseconds. A check is answered in well under a millisecond, and
  # the slowest answer is one whose app's server has just gone: finding that out and taking over from it
  # takes about three seconds, and more where the bucket is far away. This is that
  # with room to spare, and still short of a person giving up on what they asked.
  @timeout 10_000

  @doc """
  Runs `fun`, an app's `hook/3` at `hook`, and gives what it answered, or raises what it raised. One that hasn't
  answered within ten seconds is stopped and has refused: it raises, saying so.
  """
  def within(app, hook, fun) do
    task =
      Task.async(fn ->
        try do
          {:returned, fun.()}
        catch
          kind, reason -> {kind, reason, __STACKTRACE__}
        end
      end)

    case Task.yield(task, @timeout) || Task.shutdown(task, :brutal_kill) do
      {:ok, {:returned, result}} -> result
      {:ok, {kind, reason, stacktrace}} -> :erlang.raise(kind, reason, stacktrace)
      _ ->
        # Said as a number too, by its app, whose not answering is the symptom (Telemetry's `unanswered` alert).
        Counter.add(:"one.hook.timeouts", 1, %{"one.app" => to_string(app), "one.hook" => to_string(hook)})
        raise "#{app} didn't answer #{hook} within #{div(@timeout, 1000)} seconds"
    end
  end

  # How long a server keeps that something may be reached, or whose a device is, in milliseconds, where nothing told it
  # to invalidate it: how long a grant that's taken away may still be answered from a server that wasn't told.
  @ttl 10_000

  # Where what each of `cached/0` answered is cached. These two are asked of the same thing again and again: whether
  # someone may reach an artifact, at every read of one, and whose a device is, at every call from it. What the
  # others are asked is a call's own inputs or a moment that happens once, so an answer kept would never be asked
  # again.
  @caches %{"reach" => __MODULE__.Reach, "device" => __MODULE__.Device}

  @doc "Makes where which apps hook into what is kept. Whatever calls this owns it."
  def keep, do: :ets.new(__MODULE__, [:named_table, read_concurrency: true])

  @doc "The hooks whose answers a server caches."
  def cached, do: Map.keys(@caches)

  @doc """
  Invalidates what `hooks` answered, so that they're asked again: who may reach what may have changed, or whose a
  device is.
  """
  def invalidate(hooks \\ cached()) do
    for hook <- hooks, cache = @caches[hook], do: Cache.invalidate(cache)
    :ok
  end

  @doc """
  The apps that hook into `hook` for a moment that runs for `who`: each as who it was installed for, and its name
  there. Which hook into what is kept by `OneCore.Apps`, by who they were installed for, as it looks at them.
  """
  def hooking(hook, who) do
    for at <- Apps.chain(who),
        {hooking, _offered} = installed(at),
        name <- Map.get(hooking, hook, []),
        do: {at, name}
  end

  @doc "The kind of the hook `hook` that an app installed for `owner` offers, by its full name, or `nil` for none."
  def offered(hook, owner), do: with({kind, _untrusted} <- elem(installed(owner), 1)[hook], do: kind)

  @doc """
  Whether an untrusted app may answer `hook`: a collect that an app installed for one of `owners` offers saying so
  (`untrusted: true`), where what it answers is only its own part, read as data. None of One Core's hooks is.
  """
  def untrusted?(hook, owners), do: Enum.any?(owners, &match?({:collect, true}, elem(installed(&1), 1)[hook]))

  # The hooks that the apps installed for an owner hook into, each with those apps' names, and the kinds of the
  # hooks they offer, by their full names. An owner this server hasn't looked at is looked at first.
  defp installed(owner) do
    with [] <- :ets.lookup(__MODULE__, owner),
         _ = Apps.sync(owner),
         [] <- :ets.lookup(__MODULE__, owner) do
      {%{}, %{}}
    else
      [{^owner, hooking, offered}] -> {hooking, offered}
    end
  end

  @doc """
  What an app's `hook/3` answered `hook`, of `kind`, as it's passed between servers: what JSON holds. Raises if it
  isn't an answer of that kind.
  """
  def check_result(kind, hook, result) do
    case {kind, result} do
      {:observe, _} ->
        nil

      {:mutate, %{} = changes} ->
        OneCore.JSON.normalize(changes)

      {:validate, :ok} ->
        "ok"

      {:validate, {:refuse, why}} when is_binary(why) ->
        %{"refuse" => why}

      {:collect, part} ->
        OneCore.JSON.normalize(part)

      {:mutate, _} ->
        raise "It answered #{inspect(result)} to #{hook}, which is answered with what it changes, by name"

      {:validate, _} ->
        raise "It answered #{inspect(result)} to #{hook}, which is answered :ok or {:refuse, why}"
    end
  end

  @doc "What an app that has nothing to say answers a hook of `kind`."
  def nothing(:observe), do: nil
  def nothing(:mutate), do: %{}
  def nothing(:validate), do: :ok
  def nothing(:collect), do: nil

  @doc "Tells each app that observes `hook` what it's about, for `asked`."
  def observe(hook, about, {_actor, subject} = principal) do
    for {at, name} <- hooking(hook, subject), do: __MODULE__.Observer.notify(at, name, hook, about, principal)
    :ok
  end

  @doc """
  Asks each app that mutates `hook` what it changes of what it's about, for `asked`. Gives `{:ok, changes, by}`:
  what's changed by name, and the app that changed each. `{:refuse, why}` if two don't agree, or one couldn't
  say.
  """
  def mutate(hook, about, {_actor, subject} = principal) do
    Enum.reduce_while(asked(hook, about, subject), {:ok, %{}, %{}}, fn {_at, app} = installed, {:ok, changes, by} ->
      with {:ok, changed} <- call_hook(installed, :mutate, hook, about, principal),
           nil <- Enum.find(changed, fn {name, to} -> is_map_key(changes, name) and changes[name] != to end) do
        by = Map.merge(Map.new(changed, fn {name, _to} -> {name, app} end), by)
        {:cont, {:ok, Map.merge(changes, changed), by}}
      else
        {:error, why} ->
          {:halt, {:refuse, "#{app} couldn't say what it changes: #{why}"}}

        {name, _to} ->
          {:halt, {:refuse, "#{by[name]} and #{app} each change #{name} to something else. One has to give way."}}
      end
    end)
  end

  @doc """
  Asks each app that validates `hook` about it, for `asked`. Gives `:ok`, or `{:refuse, why}` from the first to
  refuse. Where no app validates `reach` for them, a call or a reach is the network's first owner's alone.
  """
  def validate(hook, about, {_actor, subject} = principal) do
    case @caches[hook] do
      nil ->
        decide(hook, about, principal)

      # Who validates is whoever's apps they are, so what's cached is what they validated. A refusal never is.
      cache ->
        Cache.fetch(cache, {about, Apps.chain(subject)}, fn ->
          with :ok <- decide(hook, about, principal),
               do: {:commit, :ok, expire: @ttl},
               else: (refused -> {:ignore, refused})
        end)
    end
  end

  defp decide(hook, about, {actor, subject} = principal) do
    # Who may do what is decided only where an app validates who reaches what.
    undecided = hook in ["call", "reach"] and asked("reach", about, subject) == []

    with :ok <- if(undecided, do: undecided(actor), else: :ok) do
      Enum.find_value(asked(hook, about, subject), :ok, fn {_at, app} = installed ->
        case call_hook(installed, :validate, hook, about, principal) do
          {:ok, "ok"} ->
            nil

          {:ok, %{"refuse" => why}} ->
            Logger.info("#{app} refused #{hook} for #{subject}: #{why}")
            {:refuse, why}

          {:error, why} ->
            {:refuse, "#{app} couldn't validate it: #{why}"}
        end
      end)
    end
  end

  @doc """
  Asks each app that collects `hook` for its part, for `asked`, all at once (`OneCore.Asks.each/3`), each under its own
  time (`within/3`). Gives `{:ok, parts, failed}`: each app's part by its name, leaving out those that answered `nil`,
  and why each app that couldn't answer couldn't, by its name.
  """
  def collect(hook, about, {_actor, subject} = principal) do
    # A second past `within/3`'s, which stops an app's hook that hasn't answered and says so: this one is for what
    # comes before it, as waiting for the app's version.
    {answers, failed} =
      OneCore.Asks.each(hooking(hook, subject), &call_hook(&1, :collect, hook, about, principal), @timeout + 1_000)

    Enum.reduce(answers, {:ok, %{}, Map.new(failed, fn {{_at, app}, why} -> {app, why} end)}, fn
      {_installed, {:ok, nil}}, collected -> collected
      {{_at, app}, {:ok, part}}, {:ok, parts, failed} -> {:ok, Map.put(parts, app, part), failed}
      {{_at, app}, {:error, why}}, {:ok, parts, failed} -> {:ok, parts, Map.put(failed, app, why)}
    end)
  end

  # The apps asked about `hook` for a moment that runs for `subject`: those that hook into it, but the one a call of a
  # tool that says `exempts_app` removes or deploys over (`OneCore.Tools`), which has no say in it.
  defp asked(hook, %{"app" => nil, "tool" => tool, "inputs" => inputs}, subject) do
    if Enum.find(OneCore.Tools.Definitions.tools(), &(&1.name == tool))[:exempts_app],
      do: hooking(hook, subject) -- [{OneCore.User.owner(subject), inputs["name"]}],
      else: hooking(hook, subject)
  end

  defp asked(hook, _about, subject), do: hooking(hook, subject)

  # What's decided where no app validates who may reach what.
  defp undecided(by) do
    cond do
      Application.get_env(:one_core, :development, false) -> :ok
      by != nil and by == OneCore.User.first_owner() -> :ok
      true -> {:refuse, "Nothing here decides who may do what, so only the network's first owner may do anything"}
    end
  end

  @doc """
  Has the apps installed for the network that can wake the device `device` wake it (the hook `wake`, as
  Notifications does by a push), as a server does before it asks one whose app isn't running
  (`OneCore.Server.ask_device/3`). Whether one did.
  """
  def wake(device) do
    {:ok, woken, _failed} = collect("wake", %{"device" => device}, {nil, "network"})
    Enum.any?(woken, fn {_app, part} -> part["woken"] == true end)
  end

  @doc """
  Whose the device `device` is, as the apps that mutate `device` say: a user, or `nil` for nobody's. Kept
  once it's somebody's, so a call asks nothing. `{:error, why}` where an app couldn't say, or none that says runs
  here (as while it waits for this server's update), which isn't nobody's: only an app says a device is nobody's, and
  a device a client is told is nobody's is signed out.
  """
  def whose(device) do
    Cache.fetch(@caches["device"], device, fn ->
      with [_ | _] <- hooking("device", "network"),
           {:ok, %{"who" => who}, _by} <- mutate("device", %{"device" => device, "who" => nil}, {nil, "network"}),
           :ok <- OneCore.User.user(who) do
        {:commit, who, expire: @ttl}
      else
        [] -> {:ignore, {:error, "no app that says whose a device is runs on this server"}}
        {:refuse, why} -> {:ignore, {:error, why}}
        _ -> {:ignore, nil}
      end
    end)
  end

  # A hook whose app keeps what it needs where another server writes it is answered there.
  defp call_hook({at, app}, kind, hook, about, principal) do
    case Calls.hook(at, app, kind, hook, about, principal) do
      {:ok, result} -> {:ok, result}
      {:forwarded, %{"answer" => result}} -> {:ok, result}
      {:error, why} -> {:error, why}
    end
  end
end
