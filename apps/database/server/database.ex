defmodule Database do
  @moduledoc """
  The database: a kind of artifact that other apps keep their data in. It is SQLite, and the app's own
  artifact holds it as a log and snapshots (`Database.Log`), which one server at a time writes (`Database.Lease`).
  """
  @behaviour OneCore.App

  @impl true
  def icon, do: "icon.svg"

  @impl true
  def requires, do: %{"core" => ">= 0.34.250"}

  require Logger
  alias Database.Lease
  alias OneCore.Call

  @doc """
  Runs `fun` in a transaction on the calling app's database, after the `schema` statements it hasn't run yet,
  and gives what `fun` returns once what it wrote is in the bucket.
  """
  def transaction(%Call{} = call, schema \\ [], fun) when is_list(schema) and is_function(fun, 1) do
    lease = Lease.of(call)
    if lease == self(), do: raise("A transaction is already running here: use `query` inside it")

    job = Lease.job(lease)

    case :gen_statem.call(lease, {call.own.codec, {:transaction, schema, fun, job}}) do
      {:ok, result} ->
        if job, do: Lease.marked()
        result

      :codec_changed ->
        opened_again(lease)
        transaction(call, schema, fun)

      {:raised, kind, reason, stacktrace} ->
        :erlang.raise(kind, reason, stacktrace)

      # The call ends here if the server holding the lease runs it, and otherwise goes on with who holds it now.
      {:held_by, server} ->
        taken_over(call, lease, server)
        transaction(call, schema, fun)
    end
  end

  # How long, in milliseconds, a server that's there takes at most to answer a new connection: QUIC has sent what
  # it sends again by then (`client::THERE`).
  @waited 1_000

  # How long, in milliseconds, the server holding the lease has to run a call passed on to it that's safe to ask
  # again, a hook, a tool that only reads or a write by its idempotency key (`OneCore.Writes`), before it's taken as gone. One that's there but stuck keeps the call
  # open, saying it's still running it, and a hook gives up at 10 s (`OneCore.Hooks`): so every hook passed to a
  # stuck holder gave up before anything took the lease back, and the next did the same. A holder that's up runs one
  # in well under a second, and this and `@witness` leave a hook time to take the lease over and answer.
  @reach 4_000

  # How long, in milliseconds, a server another passed a call to, since that one couldn't reach the holder, gives the
  # holder to run it: a holder that's up runs one in well under a second.
  @witness 2_000

  # How long, in milliseconds, the relay at the network's address gives the holder to answer its ping
  # (`OneCore.Servers.answers?/2`), which is asked as the holder is. A holder that's up answers it in well under a
  # second. It's also how long a read or a hook is asked of the holder before the relay's saying it doesn't answer
  # either ends the wait (`Call.forward_to/4`), where it waited out `@reach`: a holder frozen with its relay connection
  # open is taken over from in about this, not 4 s. One that is only slow, and didn't answer the relay in this time, is
  # taken over from too: that costs it its lease, not data, since a server writes only by the conditional write of the
  # log's next entry, so of two that think they hold it one writes, and the one that took it writes nothing until the
  # one before has stopped answering from its copy (`Database.Lease`).
  @ping 1_500

  # The server holding the lease didn't run a call passed on to it: it's gone, or it can't run this call, as one
  # that needs what only another server has can't be passed on again (a Vault item only some servers open, say).
  # Not reaching it just now isn't its being gone: a network loses a connection now and then, and what this server
  # knew of it may be from before it came back. So one given up on at once is asked once more first. One that was
  # waited for has had what it was sent sent again by then (`@waited`).
  #
  # Then the call is passed to the other servers that may hold the database, as SWIM's indirect probe asks other
  # members to reach one that didn't answer: one that reaches the holder has it run there, and the lease stays where
  # the others reach it, and one that can't either takes the lease, two servers having failed to reach it, and runs it.
  # Only where none of them answered, or there's none, does this server take the lease itself, as the log lets it
  # (`Database.Lease`), unless the relay at the network's address still reaches a holder that took it from this one.
  # The relay is asked as the holder is, not after it gave up, and its saying the holder doesn't answer ends the wait
  # for it, of a read or a hook, once `@ping` has passed: a frozen holder's first call waited 10 s, one after the other,
  # and then the slower of the two. A write isn't given up on then, as the holder may have begun it, and the server
  # that took the lease then answers what the holder answers it (`OneCore.Writes`).
  # A server that isn't up takes no lease, so the call fails here, for whoever asked to ask another, as it does where
  # this server is the one out of reach. Whoever asked is told only that the app couldn't be reached: which server
  # holds what, and how long it waited, is for whoever reads this server's log.
  #
  # How long the holder kept it is said as a number of the app's (Telemetry's `one.database.lease.wait`):
  # `not_answered` where it kept it `@waited` or more without running it, as one that's stuck does, which is warned
  # of, `gone` where it said at once that it wouldn't, and `connected` where the relay still reached it after it
  # took the lease from this one, and nothing else could be asked.
  defp taken_over(call, lease, server) do
    began = System.monotonic_time(:millisecond)
    # A call another server passed here to run, as one whose own copy failed passes its own (`:failed`), or as one
    # that couldn't reach the holder does, is asked of the holder once, for `@witness`, and passed on no further.
    passed = call.forwarded == :to
    relay = if server && not passed, do: Task.async(fn -> OneCore.Servers.answers?(server, @ping) end)

    try do
      within = if passed, do: @witness, else: @reach
      first = if server, do: Call.forward_to(call, server, within, relay && {relay, @ping})
      again = if server && not passed && System.monotonic_time(:millisecond) - began < @waited, do: Call.forward_to(call, server, @reach)
      waited = System.monotonic_time(:millisecond) - began
      whose = OneCore.User.owner(OneCore.Artifact.name(call.own))
      # One that answered it would pass the call on in turn is there, and can't run it: the others aren't asked.
      cannot_run = :forwarding_loop in [first, again]
      others = if server && not passed && not cannot_run, do: OneCore.Apps.servers(whose) -- [server, OneCore.Server.key()], else: []
      if others != [], do: Call.forward(call, others)

      why =
        cond do
          cannot_run -> :cannot_run
          passed -> :unanswered
          Task.await(relay, :infinity) == true -> :relayed
          true -> :alone
        end

      took = :gen_statem.call(lease, {:take_over, server, why})
      app = Lease.app(call.own)
      holder = server && (OneCore.Servers.name(server) || server)

      if server do
        outcome = if took == :connected, do: "connected", else: if(waited >= @waited, do: "not_answered", else: "gone")
        Lease.waited(waited, %{"one.app" => app, "one.outcome" => outcome})

        if outcome == "not_answered",
          do: Logger.warning("#{app}'s database: #{holder} holds its lease and didn't answer within #{waited} ms, so this server takes it")
      end

      case took do
        :ok ->
          :ok

        :connected ->
          Logger.warning("#{app}'s database: #{holder} took its lease from this server and answers the network's relay, but didn't answer this server within #{waited} ms, and no other server could be asked, so this server doesn't take it back")
          unreachable(call)

        # This server is stopping, or not one of its network's yet, so it takes no lease that others would then come
        # to it for: the call goes to one of the servers that are, which takes it.
        :not_up ->
          Call.forward(call, OneCore.Apps.servers(whose))
          held = if server, do: "#{holder} holds it and didn't answer", else: "nobody holds it"
          Logger.warning("#{app}'s database: this server is starting or stopping, so it doesn't take its lease over, #{held}, and no other server took it")
          unreachable(call)

        # This server's own copy failed just now, and it handed the lease on: a call asked here is passed to another of
        # the servers acting for whoever the database is, which takes the lease and runs it, and one passed on here is
        # answered as one this server would pass on again, so the server that passed it takes the lease (`Call.forward/2`).
        :failed ->
          Call.forward(call, OneCore.Apps.servers(whose))
          Logger.warning("#{app}'s database: this server's copy failed just now, so it handed its lease on, and no other server took it")
          unreachable(call)
      end
    after
      if relay, do: Task.shutdown(relay, :brutal_kill)
    end
  end

  # What whoever asked is told of a call whose database couldn't be reached, by the name they know its app by.
  defp unreachable(call) do
    app = OneCore.Apps.version(call.app_artifact)
    raise "#{(app && app.title) || call.app} couldn't be reached just now. Try again in a moment."
  end

  @doc """
  Has each of the calling app's scheduled `jobs` that is due called with `fire`, once, by the server holding
  the lease of its database: here, or by passing this on to the one that does, and taking the lease if that one
  doesn't answer. One Core calls this on every server, each time a job is due (`OneCore.App`).
  """
  @impl true
  def schedule(%Call{} = call, jobs, fire) when is_list(jobs) and is_function(fire, 1) do
    lease = Lease.of(call)

    case :gen_statem.call(lease, {call.own.codec, {:schedule, jobs, fire, System.os_time(:second)}}) do
      # What's due is being made, and its timers' alarm says when the next is, so the clock comes back then.
      :ok ->
        alarm(call, transaction(call, fn db -> query(db, timers()) && next(db) end))

      {:raised, kind, reason, stacktrace} ->
        :erlang.raise(kind, reason, stacktrace)

      :codec_changed ->
        opened_again(lease)
        schedule(call, jobs, fire)

      {:held_by, server} ->
        taken_over(call, lease, server)
        schedule(call, jobs, fire)
    end
  end

  # A database that was open under its artifact's codec before has closed, for the next call to open it.
  defp opened_again(lease) do
    closing = Process.monitor(lease)

    receive do
      {:DOWN, ^closing, :process, _lease, _why} -> :ok
    end
  end

  @doc """
  Sets a timer for whoever the calling app's call keeps data for, as Android's WorkManager or a Durable Object's
  alarm does: the app's `tool` is called with `inputs` at `at`, in seconds since 1970, once, or with `every`, every
  that many seconds after, by the server holding this database's lease, which One Core's clock finds within a
  minute of it (`OneCore.Jobs`). Setting it again for the same tool and inputs moves it, and `at` `nil` removes it.
  It's kept in the database, so it's made once whichever server holds it, and after any of them starts again.
  """
  def timer(%Call{} = call, tool, inputs, at, every \\ nil) when is_binary(tool) and is_map(inputs) do
    job = "#{tool} #{JSON.encode!(inputs)}"

    next =
      transaction(call, fn db ->
        query(db, timers())

        if at,
          do: query(db, set_timer(), [job, tool, JSON.encode!(inputs), at, every]),
          else: query(db, "delete from database_timers where job = ?", [job])

        next(db)
      end)

    alarm(call, next)
  end

  @doc false
  def timers,
    do: "create table if not exists database_timers (job text primary key, tool text not null, inputs text not null, at integer not null, every integer) without rowid"

  defp set_timer,
    do: "insert into database_timers (job, tool, inputs, at, every) values (?, ?, ?, ?, ?) on conflict (job) do update set at = excluded.at, every = excluded.every"

  defp next(db), do: with([%{"next" => next}] <- query(db, "select min(at) as next from database_timers"), do: next)

  # Says when this database's next timer is due, `next`, or that it has none, as the one key One Core's clock looks
  # for (`OneCore.Jobs`), `<whose>/alarms/<installed for>/<app>/<due>`, under whose data it is.
  defp alarm(call, next) do
    whose = OneCore.User.owner(OneCore.Artifact.name(call.own))
    prefix = "#{whose}/alarms/#{call.installed}/#{call.app}/"
    keys = OneCore.Bucket.list(prefix)
    if next && "#{prefix}#{next}" not in keys, do: OneCore.Bucket.write("#{prefix}#{next}", "")
    for key <- keys, key != "#{prefix}#{next}", do: OneCore.Bucket.remove(key)
    :ok
  end

  @doc "Runs one SQL statement in a transaction, with a `?` for each value, and gives its rows."
  def query({sqlite, db}, sql, values \\ []) do
    case sqlite.query(db, sql, values) do
      {:ok, rows} -> rows
      {:error, why} -> raise "#{why}, in: #{sql}"
    end
  end
end
