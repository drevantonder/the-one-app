defmodule Database.Lease do
  @moduledoc """
  One database on this server: open here while this server holds its lease, and otherwise only known to be held by
  another.

  The server holding the lease keeps the database as a file of its own, which is only a copy: the log in the
  bucket is the database. A transaction commits to that file, and its pages are then written as the log's next
  entry. Only once that's written is the transaction answered, so nothing answered is lost with this server. If
  another server wrote that entry first, this one lost the lease without knowing: it throws its copy away, and
  what it had committed there with it.

  A server that can't reach the one holding the lease takes it, even while that one is up and answering others, and
  that one learns so only by looking at the log. So a server answers from its copy only for a few seconds after it
  last saw the log's next entry free, and looks again before it answers after that; and a server that has just
  opened the database writes nothing to it for a little longer. One that lost the lease without knowing has stopped
  answering from what it had before the database changes.

  One transaction runs at a time, and the next begins after the last is in the bucket, so none sees what could
  still be lost.

  The server holding the lease also makes the calls of the app's scheduled jobs. When each job last ran is a
  row of the database, in `database_scheduled`: the time it was due at. A job that's due is called in a process
  of its own, whose transactions carry the job and that time (`job/0`), and the first that commits marks it as run,
  in that same transaction, or is refused where it's marked already. So the mark and what the call wrote reach the
  log in one entry, or neither does, whichever process keeps the database by then: a server that takes the lease, or
  this one's started again, reads from the log whether the call is still to make, and a call it makes again meanwhile
  writes nothing. A call that made no transaction is marked when it ends.

  A call says what wraps the artifact it has (`OneCore.Artifact`). If that isn't what this was opened with, the
  database closes, and the call opens it again: an encrypted artifact's keys are given anew to a server that was
  taken off it and chosen again, and what it had open is from before another server wrote.

  It's a `:gen_statem`, in one of these states:

  - `{:following, server}`: `server` holds the lease. A transaction is answered with who holds it, to be passed on
    to them. One they didn't run has this server read what was logged after the last entry it read, and take the
    lease if they handed it on, or if that's nothing and it's not the one out of reach (`Database.transaction/3`);
    one that another server took meanwhile is followed there. `nil` is nobody: the log is empty or handed on, and this server isn't up
    to take it.
  - `:settling`: this server has just opened the database, and writes nothing until `@writes_after` has passed.
    Its copy is read-only meanwhile: a transaction that only reads is answered from it at once, and one that would
    write raises there, is rolled back, and waits until then to run again, with any that came after it that write.
  - `:holding`: each transaction's write is logged as it commits.

  It moves between them by reading the log (`look/1`): to `:settling` with the database opened if the log names
  this server, after taking the lease if the log is empty, and otherwise to following the server it names. One that
  finds another server wrote the log's next entry has lost the lease, and reads the log again.

  Only a server that's up takes the lease (`OneCore.Server.up?/0`): a build being tried that can't be reached by its
  key, or a server that's stopping, would have everyone pass their calls to it and wait. It still holds the lease
  the log gave it before it started, which everyone passes their calls to already.

  A server whose own work on the database fails, as one out of files does, hands its lease on as well, and takes
  none for a while: an app's own error is its caller's, but one opening its copy or reading SQLite's log is this
  server's, and every call passed to it would fail the same way while it kept the lease.

  A server that stops as it's asked to hands its leases on: it writes the log's next entry naming nobody, once it
  answers nothing more from its copy, and whoever asks next takes it and writes at once, with nothing to wait out.

  A lease belongs to whichever server the others reach, whatever its machine, as a lease in Chubby or etcd lasts only
  while its holder answers: a holder frozen, asleep, or behind a firewall that keeps the others out, though it still
  reaches the bucket, is taken over from once a server that can't reach it finds that another can't either, or that
  there's no other to ask (`Database.transaction/3`). One that couldn't reach a holder that took the lease from it, and
  found no other server to ask, doesn't take it back while the relay at the network's address still reaches that one
  (`OneCore.Servers.answers?/1`): it's the one out of reach, as a laptop that has just woken is, and taking the lease
  back would have it move back and forth.

  Whether a server may answer from its copy isn't a state of its own. It's the time since it last saw the log's next
  entry free, read after the transaction ran by both its clocks, since a state timeout fires only between events,
  and by the monotonic clock alone, which stops while a machine sleeps.
  """
  @behaviour :gen_statem
  require Logger
  alias Database.Log
  alias OpenTelemetryAPIExperimental.{Counter, Histogram}
  require Counter
  require Histogram

  # The fewest pages logged since a snapshot before the next, so that a small database isn't kept whole on every
  # other write. An entry that only takes the lease counts as a page: a server opening the database reads every
  # entry since the snapshot, while every call waits, and servers taking the lease from each other log nothing else.
  @least 64

  # How long, in milliseconds, a server answers from its copy after it last saw the log's next entry free, and how
  # long one that has just opened the database waits before it writes to it, from when it began reading the log
  # that names it. The second is longer by far more than two machines' clocks drift apart in that time.
  @lease_ttl 4_000
  @writes_after 5_000

  # What SQLite says of a write to a copy that's read-only (`pragma query_only`), as `Database.query/3` raises it.
  @read_only "attempt to write a readonly database"

  # What it keeps besides its state. `db` is the copy while it's open, with the build of `Sqlite` that opened it,
  # `{sqlite, db}`, which is the build it's used with (`OneCore.Nif`), `number` the log's last entry it read or
  # wrote, `logged` the pages logged since the last snapshot and `pages` the copy's size, `proved` its clocks when it
  # last saw the log's next entry free, `settled` when by its monotonic clock it may write, and `firing` the
  # scheduled jobs being called here: each one's `job` and the time it was due `at`, by its call's process, and
  # whether it's `marked` as run yet; `lost_to` the server that last took the lease from this one. A field's default is what's safe for a database that was open under a version
  # of this module without it (`code_change/4`): never seen free, so it looks before it answers.
  defstruct [:artifact, :server, :folder, :db, :proved, :settled, :failed, :lost_to, number: 0, logged: 0, pages: 0, firing: %{}]

  # How long, in milliseconds, a server whose own copy of the database failed takes no lease after it handed this one
  # on, so a server that can open files takes it rather than this one again (`failed/4`).
  @failed_for 60_000

  @doc """
  The scheduled job of the database `lease` that this process is the call of, `{job, at}`, until a transaction it
  made there has marked it as run (`marked/0`), or `nil`. A job's call may write to other apps' databases too, as
  one keeping something in its person's Vault does, and those have no schedule of its.
  """
  def job(lease) do
    case Process.get(__MODULE__) do
      {^lease, job, at} -> {job, at}
      _ -> nil
    end
  end

  @doc "This process's scheduled job is marked as run."
  def marked, do: Process.delete(__MODULE__)

  @doc """
  The process for the database that is the calling app's own artifact, on the server running the call. It
  outlives the call that started it, and carries on through this app's updates with the database open.
  """
  def of(%OneCore.Call{own: artifact, server: server, folder: folder} = call) do
    OneCore.Call.process(call, __MODULE__, {server, artifact.prefix}, {artifact, server, Path.join(folder, "database")})
  end

  @impl true
  def callback_mode, do: :handle_event_function

  # `folder` is where this server keeps its copy, which is nobody else's.
  @impl true
  def init({artifact, server, folder}) do
    # So it hears that its server is stopping, and hands its lease on (`terminate/3`).
    Process.flag(:trap_exit, true)
    measures()
    {:next_state, state, data, actions} = moved(look(%__MODULE__{artifact: artifact, server: server, folder: folder}))
    {:ok, state, data, actions}
  end

  # Once settled, its copy takes writes, and the transactions that would have written while settling run again.
  @impl true
  def handle_event(:state_timeout, :settled, :settling, data) do
    {:ok, _} = sqlite(data.db, "pragma query_only = off")
    {:next_state, :holding, data}
  end

  # The server this one follows didn't run a call passed on to it, `why`: `:cannot_run` where it answered that it
  # would pass it on in turn, `:unanswered` where neither this server nor another it asked reached it, and `:alone`
  # where this one couldn't and had no other to ask. If it handed the lease on, this server takes it; if the log names
  # nobody after it, this server takes it, unless it's `:alone`, that server took it from this one, and the relay still
  # reaches it; and otherwise it has learnt who holds it now. Only what was logged after the last entry it read is read,
  # which is one request: every call passed to a holder that's gone asks this, one after another, and each waits for
  # the ones before. One that isn't up takes nothing, and says so.
  def handle_event({:call, from}, {:take_over, server, why}, {:following, server}, data) do
    entries = Log.since(data.artifact, data.number)
    data = %{data | number: data.number + length(entries)}

    case List.last(entries) do
      _free when not is_nil(server) and entries == [] ->
        cond do
          not OneCore.Server.up?() ->
            {:keep_state_and_data, {:reply, from, :not_up}}

          failed?(data) ->
            {:keep_state_and_data, {:reply, from, :failed}}

          why == :alone and data.lost_to == server and OneCore.Servers.answers?(server) ->
            {:keep_state_and_data, {:reply, from, :connected}}

          true ->
            Logger.info("#{data.artifact.prefix}: #{server}, which holds its lease, didn't run a call passed to it")
            moved(take(data, "unanswered"), [{:reply, from, :ok}])
        end

      free when free in [nil, %{server: nil}] ->
        cond do
          not OneCore.Server.up?() -> {:keep_state, data, {:reply, from, :not_up}}
          failed?(data) -> {:keep_state, data, {:reply, from, :failed}}
          true -> moved(take(data, if(free, do: "handed_on", else: "free")), [{:reply, from, :ok}])
        end

      %{server: holder} when holder != data.server ->
        Logger.info("#{data.artifact.prefix}: its lease is held by #{holder}")
        {:next_state, {:following, holder}, data, {:reply, from, :ok}}

      _this ->
        moved(look(data), [{:reply, from, :ok}])
    end
  end

  # Another holds it now, or this server does: the call is asked again, and goes there.
  def handle_event({:call, from}, {:take_over, _server, _why}, _state, _data),
    do: {:keep_state_and_data, {:reply, from, :ok}}

  # Anything else is asked with what wraps the artifact of the call that asks.
  def handle_event({:call, from}, {codec, _ask}, _state, %{artifact: %{codec: was}}) when codec != was,
    do: {:stop_and_reply, :normal, {:reply, from, :codec_changed}}

  def handle_event({:call, from}, {_codec, ask}, {:following, server}, _data)
      when elem(ask, 0) in [:transaction, :schedule],
      do: {:keep_state_and_data, {:reply, from, {:held_by, server}}}

  def handle_event({:call, _from} = type, {_codec, {:transaction, schema, fun, job}} = content, state, data) do
    transact({type, content}, state, data, fn db ->
      migrate(db, schema)
      result = fun.(db)
      if job, do: mark(db, job)
      result
    end)
  end

  def handle_event({:call, _from} = type, {_codec, {:schedule, jobs, _fire, now}} = content, state, data),
    do: transact({type, content}, state, data, &due(&1, jobs, now))

  # A job's call ended. One that committed no transaction is marked as run now, unless the lease has gone: then the
  # server holding it makes the call.
  def handle_event(:info, {:DOWN, _monitor, :process, caller, _why} = down, state, data) do
    case pop_in(data.firing[caller]) do
      {%{marked: false} = firing, data} when state in [:settling, :holding] ->
        transact({:info, down}, state, data, &mark(&1, {firing.job, firing.at}))

      {_firing, data} ->
        {:keep_state, data}
    end
  end

  def handle_event(:info, _message, _state, _data), do: :keep_state_and_data

  # A server stopping as it was asked to hands on the lease it holds: its copy is closed first, so it answers nothing
  # from it after, and then the log's next entry names nobody. One that has lost the lease meanwhile writes nothing.
  @impl true
  def terminate(why, state, data) do
    data = close(data)

    if state in [:settling, :holding] and (why == :shutdown or match?({:shutdown, _}, why)) do
      wrote = Log.write(data.artifact, data.number + 1, %{server: nil})
      Logger.info("#{data.artifact.prefix}: handing its lease on at #{data.number + 1}: #{wrote}")
    end
  end

  # A database open under an earlier version of this module carries on open under this one, in the state it was in,
  # with what this one keeps that that one didn't at its default. One opened by another build of `Sqlite` is opened
  # with this version's, and closed with that one's, which is still loaded now.
  @impl true
  def code_change(_vsn, state, data, _extra) do
    # One started by a version before it handed its lease on hears its server stopping from now on (`init/1`).
    Process.flag(:trap_exit, true)
    data = struct(__MODULE__, Map.from_struct(data))
    {:ok, state, if(match?({sqlite, _db} when sqlite != Sqlite, data.db), do: reopened(data, state), else: data)}
  end

  # Opened before it's closed, so SQLite keeps its write-ahead log: the last connection to close empties it into the
  # file. One that's settling is read-only still.
  defp reopened(%{db: {sqlite, db}} = data, state) do
    opened = connected(file(data))
    if state == :settling, do: {:ok, _} = sqlite(opened, "pragma query_only = on")
    sqlite.close(db)
    %{data | db: opened}
  end

  # Runs `work` with the database in a transaction, for `event`, which is answered once what it changed is in the
  # bucket (`done/3`). One that changed nothing, raised or not, is answered from the copy, while the copy is still the
  # database. While settling the copy is read-only, so one that raised for writing, as SQLite says (`SQLITE_READONLY`),
  # waits, with nothing it did kept, to run again once settled, which is the state it changes to next; one that raised
  # for anything else is answered with that now. The rest of what `data` changed for it is left as it was, for then.
  defp transact(event, state, data, work) do
    transacted(event, state, data, work)
  rescue
    unreachable in OneCore.Edge.Unreachable -> reraise unreachable, __STACKTRACE__
    error -> failed(event, data, error, __STACKTRACE__)
  end

  defp transacted(event, state, data, work) do
    ran = run(data.db, work)

    case {Log.changed(file(data)), state, read_only?(ran)} do
      {nil, :settling, true} ->
        {:keep_state_and_data, :postpone}

      {nil, _state, _read_only} ->
        case proved(data) do
          {:ok, data} -> answered(event, ran, data)
          :lost -> lost(event, data, "so it didn't answer from its copy")
        end

      {changed, :holding, _read_only} ->
        case write(data, changed) do
          {:ok, data} -> answered(event, ran, data)
          :lost -> lost(event, data, "and the write it hadn't answered with it")
        end
    end
  end

  defp read_only?({:raised, :error, error, _stacktrace}) when is_exception(error), do: Exception.message(error) =~ @read_only
  defp read_only?(_ran), do: false

  # This server's own work on the database failed, not the app's (`run/2` answers that): its lease is handed on, and
  # a call is answered with what failed, to be asked of whoever takes the lease. Not reaching the bucket isn't this:
  # nothing could be handed on then, and nobody else would reach it either.
  defp failed({{:call, from}, _content}, data, error, stacktrace),
    do: {:next_state, {:following, nil}, hand_on_failed(data, error), {:reply, from, {:raised, :error, error, stacktrace}}}

  defp failed({:info, _message}, data, error, _stacktrace),
    do: {:next_state, {:following, nil}, hand_on_failed(data, error)}

  # Its copy is thrown away, which may fail too where files can't be opened, and is made anew where it's opened next:
  # then the log's next entry names nobody, as a server stopping writes it.
  defp hand_on_failed(data, error) do
    data =
      try do
        close(data)
      rescue
        _closing -> %{data | db: nil}
      end

    wrote = Log.write(data.artifact, data.number + 1, %{server: nil})
    Logger.error("#{data.artifact.prefix}: this server's copy failed (#{Exception.message(error)}), so it hands its lease on at #{data.number + 1}: #{wrote}")
    %{data | failed: System.monotonic_time(:millisecond)}
  end

  defp failed?(%{failed: nil}), do: false
  defp failed?(%{failed: at}), do: System.monotonic_time(:millisecond) - at < @failed_for

  defp answered(event, ran, data) do
    {data, actions} = done(event, ran, data)
    {:keep_state, data, actions}
  end

  # What a transaction gave, once that's safe to say, and what follows from it: a call that asked is answered, the
  # job whose call made it is marked as run, and the jobs a schedule found due are called, each in a process of its
  # own. A call that has marked its job as run is done with it, even while its process is still ending. A database
  # is kept as a snapshot once its log is long enough.
  defp done({{:call, {caller, _tag} = from}, {_codec, {:transaction, _schema, _fun, job}}}, ran, data) do
    data =
      if job && data.firing[caller] && match?({:ok, _}, ran), do: put_in(data.firing[caller].marked, true), else: data

    {snapshot(data, ran), [{:reply, from, ran}]}
  end

  defp done({{:call, from}, {_codec, {:schedule, _jobs, fire, _now}}}, {:ok, due} = ran, data) do
    calling = for {_caller, %{job: job, marked: false}} <- data.firing, do: job

    lease = self()

    firing =
      for {job, at, called} <- due, job not in calling, into: data.firing do
        {caller, _monitor} =
          spawn_monitor(fn ->
            Process.put(__MODULE__, {lease, job, at})
            fire.(called)
          end)

        {caller, %{job: job, at: at, marked: false}}
      end

    {snapshot(%{data | firing: firing}, ran), [{:reply, from, :ok}]}
  end

  defp done({{:call, from}, {_codec, {:schedule, _jobs, _fire, _now}}}, raised, data),
    do: {data, [{:reply, from, raised}]}

  defp done({:info, {:DOWN, _monitor, :process, _caller, _why}}, ran, data), do: {snapshot(data, ran), []}

  # Another server wrote the log's next entry: the lease is theirs, and what this one committed to its copy since
  # is thrown away with it. `event` is handled again where it is now, which for a call is with who holds it.
  defp lost({type, content}, data, what) do
    Logger.info("#{data.artifact.prefix}: lost its lease, #{what}")

    case look(close(data)) do
      {{:following, holder}, data} -> moved({{:following, holder}, %{data | lost_to: holder}}, [{:next_event, type, content}])
      looked -> moved(looked, [{:next_event, type, content}])
    end
  end

  # Where it is once it has read the log: settling waits out its time before it writes.
  defp moved(looked, actions \\ [])
  defp moved({:settling, data}, actions), do: {:next_state, :settling, data, [settle(data) | actions]}
  defp moved({state, data}, actions), do: {:next_state, state, data, actions}

  defp settle(data), do: {:state_timeout, data.settled, :settled, abs: true}

  # Runs `work` with the database in a transaction: `{:ok, result}` once it's committed to the copy, or
  # `{:raised, kind, reason, stacktrace}` if it raised, with nothing kept.
  defp run(db, work) do
    {:ok, _} = sqlite(db, "begin")

    try do
      result = work.(db)
      with {:error, why} <- sqlite(db, "commit"), do: raise(why)
      {:ok, result}
    catch
      kind, reason ->
        raised = {:raised, kind, reason, __STACKTRACE__}
        {:ok, _} = sqlite(db, "rollback")
        checkpoint(db)
        raised
    end
  end

  # The jobs and timers that are due at `now`: each one's name here, the time it was due at, and the job. A job is due
  # at each whole multiple of its `every` seconds, and the newest of those that passed is the one that's made up
  # for. One that's new is first due at the next. A timer is due at its `at` (`Database.timer/5`).
  defp due(db, jobs, now), do: jobs_due(db, jobs, now) ++ timers_due(db, now)

  defp timers_due(db, now) do
    Database.query(db, Database.timers())

    for %{"job" => job, "tool" => tool, "inputs" => inputs, "at" => at} <-
          Database.query(db, "select job, tool, inputs, at from database_timers where at <= ?", [now]),
        do: {"timer " <> job, at, %{tool: tool, inputs: JSON.decode!(inputs)}}
  end

  defp jobs_due(db, jobs, now) do
    Database.query(db, """
    create table if not exists database_scheduled (job text primary key, fired integer not null) without rowid
    """)

    fired = Map.new(Database.query(db, "select job, fired from database_scheduled"), &{&1["job"], &1["fired"]})

    Enum.flat_map(jobs, fn called ->
      job = "#{called.tool} #{JSON.encode!(called.inputs)}"
      at = div(now, called.every) * called.every

      case fired[job] do
        nil ->
          Database.query(db, "insert into database_scheduled (job, fired) values (?, ?)", [job, at])
          []

        fired when fired < at ->
          [{job, at, called}]

        _fired ->
          []
      end
    end)
  end

  # Marks a job as run for the time it was due at. If it's marked already, a server that held the lease
  # meanwhile made the call, and this one's writes aren't kept.
  defp mark(db, {"timer " <> job, at}) do
    # One that repeats is next due at the first of its times after now, and one that doesn't is done.
    repeated = """
    update database_timers set at = at + every * ((cast(strftime('%s', 'now') as integer) - at) / every + 1)
    where job = ? and at = ? and every is not null returning job
    """

    if Database.query(db, repeated, [job, at]) == [] and
         Database.query(db, "delete from database_timers where job = ? and at = ? and every is null returning job", [job, at]) == [],
       do: raise("Another server made this timer's call")
  end

  defp mark(db, {job, at}) do
    marked = "update database_scheduled set fired = ? where job = ? and fired < ? returning job"
    if Database.query(db, marked, [at, job, at]) == [], do: raise("Another server made this scheduled call")
  end

  # Reads the log: following the server it names, or settling with the database opened if it names this one. The
  # entry after the last it read was free when it read it, which is after it began.
  defp look(data) do
    began = clocks()
    snapshot = Log.snapshot(data.artifact)
    entries = Log.since(data.artifact, snapshot)
    data = %{data | number: snapshot + length(entries)}

    case List.last(entries) || Log.entry(data.artifact, snapshot) do
      free when free in [nil, %{server: nil}] ->
        if OneCore.Server.up?() and not failed?(data),
          do: take(data, if(free, do: "handed_on", else: "free")),
          else: {{:following, nil}, data}

      %{server: server} when server == data.server ->
        Logger.info("#{data.artifact.prefix}: holds its lease, from snapshot #{snapshot} and the log to #{data.number}")
        open(%{data | proved: began}, snapshot, entries)

      %{server: server} ->
        Logger.info("#{data.artifact.prefix}: its lease is held by #{server}")
        {{:following, server}, data}
    end
  end

  # Writing the log's next entry is taking the lease. If another server wrote it first, the lease is theirs. A lease
  # this server took is said as a number of its app's, with why: `free`, as nobody held it, `handed_on`, as its holder
  # stopped and said so, or `unanswered`, as its holder didn't run a call passed to it (Telemetry's
  # `one.database.lease.moves`).
  defp take(data, why) do
    wrote = Log.write(data.artifact, data.number + 1, %{server: data.server})
    Logger.info("#{data.artifact.prefix}: taking its lease at #{data.number + 1}: #{wrote}")
    if wrote == :ok, do: Counter.add(:"one.database.lease.moves", 1, %{"one.app" => app(data.artifact), "one.reason" => why})
    look(data)
  end

  @doc """
  Makes what a lease is measured by, OpenTelemetry's instruments, where they aren't yet: how long a call waited on
  another server holding it (`Database.transaction/3`), and how often it moved here.
  """
  def measures do
    meter = :opentelemetry_experimental.get_meter(:opentelemetry.get_application_scope(__MODULE__))

    if :otel_meter.lookup_instrument(meter, :"one.database.lease.moves") == :undefined do
      Counter.create(:"one.database.lease.moves", %{description: "A database's lease that moved to this server, and why"})

      Histogram.create(:"one.database.lease.wait", %{
        unit: :ms,
        description: "How long a call waited on the server holding its database's lease before this one took it",
        advisory_params: %{explicit_bucket_boundaries: OneCore.Otlp.buckets()}
      })
    end
  end

  @doc "Says how long a call waited on another server holding its database's lease, in milliseconds."
  def waited(ms, attributes), do: Histogram.record(:"one.database.lease.wait", ms, attributes)

  @doc "The app whose database `artifact` is, by the name it's installed by: `permissions` of `network/own/permissions/`."
  def app(artifact) do
    case Regex.run(~r{(?:^|/)(?:own|apps)/([^/]+)}, artifact.prefix) do
      [_, app] -> app
      nil -> artifact.prefix
    end
  end

  # The copy is made anew each time: one left by an earlier run may hold what was never in the bucket. One this server
  # can't make, as one out of files can't, has its lease handed on (`failed/4`).
  defp open(data, snapshot, entries) do
    File.rm_rf!(data.folder)
    File.mkdir_p!(data.folder)

    case Log.restore(data.artifact, snapshot, entries, file(data)) do
      :ok -> {:settling, opened(data, snapshot, entries)}
      :gone -> look(data)
    end
  rescue
    unreachable in OneCore.Edge.Unreachable -> reraise unreachable, __STACKTRACE__
    error -> {{:following, nil}, hand_on_failed(data, error)}
  end

  # It writes nothing for `@writes_after` from when it began reading the log (`proved`), by when one that held the
  # lease before and doesn't know it lost it has stopped answering from its copy (`proved/1`): that one last saw the
  # entry naming this server free before it was written, which was before this began. Until then its copy is
  # read-only. The monotonic clock stops while the machine sleeps, which only makes it wait longer. A new database,
  # whose log is only this server taking its lease, has nothing anyone could answer from, so it writes at once, as
  # does one whose last holder handed it on, which it did once it answered nothing more.
  defp opened(%{proved: {began, _system}} = data, snapshot, entries) do
    logged = Enum.sum_by(entries, &max(length(Map.get(&1, :pages, [])), 1))

    wait =
      case Enum.take(entries, -2) do
        [%{server: nil}, %{server: server}] when server == data.server -> 0
        [%{server: server}] when server == data.server and snapshot == 0 -> 0
        _held -> @writes_after
      end

    db = connected(file(data))
    {:ok, _} = sqlite(db, "pragma query_only = on")
    %{data | db: db, logged: logged, settled: began + wait}
  end

  # The bucket is what keeps a write, so SQLite needn't wait for the disk. Its write-ahead log is emptied here
  # after each transaction, never by SQLite itself.
  defp connected(file) do
    {:ok, db} = Sqlite.open(file)
    db = {Sqlite, db}

    for pragma <- ["journal_mode = wal", "synchronous = off", "wal_autocheckpoint = 0"],
        do: {:ok, _} = sqlite(db, "pragma " <> pragma)

    db
  end

  defp sqlite({sqlite, db}, sql), do: sqlite.query(db, sql, [])

  # Writes what the transaction changed, which SQLite's write-ahead log holds, as the log's next entry: `{:ok, data}`,
  # or `:lost` if another server wrote that entry first.
  defp write(data, {size, pages}) do
    began = clocks()

    case Log.write(data.artifact, data.number + 1, %{server: data.server, size: size, pages: pages}) do
      :ok ->
        checkpoint(data.db)
        {:ok, %{data | number: data.number + 1, logged: data.logged + length(pages), pages: size, proved: began}}

      :exists ->
        :lost
    end
  end

  # A server that couldn't reach this one may have taken the lease by writing the log's next entry, and this one
  # only finds out by looking. So it answers from its copy for `@lease_ttl` after it last saw that entry free, by
  # both its clocks, the monotonic one, which never goes back, and the system's, which goes on while the machine
  # sleeps; and then looks again. A server that took the lease meanwhile writes nothing until that's passed.
  defp proved(data) do
    now = clocks()

    if proved?(data.proved, now) do
      {:ok, data}
    else
      case Log.entry(data.artifact, data.number + 1) do
        nil -> {:ok, %{data | proved: now}}
        _taken -> :lost
      end
    end
  end

  defp proved?({monotonic, system}, {now_monotonic, now_system}),
    do: now_monotonic < monotonic + @lease_ttl and now_system >= system and now_system < system + @lease_ttl

  defp proved?(nil, _now), do: false

  defp clocks, do: {System.monotonic_time(:millisecond), System.os_time(:millisecond)}

  # A snapshot is kept once the log since the last one holds as many pages as the database, and at least `@least`,
  # after a transaction that didn't raise. The next transaction waits for it, so the write that brought it on may as
  # well.
  defp snapshot(%{logged: logged, pages: pages} = data, {:ok, _result}) when logged >= pages and logged >= @least do
    Log.keep(data.artifact, data.number, file(data))
    Logger.info("#{data.artifact.prefix}: kept snapshot #{data.number}")
    %{data | logged: 0}
  end

  defp snapshot(data, _ran), do: data

  # Moves the write-ahead log's pages into the database file, and empties it for the next transaction.
  defp checkpoint(db) do
    {:ok, [%{"busy" => 0}]} = sqlite(db, "pragma wal_checkpoint(truncate)")
  end

  defp close(%{db: nil} = data), do: data

  defp close(%{db: {sqlite, db}} = data) do
    sqlite.close(db)
    File.rm_rf!(data.folder)
    %{data | db: nil}
  end

  defp file(data), do: Path.join(data.folder, "copy.sqlite")

  # The schema's statements this database hasn't run, which SQLite's `user_version` counts.
  defp migrate(db, schema) do
    [%{"user_version" => ran}] = Database.query(db, "pragma user_version")

    if length(schema) > ran do
      for statement <- Enum.drop(schema, ran), do: Database.query(db, statement)
      Database.query(db, "pragma user_version = #{length(schema)}")
    end
  end
end
