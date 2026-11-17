defmodule Database do
  @moduledoc """
  The database: a kind of artifact that other plugins keep their data in. It is SQLite, and the plugin's own
  artifact holds it as a log and snapshots (`Database.Log`), which one server at a time writes (`Database.Lease`).
  """
  @behaviour OneCore.Plugin

  # Most plugins keep what they keep in it, the essential ones among them.
  @impl true
  def icon, do: "icon.svg"

  @impl true
  def essential, do: true

  alias Database.Lease
  alias OneCore.Call

  @impl true
  def hooks, do: ["renamed"]

  # A renamed plugin's database is under its new name now. Each server that may hold the old one's lease lets go of
  # it, so a plugin given the old name later starts from what the bucket has, not from this one's copy.
  @impl true
  def hook("renamed", %{"for" => at, "from" => from}, call) do
    lease = OneCore.Plugins.Process.whereis(call.who, "Database.Lease", {call.server, "#{at}/own/#{from}/"})
    if lease, do: OneCore.Plugins.Process.stop(lease)
    if call.forwarded == false, do: Call.forward(call, OneCore.Servers.acting(at) -- [call.server])
  end

  @doc """
  Runs `fun` in a transaction on the calling plugin's database, after the `schema` statements it hasn't run yet,
  and gives what `fun` returns once what it wrote is in the bucket.
  """
  def transaction(%Call{} = call, schema \\ [], fun) when is_list(schema) and is_function(fun, 1) do
    lease = Lease.of(call)
    if lease == self(), do: raise("A transaction is already running here: use `query` inside it")

    case :gen_statem.call(lease, {call.own.codec, {:transaction, schema, fun}}) do
      {:ok, result} ->
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
  # again, a hook or a tool that only reads, before it's taken as gone. One that's there but stuck keeps the call
  # open, saying it's still running it, and a hook gives up at 10 s (`OneCore.Hooks`): so every hook passed to a
  # stuck holder gave up before anything took the lease back, and the next did the same. A holder that's up runs one
  # in well under a second, and this leaves a hook time to take the lease over and answer.
  @reach 4_000

  # The server holding the lease didn't run a call passed on to it: it's gone, or it can't run this call, as one
  # that needs what only another server has can't be passed on again (a Vault item only some servers open, say).
  # Either way this server takes the lease, if the log still names that one, and runs the call itself. But not
  # reaching it just now isn't its being gone: a network loses a connection now and then, and what this server knew
  # of it may be from before it came back. So one given up on at once is asked once more first. One that was waited
  # for has had what it was sent sent again by then (`@waited`). A server that isn't up takes no lease, so the call
  # fails here, for whoever asked to ask another.
  defp taken_over(call, lease, server) do
    began = System.monotonic_time(:millisecond)
    if server, do: Call.forward_to(call, server, @reach)
    if server && System.monotonic_time(:millisecond) - began < @waited, do: Call.forward_to(call, server, @reach)

    with :not_up <- :gen_statem.call(lease, {:take_over, server}) do
      held =
        if server, do: "#{OneCore.Servers.name(server) || server} holds it and didn't answer", else: "nobody holds it"

      raise "This server is starting or stopping, so it doesn't take a database over, and #{held}. Try again in a moment."
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

    case :gen_statem.call(lease, {call.own.codec, {:schedule, jobs, fire, System.os_time(:second)}}) do
      :ok ->
        :ok

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

  @doc "Runs one SQL statement in a transaction, with a `?` for each value, and gives its rows."
  def query({sqlite, db}, sql, values \\ []) do
    case sqlite.query(db, sql, values) do
      {:ok, rows} -> rows
      {:error, why} -> raise "#{why}, in: #{sql}"
    end
  end
end
