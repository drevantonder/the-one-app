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

  The server holding the lease also makes the calls of the plugin's scheduled jobs. When each job last ran is a
  row of the database, in `database_scheduled`: the time it was due at. A job that's due is called in a process
  of its own, and the first transaction that process commits marks it as run, in that same transaction. So the
  mark and what the call wrote reach the log in one entry, or neither does, and a server that takes the lease
  reads from the log whether the call is still to make. A call that made no transaction is marked when it ends.

  A call says what wraps the artifact it has (`OneCore.Artifact`). If that isn't what this was opened with, the
  database closes, and the call opens it again: an encrypted artifact's keys are given anew to a server that was
  taken off it and chosen again, and what it had open is from before another server wrote.

  It's a `:gen_statem`, in one of these states:

  - `{:following, server}`: `server` holds the lease. A transaction is answered with who holds it, to be passed on
    to them, and one they didn't run has this server read the log again and take the lease if it still names them.
  - `:settling`: this server has just opened the database. It answers from its copy, and writes nothing until
    `@writes_after` has passed.
  - `{:settling, written}`: a transaction wrote while settling. What it wrote is committed to the copy, and is
    logged and answered once that time has passed. Everything else waits behind it.
  - `:holding`: each transaction's write is logged as it commits.

  It moves between them by reading the log (`look/1`): to `:settling` with the database opened if the log names
  this server, after taking the lease if the log is empty, and otherwise to following the server it names. One that
  finds another server wrote the log's next entry has lost the lease, and reads the log again.

  Whether a server may answer from its copy isn't a state of its own. It's the time since it last saw the log's next
  entry free, read after the transaction ran by both its clocks, since a state timeout fires only between events,
  and by the monotonic clock alone, which stops while a machine sleeps.
  """
  @behaviour :gen_statem
  require Logger
  alias Database.Log

  # The fewest pages logged since a snapshot before the next, so that a small database isn't kept whole on every
  # other write.
  @least 64

  # How long, in milliseconds, a server answers from its copy after it last saw the log's next entry free, and how
  # long one that has just opened the database waits before it writes to it. The second is longer by far more than
  # two machines' clocks drift apart in that time.
  @lease_ttl 4_000
  @writes_after 5_000

  # What it keeps besides its state. `db` is the copy while it's open, `number` the log's last entry it read or
  # wrote, `logged` the pages logged since the last snapshot and `pages` the copy's size, `proved` its clocks when it
  # last saw the log's next entry free, `settled` when by its monotonic clock it may write, and `firing` the
  # scheduled jobs being called here: each one's `job` and the time it was due `at`, by its call's process, and
  # whether it's `marked` as run yet. A field's default is what's safe for a database that was open under a version
  # of this module without it (`code_change/4`): never seen free, so it looks before it answers.
  defstruct [:artifact, :server, :folder, :db, :proved, :settled, number: 0, logged: 0, pages: 0, firing: %{}]

  @doc """
  The process for the database that is the calling plugin's own artifact, on the server running the call. It
  outlives the call that started it, and carries on through this plugin's updates with the database open.
  """
  def of(%OneCore.Call{own: artifact, server: server, folder: folder} = call) do
    OneCore.Call.process(call, __MODULE__, {server, artifact.prefix}, {artifact, server, Path.join(folder, "database")})
  end

  @impl true
  def callback_mode, do: :handle_event_function

  # `folder` is where this server keeps its copy, which is nobody else's.
  @impl true
  def init({artifact, server, folder}) do
    {:next_state, state, data, actions} = moved(look(%__MODULE__{artifact: artifact, server: server, folder: folder}))
    {:ok, state, data, actions}
  end

  # A write made while settling is logged once the settle time has passed, and everything else waits behind it.
  @impl true
  def handle_event(:state_timeout, :settled, {:settling, {event, ran}}, data) do
    case write(data, Log.changed(file(data))) do
      {:ok, data} ->
        {data, actions} = done(event, ran, data)
        {:next_state, :holding, data, actions}

      :lost ->
        lost(event, data, "and the write it hadn't answered with it")
    end
  end

  def handle_event(_type, _content, {:settling, _written}, _data), do: {:keep_state_and_data, :postpone}

  def handle_event(:state_timeout, :settled, :settling, data), do: {:next_state, :holding, data}

  # The server this one took to hold the lease didn't run a call passed on to it. If the log still names it, this
  # server takes the lease, and otherwise it has learnt who holds it now.
  def handle_event({:call, from}, {:take_over, server}, {:following, _server}, data) do
    case look(data) do
      {{:following, ^server}, data} ->
        Logger.info("#{data.artifact.prefix}: #{server}, which holds its lease, didn't run a call passed to it")
        moved(take(data), [{:reply, from, :ok}])

      looked ->
        moved(looked, [{:reply, from, :ok}])
    end
  end

  def handle_event({:call, from}, {:take_over, _server}, _holding, _data),
    do: {:keep_state_and_data, {:reply, from, :ok}}

  # Anything else is asked with what wraps the artifact of the call that asks.
  def handle_event({:call, from}, {codec, _ask}, _state, %{artifact: %{codec: was}}) when codec != was,
    do: {:stop_and_reply, :normal, {:reply, from, :codec_changed}}

  def handle_event({:call, from}, {_codec, ask}, {:following, server}, _data)
      when elem(ask, 0) in [:transaction, :schedule],
      do: {:keep_state_and_data, {:reply, from, {:held_by, server}}}

  def handle_event({:call, {caller, _tag}} = type, {_codec, {:transaction, schema, fun}} = content, state, data) do
    firing = data.firing[caller]

    transact({type, content}, state, data, fn db ->
      migrate(db, schema)
      result = fun.(db)
      if firing && not firing.marked, do: mark(db, firing)
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
        transact({:info, down}, state, data, &mark(&1, firing))

      {_firing, data} ->
        {:keep_state, data}
    end
  end

  def handle_event(:info, _message, _state, _data), do: :keep_state_and_data

  @impl true
  def terminate(_why, _state, data), do: close(data)

  # A database open under an earlier version of this module carries on open under this one, in the state it was in,
  # with what this one keeps that that one didn't at its default.
  @impl true
  def code_change(_vsn, state, data, _extra), do: {:ok, state, struct(__MODULE__, Map.from_struct(data))}

  # Runs `work` with the database in a transaction, for `event`, which is answered once what it changed is in the
  # bucket (`done/3`): at once while holding, and once the settle time has passed while settling. One that changed
  # nothing, raised or not, is answered from the copy, while the copy is still the database.
  defp transact(event, state, data, work) do
    ran = run(data.db, work)

    case {Log.changed(file(data)), state} do
      {nil, _state} ->
        case proved(data) do
          {:ok, data} -> answered(event, ran, data)
          :lost -> lost(event, data, "so it didn't answer from its copy")
        end

      {_changed, :settling} ->
        {:next_state, {:settling, {event, ran}}, data, settle(data)}

      {changed, :holding} ->
        case write(data, changed) do
          {:ok, data} -> answered(event, ran, data)
          :lost -> lost(event, data, "and the write it hadn't answered with it")
        end
    end
  end

  defp answered(event, ran, data) do
    {data, actions} = done(event, ran, data)
    {:keep_state, data, actions}
  end

  # What a transaction gave, once that's safe to say, and what follows from it: a call that asked is answered, the
  # job whose call made it is marked as run, and the jobs a schedule found due are called, each in a process of its
  # own. A call that has marked its job as run is done with it, even while its process is still ending. A database
  # is kept as a snapshot once its log is long enough.
  defp done({{:call, {caller, _tag} = from}, {_codec, {:transaction, _schema, _fun}}}, ran, data) do
    data = if data.firing[caller] && match?({:ok, _}, ran), do: put_in(data.firing[caller].marked, true), else: data
    {snapshot(data, ran), [{:reply, from, ran}]}
  end

  defp done({{:call, from}, {_codec, {:schedule, _jobs, fire, _now}}}, {:ok, due} = ran, data) do
    calling = for {_caller, %{job: job, marked: false}} <- data.firing, do: job

    firing =
      for {job, at, called} <- due, job not in calling, into: data.firing do
        {caller, _monitor} = spawn_monitor(fn -> fire.(called) end)
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
    moved(look(close(data)), [{:next_event, type, content}])
  end

  # Where it is once it has read the log: settling waits out its time before it writes.
  defp moved(looked, actions \\ [])
  defp moved({:settling, data}, actions), do: {:next_state, :settling, data, [settle(data) | actions]}
  defp moved({state, data}, actions), do: {:next_state, state, data, actions}

  defp settle(data), do: {:state_timeout, data.settled, :settled, abs: true}

  # Runs `work` with the database in a transaction: `{:ok, result}` once it's committed to the copy, or
  # `{:raised, kind, reason, stacktrace}` if it raised, with nothing kept.
  defp run(db, work) do
    {:ok, _} = Sqlite.query(db, "begin", [])

    try do
      result = work.(db)
      with {:error, why} <- Sqlite.query(db, "commit", []), do: raise(why)
      {:ok, result}
    catch
      kind, reason ->
        raised = {:raised, kind, reason, __STACKTRACE__}
        {:ok, _} = Sqlite.query(db, "rollback", [])
        checkpoint(db)
        raised
    end
  end

  # The jobs that are due at `now`: each one's name here, the time it was due at, and the job. A job is due at
  # each whole multiple of its `every` seconds, and the newest of those that passed is the one that's made up
  # for. One that's new is first due at the next.
  defp due(db, jobs, now) do
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
  defp mark(db, %{job: job, at: at}) do
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
      nil ->
        take(data)

      %{server: server} when server == data.server ->
        Logger.info("#{data.artifact.prefix}: holds its lease, from snapshot #{snapshot} and the log to #{data.number}")
        open(%{data | proved: began}, snapshot, entries)

      %{server: server} ->
        Logger.info("#{data.artifact.prefix}: its lease is held by #{server}")
        {{:following, server}, data}
    end
  end

  # Writing the log's next entry is taking the lease. If another server wrote it first, the lease is theirs.
  defp take(data) do
    wrote = Log.write(data.artifact, data.number + 1, %{server: data.server})
    Logger.info("#{data.artifact.prefix}: taking its lease at #{data.number + 1}: #{wrote}")
    look(data)
  end

  # The copy is made anew each time: one left by an earlier run may hold what was never in the bucket.
  defp open(data, snapshot, entries) do
    File.rm_rf!(data.folder)
    File.mkdir_p!(data.folder)

    case Log.restore(data.artifact, snapshot, entries, file(data)) do
      :ok -> {:settling, opened(data, entries)}
      :gone -> look(data)
    end
  end

  # It writes nothing for `@writes_after`, by when one that held the lease before and doesn't know it lost it has
  # stopped answering from its copy (`proved/1`). The monotonic clock stops while the machine sleeps, which only
  # makes it wait longer.
  defp opened(data, entries) do
    {:ok, db} = Sqlite.open(file(data))

    # The bucket is what keeps a write, so SQLite needn't wait for the disk. Its write-ahead log is emptied here
    # after each transaction, never by SQLite itself.
    for pragma <- ["journal_mode = wal", "synchronous = off", "wal_autocheckpoint = 0"] do
      {:ok, _} = Sqlite.query(db, "pragma " <> pragma, [])
    end

    logged = Enum.sum_by(entries, &length(Map.get(&1, :pages, [])))
    %{data | db: db, logged: logged, settled: System.monotonic_time(:millisecond) + @writes_after}
  end

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
    {:ok, [%{"busy" => 0}]} = Sqlite.query(db, "pragma wal_checkpoint(truncate)", [])
  end

  defp close(%{db: nil} = data), do: data

  defp close(data) do
    Sqlite.close(data.db)
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
