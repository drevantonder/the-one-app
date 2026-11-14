defmodule Database do
  @moduledoc """
  The database: a kind of artifact that other plugins keep their data in. It is SQLite, and the plugin's own
  artifact holds it as a log and snapshots (`Database.Log`), which one server at a time writes (`Database.Lease`).
  """
  @behaviour OneCore.Plugin
  alias Database.Lease
  alias OneCore.Call

  @impl true
  def guide do
    """
    A plugin keeps its data in a database by using this plugin. The database is SQLite, kept in the plugin's own
    artifact for whoever it runs for, so each user and each group has their own.

        def uses, do: ["database"]

        @schema ["create table notes (id integer primary key, text text not null, at text not null)"]

        def call("add", %{"text" => text}, call) do
          database = OneCore.Call.plugin(call, "database")
          at = DateTime.to_iso8601(DateTime.utc_now(:second))

          database.transaction(call, @schema, fn db ->
            [%{"id" => id}] =
              database.query(db, "insert into notes (text, at) values (?, ?) returning id", [text, at])

            "Added note \#{id}."
          end)
        end

    - `transaction(call, schema, fun)` runs `fun` with the database and gives what it returns. What `fun` writes
      is kept all together or not at all: raise to keep none of it. Once it returns, what was written is safe in
      the bucket, and losing the server loses none of it.
    - `query(db, sql, values)` runs one SQL statement with a `?` for each value, and gives its rows, each a map
      from a column's name to its value. `returning` gives back what an insert or an update made. A value is a
      number, text, `nil`, a boolean (kept as 1 or 0) or `{:blob, binary}`.
    - `schema` is the statements that make the tables, in the order they were added. Each runs once on each
      database. To change the tables, add statements to the end, and leave the ones already deployed as they are.
    - The database is the blobs under `log/` and `snapshot/` in the plugin's own artifact. Every other name there
      is the plugin's, for what isn't rows: a photo, a file.
    - One server at a time writes a database, and a call that reaches another is passed on to it. So the tool may
      start again on another server, and `fun` may run more than once. Do everything that touches the database
      inside `fun`, and what reaches outside, such as sending a message, after `transaction` returns.
    """
  end

  @doc """
  Runs `fun` in a transaction on the calling plugin's database, after the `schema` statements it hasn't run yet,
  and gives what `fun` returns once what it wrote is in the bucket.
  """
  def transaction(%Call{} = call, schema \\ [], fun) when is_list(schema) and is_function(fun, 1) do
    lease = Lease.of(call)
    if lease == self(), do: raise("A transaction is already running here: use `query` inside it")

    case GenServer.call(lease, {:transaction, schema, fun}, :infinity) do
      {:ok, result} ->
        result

      {:raised, kind, reason, stacktrace} ->
        :erlang.raise(kind, reason, stacktrace)

      # The call ends here if the server holding the lease runs it. If that one doesn't answer, this one takes
      # the lease.
      {:held_by, server} ->
        :no_answer = Call.pass_on(call, server)
        :ok = GenServer.call(lease, {:take_over, server}, :infinity)
        transaction(call, schema, fun)
    end
  end

  @doc "Runs one SQL statement in a transaction, with a `?` for each value, and gives its rows."
  def query(db, sql, values \\ []) do
    case Sqlite.query(db, sql, values) do
      {:ok, rows} -> rows
      {:error, why} -> raise "#{why}, in: #{sql}"
    end
  end
end
