defmodule Database do
  @moduledoc """
  The database: a kind of artifact that other plugins keep their data in. It is SQLite, and the plugin's own
  artifact holds it as a log and snapshots (`Database.Log`), which one server at a time writes (`Database.Lease`).
  """
  @behaviour OneCore.Plugin
  alias Database.Lease
  alias OneCore.Call

  @doc """
  Runs `fun` in a transaction on the calling plugin's database, after the `schema` statements it hasn't run yet,
  and gives what `fun` returns once what it wrote is in the bucket.
  """
  def transaction(%Call{} = call, schema \\ [], fun) when is_list(schema) and is_function(fun, 1) do
    lease = Lease.of(call)
    if lease == self(), do: raise("A transaction is already running here: use `query` inside it")

    case GenServer.call(lease, {call.own.codec, {:transaction, schema, fun}}, :infinity) do
      {:ok, result} ->
        result

      :codec_changed ->
        opened_again(lease)
        transaction(call, schema, fun)

      {:raised, kind, reason, stacktrace} ->
        :erlang.raise(kind, reason, stacktrace)

      # The call ends here if the server holding the lease runs it. If that one doesn't answer, this one takes
      # the lease.
      {:held_by, server} ->
        :no_answer = Call.forward_to(call, server)
        :ok = GenServer.call(lease, {:take_over, server}, :infinity)
        transaction(call, schema, fun)
    end
  end

  @doc """
  Has each of the calling plugin's scheduled `jobs` that is due called with `fire`, once, by the server holding
  the lease of its database: here, or by passing this on to the one that does, and taking the lease if that one
  doesn't answer. One Core calls this on every server, each time a job is due (`OneCore.Plugin`).
  """
  @impl true
  def schedule(%Call{} = call, jobs, fire) when is_list(jobs) and is_function(fire, 1) do
    lease = Lease.of(call)

    case GenServer.call(lease, {call.own.codec, {:schedule, jobs, fire, System.os_time(:second)}}, :infinity) do
      :ok ->
        :ok

      :codec_changed ->
        opened_again(lease)
        schedule(call, jobs, fire)

      {:held_by, server} ->
        :no_answer = Call.forward_to(call, server)
        :ok = GenServer.call(lease, {:take_over, server}, :infinity)
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

  @doc "Runs one SQL statement in a transaction, with a `?` for each value, and gives its rows."
  def query(db, sql, values \\ []) do
    case Sqlite.query(db, sql, values) do
      {:ok, rows} -> rows
      {:error, why} -> raise "#{why}, in: #{sql}"
    end
  end
end
