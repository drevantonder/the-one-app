defmodule Database.Lease do
  @moduledoc """
  One database on this server: open here while this server holds its lease, and otherwise only known to be held by
  another.

  The server holding the lease keeps the database as a file of its own, which is only a copy: the log in the
  bucket is the database. A transaction commits to that file, and its pages are then written as the log's next
  entry. Only once that's written is the transaction answered, so nothing answered is lost with this server. If
  another server wrote that entry first, this one lost the lease without knowing: it throws its copy away, and
  what it had committed there with it.

  One transaction runs at a time, and the next begins after the last is in the bucket, so none sees what could
  still be lost.

  The server holding the lease also makes the calls of the plugin's scheduled jobs. When each job last ran is a
  row of the database, in `database_scheduled`: the time it was due at. A job that's due is called in a process
  of its own, and the first transaction that process commits marks it as run, in that same transaction. So the
  mark and what the call wrote reach the log in one entry, or neither does, and a server that takes the lease
  reads from the log whether the call is still to make. A call that made no transaction is marked when it ends.
  """
  use GenServer
  require Logger
  alias Database.Log

  # The fewest pages logged since a snapshot before the next, so that a small database isn't kept whole on every
  # other write.
  @least 64

  @doc """
  The process for the database that is the calling plugin's own artifact, on the server running the call. It
  outlives the call that started it, and carries on through this plugin's updates with the database open.
  """
  def of(%OneCore.Call{own: artifact, server: server, folder: folder} = call) do
    OneCore.Call.process(call, __MODULE__, {server, artifact.prefix}, {artifact, server, Path.join(folder, "database")})
  end

  # `folder` is where this server keeps its copy, which is nobody else's.
  @impl true
  def init({artifact, server, folder}) do
    lease = %{artifact: artifact, server: server, folder: folder, db: nil, number: 0, held_by: nil, logged: 0, pages: 0}
    # The scheduled jobs being called here: each one's `job` and the time it was due `at`, by its call's process,
    # and whether it's `marked` as run yet.
    {:ok, look(Map.put(lease, :firing, %{}))}
  end

  @impl true
  def handle_call({:transaction, _schema, _fun}, _from, %{db: nil} = lease) do
    {:reply, {:held_by, lease.held_by}, lease}
  end

  def handle_call({:transaction, schema, fun} = transaction, {caller, _} = from, lease) do
    firing = lease.firing[caller]

    ran =
      transact(lease, fn db ->
        migrate(db, schema)
        result = fun.(db)
        if firing && not firing.marked, do: mark(db, firing)
        result
      end)

    case ran do
      {:ok, result, lease} when firing != nil -> {:reply, {:ok, result}, put_in(lease.firing[caller].marked, true)}
      {:ok, result, lease} -> {:reply, {:ok, result}, lease}
      # Here again if this server holds the lease after all, and otherwise answered with who does.
      {:lost, lease} -> handle_call(transaction, from, lease)
      {raised, lease} -> {:reply, raised, lease}
    end
  end

  def handle_call({:schedule, _jobs, _fire, _now}, _from, %{db: nil} = lease) do
    {:reply, {:held_by, lease.held_by}, lease}
  end

  # Each job that's due at `now`, and isn't being called already, is called in a process of its own.
  def handle_call({:schedule, jobs, fire, now} = schedule, from, lease) do
    case transact(lease, &due(&1, jobs, now)) do
      {:ok, due, lease} ->
        calling = for {_caller, %{job: job}} <- lease.firing, do: job

        firing =
          for {job, at, called} <- due, job not in calling, into: lease.firing do
            {caller, _monitor} = spawn_monitor(fn -> fire.(called) end)
            {caller, %{job: job, at: at, marked: false}}
          end

        {:reply, :ok, %{lease | firing: firing}}

      {:lost, lease} ->
        handle_call(schedule, from, lease)
    end
  end

  # The server this one took to hold the lease didn't run a call passed on to it. If the log still names it, this
  # server takes the lease, and otherwise it has learnt who holds it now.
  def handle_call({:take_over, server}, _from, %{db: nil} = lease) do
    case look(lease) do
      %{db: nil, held_by: ^server} = lease -> {:reply, :ok, take(lease)}
      lease -> {:reply, :ok, lease}
    end
  end

  def handle_call({:take_over, _server}, _from, lease), do: {:reply, :ok, lease}

  # A job's call ended. One that committed no transaction is marked as run now, unless the lease has gone:
  # then the server holding it makes the call.
  @impl true
  def handle_info({:DOWN, _monitor, :process, caller, _why}, lease) do
    {firing, lease} = pop_in(lease.firing[caller])

    if firing && not firing.marked && lease.db do
      case transact(lease, &mark(&1, firing)) do
        {:ok, _marked, lease} -> {:noreply, lease}
        {_lost_or_marked_already, lease} -> {:noreply, lease}
      end
    else
      {:noreply, lease}
    end
  end

  def handle_info(_message, lease), do: {:noreply, lease}

  @impl true
  def terminate(_reason, lease), do: close(lease)

  # Runs `fun` with the database in a transaction, and writes what it changed as the log's next entry. Gives
  # `{:ok, result, lease}` once that's written, `{:lost, lease}` if another server wrote that entry first, so
  # the lease has gone and what `fun` wrote with it, or `{{:raised, kind, reason, stacktrace}, lease}` if `fun`
  # raised, with nothing written.
  defp transact(%{db: db} = lease, fun) do
    {:ok, _} = Sqlite.query(db, "begin", [])

    ran =
      try do
        result = fun.(db)
        with {:error, why} <- Sqlite.query(db, "commit", []), do: raise(why)
        {:ok, result}
      catch
        kind, reason -> {:raised, kind, reason, __STACKTRACE__}
      end

    case ran do
      {:ok, result} ->
        case log(lease) do
          {:ok, lease} -> {:ok, result, snapshot(lease)}
          {:lost, lease} -> {:lost, lease}
        end

      raised ->
        {:ok, _} = Sqlite.query(db, "rollback", [])
        checkpoint(db)
        {raised, lease}
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

  # Reads the log, and opens the database if this server holds its lease.
  defp look(lease) do
    snapshot = Log.snapshot(lease.artifact)
    entries = Log.since(lease.artifact, snapshot)
    lease = %{lease | number: snapshot + length(entries)}

    case List.last(entries) || Log.entry(lease.artifact, snapshot) do
      nil ->
        take(lease)

      %{server: server} when server == lease.server ->
        Logger.info(
          "#{lease.artifact.prefix}: holds its lease, from snapshot #{snapshot} and the log to #{lease.number}"
        )

        open(lease, snapshot, entries)

      %{server: server} ->
        Logger.info("#{lease.artifact.prefix}: its lease is held by #{server}")
        %{lease | held_by: server}
    end
  end

  # Writing the log's next entry is taking the lease. If another server wrote it first, the lease is theirs.
  defp take(lease) do
    wrote = Log.write(lease.artifact, lease.number + 1, %{server: lease.server})
    Logger.info("#{lease.artifact.prefix}: taking its lease at #{lease.number + 1}: #{wrote}")
    look(lease)
  end

  # The copy is made anew each time: one left by an earlier run may hold what was never in the bucket.
  defp open(lease, snapshot, entries) do
    File.rm_rf!(lease.folder)
    File.mkdir_p!(lease.folder)

    case Log.restore(lease.artifact, snapshot, entries, file(lease)) do
      :ok -> opened(lease, entries)
      :gone -> look(lease)
    end
  end

  defp opened(lease, entries) do
    {:ok, db} = Sqlite.open(file(lease))

    # The bucket is what keeps a write, so SQLite needn't wait for the disk. Its write-ahead log is emptied here
    # after each transaction, never by SQLite itself.
    for pragma <- ["journal_mode = wal", "synchronous = off", "wal_autocheckpoint = 0"] do
      {:ok, _} = Sqlite.query(db, "pragma " <> pragma, [])
    end

    logged = Enum.sum_by(entries, &length(Map.get(&1, :pages, [])))
    %{lease | db: db, held_by: lease.server, logged: logged}
  end

  # Writes what the transaction changed as the log's next entry.
  defp log(lease) do
    case Log.changed(file(lease)) do
      nil ->
        {:ok, lease}

      {size, pages} ->
        case Log.write(lease.artifact, lease.number + 1, %{server: lease.server, size: size, pages: pages}) do
          :ok ->
            checkpoint(lease.db)
            {:ok, %{lease | number: lease.number + 1, logged: lease.logged + length(pages), pages: size}}

          :exists ->
            Logger.info("#{lease.artifact.prefix}: lost its lease, and the write it hadn't answered with it")
            {:lost, look(close(lease))}
        end
    end
  end

  # A snapshot is kept once the log since the last one holds as many pages as the database, and at least `@least`.
  # The next transaction waits for it, so the write that brought it on may as well.
  defp snapshot(%{logged: logged, pages: pages} = lease) when logged >= pages and logged >= @least do
    Log.keep(lease.artifact, lease.number, file(lease))
    Logger.info("#{lease.artifact.prefix}: kept snapshot #{lease.number}")
    %{lease | logged: 0}
  end

  defp snapshot(lease), do: lease

  # Moves the write-ahead log's pages into the database file, and empties it for the next transaction.
  defp checkpoint(db) do
    {:ok, [%{"busy" => 0}]} = Sqlite.query(db, "pragma wal_checkpoint(truncate)", [])
  end

  defp close(%{db: nil} = lease), do: lease

  defp close(lease) do
    Sqlite.close(lease.db)
    File.rm_rf!(lease.folder)
    %{lease | db: nil}
  end

  defp file(lease), do: Path.join(lease.folder, "copy.sqlite")

  # The schema's statements this database hasn't run, which SQLite's `user_version` counts.
  defp migrate(db, schema) do
    [%{"user_version" => ran}] = Database.query(db, "pragma user_version")

    if length(schema) > ran do
      for statement <- Enum.drop(schema, ran), do: Database.query(db, statement)
      Database.query(db, "pragma user_version = #{length(schema)}")
    end
  end
end
