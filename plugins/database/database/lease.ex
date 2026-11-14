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
  The process for the database that is the calling plugin's own artifact, on the server running the call. It's
  found by a name that Erlang's `:global` keeps, and outlives the call that started it.

  A process an earlier version of this plugin started is stopped first, since each version runs its own code.
  """
  def of(%OneCore.Call{own: artifact, server: server} = call) do
    name = {:global, {:database, server, artifact.prefix}}

    case GenServer.whereis(name) do
      nil ->
        case GenServer.start(__MODULE__, {artifact, server}, name: name) do
          {:ok, lease} -> lease
          {:error, {:already_started, _lease}} -> of(call)
        end

      lease ->
        if :proc_lib.translate_initial_call(lease) == {__MODULE__, :init, 1} do
          lease
        else
          stop(lease)
          of(call)
        end
    end
  end

  defp stop(lease) do
    GenServer.stop(lease)
  catch
    :exit, _already -> :ok
  end

  @impl true
  def init({artifact, server}) do
    {:ok, look(%{artifact: artifact, server: server, db: nil, file: nil, number: 0, held_by: nil, logged: 0, pages: 0})}
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

  defp open(lease, snapshot, entries) do
    file = file(lease.server)
    Log.restore(lease.artifact, snapshot, entries, file)
    {:ok, db} = Sqlite.open(file)

    # The bucket is what keeps a write, so SQLite needn't wait for the disk. Its write-ahead log is emptied here
    # after each transaction, never by SQLite itself.
    for pragma <- ["journal_mode = wal", "synchronous = off", "wal_autocheckpoint = 0"] do
      {:ok, _} = Sqlite.query(db, "pragma " <> pragma, [])
    end

    logged = Enum.sum_by(entries, &length(Map.get(&1, :pages, [])))
    %{lease | db: db, file: file, held_by: lease.server, logged: logged}
  end

  # Writes what the transaction changed as the log's next entry.
  defp log(lease) do
    case Log.changed(lease.file) do
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
    Log.keep(lease.artifact, lease.number, lease.file)
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
    for file <- [lease.file, lease.file <> "-wal", lease.file <> "-shm"], do: File.rm(file)
    %{lease | db: nil, file: nil}
  end

  # A new file for a database's copy, in a folder of this run of the server. Earlier runs' folders are removed,
  # since their copies may hold what was never in the bucket.
  defp file(server) do
    runs = Path.join([System.tmp_dir!(), "one-database", server])
    File.mkdir_p!(Path.join(runs, System.pid()))
    for run <- File.ls!(runs), run != System.pid(), do: File.rm_rf!(Path.join(runs, run))
    Path.join([runs, System.pid(), "#{System.unique_integer([:positive])}"])
  end

  # The schema's statements this database hasn't run, which SQLite's `user_version` counts.
  defp migrate(db, schema) do
    [%{"user_version" => ran}] = Database.query(db, "pragma user_version")

    if length(schema) > ran do
      for statement <- Enum.drop(schema, ran), do: Database.query(db, statement)
      Database.query(db, "pragma user_version = #{length(schema)}")
    end
  end
end
