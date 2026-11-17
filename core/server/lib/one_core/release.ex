defmodule OneCore.Release do
  @moduledoc """
  A server installed on a machine, as the program that keeps it running starts it (`one serve run`, in
  `clients/cli/src/serving.rs`): `bin/one_core eval 'OneCore.Release.serve()'`. It says on its output when it's
  running, and when it's asked to update, and that program does the rest: it takes the latest build and `one`, starts
  them to verify them, and goes back to the pair before if they don't start and answer by their key.

  That program says what this server is (`status/0`) as it starts it, in `ONE_SERVER_STATUS`, and as that changes, a
  line of JSON on its input each time:

  - `server_membership`: whether this device is one of its network's servers: `joining` while `one serve on` makes
    it one, until it answers by its key, `active`, and `leaving` while `one serve off` drains it.
  - `server_daemon_status`: `starting` until it answers by its key, `running`, and `stopping`. Stopped is what the
    network's edge says of one that's gone.
  - `server_update_status`: `up_to_date`, `downloading` the latest build and `one` while this one answers on, and
    `verifying` them, which starts them in this one's place: kept once they answer by their key, and otherwise the
    pair before starts again, saying why in `went_back`.

  It's ready once it's active and running (`OneCore.Server.up?/0`), which it says at the network's edge
  (`OneCore.Servers.report/0`), so only a ready server is sent calls, and only a ready one takes on what others then
  go to it for. A server nothing keeps running, as a network being built runs one, is active and running once its
  plugins have started.
  """
  require Logger
  alias OneCore.{Builds, Server, Servers}

  @doc "What it says when it's running, with its key after."
  def running, do: "A server is running: "

  @doc "What it says when it's asked to take the latest build."
  def update_asked, do: "Update asked"

  @doc """
  Starts the server, says so, and runs until it's stopped: until its input closes, as the program that started it
  stops it, or as a system stops a program, with SIGTERM. Either way it takes nothing new and answers what it took
  already (`OneCore.Application`). Until then, each line on its input says what changed of it.
  """
  def serve do
    case Application.ensure_all_started(:one_core) do
      {:ok, _started} ->
        IO.puts(running() <> Server.key())
        told()
        System.stop()
        Process.sleep(:infinity)

      {:error, {:one_core, {{:not_joined, key}, _start}}} ->
        IO.puts(:stderr, "This server, #{key}, isn't on the list of whoever it's for: `one serve on` puts it there.")
        System.halt(1)

      {:error, why} ->
        IO.puts(:stderr, "The server didn't start: #{inspect(why)}")
        System.halt(1)
    end
  end

  # Reads what the program that started it tells it, until its input closes.
  defp told do
    with line when is_binary(line) <- IO.read(:line) do
      case JSON.decode(line) do
        {:ok, %{} = said} -> told(said)
        _ -> Logger.warning("The program keeping this server running said what it doesn't read: #{String.trim(line)}")
      end

      told()
    end
  end

  @status %{
    "server_membership" => "active",
    "server_daemon_status" => "starting",
    "server_update_status" => "up_to_date",
    "went_back" => nil
  }

  @doc """
  What this server is, as the program keeping it running last said: `server_membership`, `server_daemon_status`,
  `server_update_status` and `went_back`, why the last update didn't take, until one does.
  """
  def status, do: Map.merge(@status, Application.get_env(:one_core, :status, %{}))

  @doc """
  Keeps what changed of this server, `said`, as the program keeping it running says it, or as it stops. It's said in
  `builds` and at the network's edge, even where the edge or the bucket can't be reached just now, and whoever waits
  for it (`await/1`) and the clients watching are told.
  """
  def told(said) do
    was = status()
    now = Map.merge(was, Map.take(said, Map.keys(@status)))

    if now != was do
      Application.put_env(:one_core, :status, now)
      Builds.say(Server.key())
      Servers.report()
      Registry.dispatch(OneCore.Waiting, :status, fn waiting -> for {process, _} <- waiting, do: send(process, :status) end)
      Server.changed()
    end
  rescue
    error -> Logger.warning("Couldn't say what changed of this server: #{Exception.message(error)}")
  end

  @doc "Returns once `test` holds of this server's `status/0`, which only what it's told changes."
  def await(test) do
    Registry.register(OneCore.Waiting, :status, nil)
    awaiting(test, :infinity)
  after
    Registry.unregister(OneCore.Waiting, :status)
  end

  defp awaiting(test, within) do
    if test.(status()), do: :ok, else: receive(do: (:status -> awaiting(test, within)), after: (within -> :late))
  end

  # How long the program keeping a server running takes to say it has begun an update, in milliseconds: it reads the
  # server's output as it comes, so this is only for a machine that's very busy.
  @begun 5_000

  @doc """
  Has the program keeping this server running take the latest builds, unless an update is under way already. Gives
  the update's `server_update_status` once that program has said it, `@begun` at most.
  """
  def update do
    if status()["server_update_status"] == "up_to_date" do
      Registry.register(OneCore.Waiting, :status, nil)
      IO.puts(update_asked())
      awaiting(&(&1["server_update_status"] != "up_to_date"), @begun)
      Registry.unregister(OneCore.Waiting, :status)
    end

    status()["server_update_status"]
  end

  @doc "Says which build this is, once its code and NIFs have loaded: what's checked of a build before it's started."
  def check do
    Application.load(:one_core)
    for module <- Application.spec(:one_core, :modules), do: Code.ensure_loaded!(module)

    for module <- Application.spec(:one_core, :modules),
        function_exported?(module, :__nif__, 0),
        do: OneCore.Nif.build(module)

    IO.puts(Builds.build())
  end
end
