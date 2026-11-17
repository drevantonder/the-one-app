defmodule OneCore.Hooks do
  @moduledoc """
  The moments an app can hook into, and the one way each is run. An app names the hooks it hooks into
  (`hooks/0`), and its `hook/3` is called at each, as that app, for whoever the moment is about. That's
  `asked` below: who asked for what brought the moment on, and who it runs for.

  The hooks are One Core's, below, and the ones apps offer (`offers/0`), each named with the app's name:
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
  it has no say in either: a call that removes an app or deploys over it isn't that app's to mutate or validate.
  One that can't answer what it collects is left out, and only its own part is missing.

  A hook may be asked more than once for one moment, as a tool may run more than once: a call passed on to
  another server is validated on both.

  That something may be reached is kept by the server that asked, so that it asks once: until a tool that doesn't
  only read is called of an app that validates it, which that server and the ones it tells invalidate at once,
  and for a few seconds anywhere else. Whose a device is is kept the same way, until a server is told to invalidate it:
  by a tool of an app that says whose, or by that app's edge function. A refusal is never
  kept, nor that a device is nobody's. So taking access away reaches every server, and giving it is seen at once.

  Who may do what is an app's to decide. Where no app validates `reach` for a moment, nothing is reached and
  no tool is called but by the network's first owner (`OneCore.User.first_owner/0`), who is how a network whose
  validating app was removed is put right. A network being built, the server's `:development` setting, lets
  everything through instead.

  A history of who changed what, relation tuples included, is an app's to keep, through app hooks.
  """
  require Logger
  alias OneCore.{Cache, Apps}
  alias OneCore.Apps.Calls

  @kinds [
    observe: """
    is told after it happened, on the server where it happened, apart from what happened and from every other
    app that observes it, in the order it happened. What it answers is ignored, and so is its raising. One that
    needs a server acting for whoever it was installed for passes it there (`OneCore.Call.forward/2`).
    """,
    mutate: """
    is handed it as it was asked, as every other app that mutates it is, and answers only what it changes, by
    name: `%{}` changes nothing. Two apps that change the same thing to something different refuse it, and say
    so. Which app changed what is said to the apps that validate and observe it.
    """,
    validate: """
    answers `:ok` or `{:refuse, why}`. Every app that validates it has to answer `:ok`: one refusal refuses it,
    and whoever asked is told why. An app that raises, or answers anything else, refuses it too.
    """,
    collect: """
    is asked, as every other app that collects it is, and answers its own part of what's collected, which is
    kept apart from every other app's, by its name: `nil` is no part. One that can't answer is left out, and
    whoever ran the hook is told which, and why: an app that breaks takes away its own part and nothing else.
    """
  ]

  @hooks [
    {"inputs", :mutate,
     """
     A tool is about to be called. It's about the tool's `app`, which is `nil` for one of One Core's own tools, its
     `tool` name and the `inputs` it was asked with. The answer is the inputs to set, by name. The tool is called only
     if they are still what it takes.
     """},
    {"call", :validate,
     """
     A tool is about to be called. It's about the tool's `app`, which is `nil` for one of One Core's own tools, its
     `tool` name, the `inputs` it will be called with, and `changed`: the app that set each input that was changed,
     by the input's name.
     """},
    {"reach", :validate,
     """
     An app is about to reach an artifact that isn't its own. It's about `who` would reach it, the `relation`
     they need to it (`viewer`, `editor` or `owner`) and the `artifact`'s name. It's asked about whoever the
     app runs for, and about the app, by its artifact's name, where the app didn't make the artifact.
     An `:ok` is kept for a few seconds, and until a tool of the app is called that doesn't only read: so
     what it validates from is what its own tools change.
     It's also asked whether whoever asks is a `member` of the group or the network they ask for: then `who` is
     them, the `relation` is `member` and the `artifact` is the group, or `network`.
     """},
    {"device", :mutate,
     """
     A device asks, and One Core is about to say whose it is. It's about the `device`, by its key, and `who`, which
     is nobody: `nil`. The answer is `who`: the user whose device it is, as `users/<name>`. With no app
     answering, the device is nobody's, and nobody is refused everything. It's asked of the apps installed for the
     network, since nobody is asking yet, and what's answered is kept until a tool of the app is called that
     doesn't only read.
     """},
    {"owner", :validate,
     """
     The network is about to have its first owner. It's about `who` they are. An app that keeps who may do what
     writes that down before it answers, and refuses if it can't: nothing else tells it.
     """},
    {"called", :observe,
     """
     A tool was called. It's about what `call` is about, how long the call `took` in microseconds, its `error`,
     which is `nil` if it answered and otherwise why it was refused or what it raised, whether it was `forwarded`
     to another server to run, whether its app makes it an MCP `task` (`OneCore.Tasks`), and its `trace`: its
     `traceId`, `spanId` and `parentSpanId` (`OneCore.Trace`). A call passed on is observed on both servers.
     """},
    {"changed", :observe,
     """
     A blob of an artifact that an app made or reached was written or removed. It's about the `artifact`'s name, the
     `blob`'s name in it, and `by`, the app that did it, as a who. An app is told only if it could already reach
     the artifact for whoever changed it, and isn't told what it changed itself.
     """},
    {"measured", :observe,
     """
     A client said what it measured of itself. It's observed by the apps installed for the network, and it's about the
     `device`'s key and `who` it's of, as its request proved, and its `records`, each as Telemetry's tool `record`
     takes one, with the `time` it was made, in milliseconds since 1970, and the client's own say of which it is in
     its `attributes`: what went wrong or was slow there, kept on the device until a server could be told.
     """},
    {"uninstalled", :observe,
     """
     An app was uninstalled for someone. It's about who it was installed `for`, the `name` it was installed by
     there and the `app` it is, by its artifact's name, and `with`, each app installed for them that required it
     and went with it, by its `name` and `app`. An app that keeps what's deleted takes what they kept and made for
     them along, as one.
     """},
    {"restored", :observe,
     """
     An app was brought back from Recently Deleted for someone. It's about who it's installed `for`, the `name` it's
     installed by there and the `app` it is. An app that keeps what's deleted brings back what it kept and made for
     them with it.
     """},
    {"joined", :observe,
     """
     A server was let act for someone. It's about the `server`'s key, who it's `for`, and `by`, the key of the
     server that installed it.
     """}
  ]

  @doc "Every one of One Core's hooks' names, with its kind: `:observe`, `:mutate`, `:validate` or `:collect`."
  def kinds, do: Map.new(@hooks, fn {hook, kind, _about} -> {hook, kind} end)

  @doc """
  Whether `hook` is a name an app can hook into: one of One Core's hooks, or an app's, `<app>.<hook>`, which
  may not be installed yet: then it's never run.
  """
  def hook?(hook),
    do: is_map_key(kinds(), hook) or (is_binary(hook) and hook =~ ~r/^[a-z0-9][a-z0-9-]*\.[a-z][a-z0-9_]*$/)

  @doc "What an app's author reads about hooks: each kind, and each of One Core's hooks with its kind and what it's about."
  def listed do
    kinds = for {kind, how} <- @kinds, do: "- An app that hooks into a hook of kind `#{inspect(kind)}` #{flat(how)}"
    Enum.join(kinds ++ ["", "The hooks:", "", listed(@hooks)], "\n")
  end

  @doc "Hooks, each `{name, kind, about}`, as an app's author reads them."
  def listed(hooks), do: Enum.map_join(hooks, "\n", fn {hook, kind, about} -> "- `#{hook}` #{kind}. #{flat(about)}" end)

  defp flat(text), do: text |> String.split() |> Enum.join(" ")

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
      _ -> raise "#{app} didn't answer #{hook} within #{div(@timeout, 1000)} seconds"
    end
  end

  # How long a server keeps that something may be reached, in milliseconds, where nothing told it to invalidate it.
  @ttl 10_000

  # Where what each of `cached/0` answered is cached.
  @caches %{"reach" => __MODULE__.Reach, "device" => __MODULE__.Device}

  @doc "Makes where which apps hook into what is kept, and what was decided. Whatever calls this owns them."
  def keep do
    :ets.new(__MODULE__, [:named_table, read_concurrency: true])
    Enum.each(Map.values(@caches), &Cache.new/1)
  end

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
  def offered(hook, owner), do: elem(installed(owner), 1)[hook]

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
        JSON.decode!(JSON.encode!(changes))

      {:validate, :ok} ->
        "ok"

      {:validate, {:refuse, why}} when is_binary(why) ->
        %{"refuse" => why}

      {:collect, part} ->
        JSON.decode!(JSON.encode!(part))

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
  say. `apart` is an app that isn't asked, as who it was installed for and its name there.
  """
  def mutate(hook, about, {_actor, subject} = principal, apart \\ nil) do
    Enum.reduce_while(hooking(hook, subject) -- [apart], {:ok, %{}, %{}}, fn {_at, app} = installed,
                                                                             {:ok, changes, by} ->
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
  refuse. `apart` is an app that isn't asked, as who it was installed for and its name there. Where no app
  validates `reach` for them, a call or a reach is the network's first owner's alone.
  """
  def validate(hook, about, {_actor, subject} = principal, apart \\ nil) do
    # Who validates is whoever's apps they are, so what's cached is what they validated. A refusal never is.
    if hook == "reach" do
      Cache.fetch(@caches["reach"], {about, Apps.chain(subject)}, fn ->
        with :ok <- decide(hook, about, principal, apart),
             do: {:commit, :ok, expire: @ttl},
             else: (refused -> {:ignore, refused})
      end)
    else
      decide(hook, about, principal, apart)
    end
  end

  defp decide(hook, about, {actor, subject} = principal, apart) do
    # Who may do what is decided only where an app validates who reaches what.
    undecided = hook in ["call", "reach"] and hooking("reach", subject) -- [apart] == []

    with :ok <- if(undecided, do: undecided(actor), else: :ok) do
      Enum.find_value(hooking(hook, subject) -- [apart], :ok, fn {_at, app} = installed ->
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
  Asks each app that collects `hook` for its part, for `asked`. Gives `{:ok, parts, failed}`: each app's part
  by its name, leaving out those that answered `nil`, and why each app that couldn't answer couldn't, by its name.
  """
  def collect(hook, about, {_actor, subject} = principal) do
    Enum.reduce(hooking(hook, subject), {:ok, %{}, %{}}, fn {_at, app} = installed, {:ok, parts, failed} ->
      case call_hook(installed, :collect, hook, about, principal) do
        {:ok, nil} -> {:ok, parts, failed}
        {:ok, part} -> {:ok, Map.put(parts, app, part), failed}
        {:error, why} -> {:ok, parts, Map.put(failed, app, why)}
      end
    end)
  end

  # What's decided where no app validates who may reach what.
  defp undecided(by) do
    cond do
      Application.get_env(:one_core, :development, false) -> :ok
      by != nil and by == OneCore.User.first_owner() -> :ok
      true -> {:refuse, "Nothing here decides who may do what, so only the network's first owner may do anything"}
    end
  end

  @doc """
  Whose the device `device` is, as the apps that mutate `device` say: a user, or `nil` for nobody's. Kept
  once it's somebody's, so a call asks nothing. `{:error, why}` where an app couldn't say, which isn't nobody's.
  """
  def whose(device) do
    Cache.fetch(@caches["device"], device, fn ->
      with {:ok, %{"who" => who}, _by} <- mutate("device", %{"device" => device, "who" => nil}, {nil, "network"}),
           true <- is_binary(who) and who =~ ~r/^users\/[a-z][a-z0-9-]*$/ do
        {:commit, who}
      else
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
