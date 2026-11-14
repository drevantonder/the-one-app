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
    {:ok, look(lease)}
  end

  @impl true
  def handle_call({:transaction, _schema, _fun}, _from, %{db: nil} = lease) do
    {:reply, {:held_by, lease.held_by}, lease}
  end

  def handle_call({:transaction, schema, fun} = transaction, from, %{db: db} = lease) do
    {:ok, _} = Sqlite.query(db, "begin", [])

    ran =
      try do
        migrate(db, schema)
        result = fun.(db)
        with {:error, why} <- Sqlite.query(db, "commit", []), do: raise(why)
        {:ok, result}
      catch
        kind, reason -> {:raised, kind, reason, __STACKTRACE__}
      end

    case ran do
      {:ok, _result} ->
        case log(lease) do
          {:ok, lease} -> {:reply, ran, snapshot(lease)}
          # Here again if this server holds the lease after all, and otherwise answered with who does.
          {:lost, lease} -> handle_call(transaction, from, lease)
        end

      raised ->
        {:ok, _} = Sqlite.query(db, "rollback", [])
        checkpoint(db)
        {:reply, raised, lease}
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

  @impl true
  def terminate(_reason, lease), do: close(lease)

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
    Log.restore(lease.artifact, snapshot, entries, file(lease))
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
