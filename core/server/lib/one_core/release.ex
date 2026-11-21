defmodule OneCore.Release do
  @moduledoc """
  A server installed on a machine, as the program that keeps it running starts it (`one serve run`, in
  `clients/cli/src/serving.rs`): `bin/one_core eval 'OneCore.Release.serve()'`. It says on its output when it's
  running, when it's asked to update and when it can reach the network again, and that program does the rest: it
  takes the latest build and `one`, which this server reads for it (`latest`), starts them to verify them, and goes
  back to the pair before if they don't start and answer by their key, which only this server is asked (MCP's
  `ping`, `OneCore.Mcp`). Whether a build is good is about this server alone, as a readiness check is: no other
  server, and none of the apps they run, is asked, so every server can take a build at once.

  What each says to the other is a line of JSON, one of the messages the `client` crate defines
  (`core/client/src/serve.rs`), which this reads and writes through its NIF (`OneCore.Iroh.told/1` and `said/1`).
  That program says what this server is (`status/0`) as it starts it, in `ONE_SERVER_STATUS`, and as that changes, a
  line on its input each time:

  - `server_membership`: whether this device is one of its network's servers: `joining` while `one serve on` makes
    it one, until it answers by its key, `active`, and `leaving` while `one serve off` drains it.
  - `server_daemon_status`: `starting` until it answers by its key, `running`, and `stopping`. Stopped is what the
    network's edge says of one that's gone.
  - `server_update_status`: `up_to_date`, `downloading` the latest build and `one` while this one answers on and
    checking that the build starts, `checked`, and `verifying` them once it's told to go, which starts them in this
    one's place, kept once the build says it's running at the edge; and otherwise the pair before is started again,
    saying why in `went_back`.

  Its network's updater (`core/edge/updater/updater.ts`) says, through the relay, what each server is to do: prepare
  the latest build, or go (`instructed/1`). This says it to that program, which does it.

  It's ready once it's active and running (`OneCore.Server.up?/0`), which it says at the network's edge
  (`OneCore.Servers.report/0`), so only a ready server is sent calls, and only a ready one takes on what others then
  go to it for. A server nothing keeps running, as a network being built runs one, is active and running once its
  apps have started.
  """
  require Logger
  alias OneCore.{Builds, Iroh, Server, Servers}

  @doc """
  Starts the server, says so, and runs until it's stopped: until its input closes, as the program that started it
  stops it, or as a system stops a program, with SIGTERM. Either way it takes nothing new and answers what it took
  already (`OneCore.Application`). Until then, each line on its input says what changed of it.

  It's started permanent, as OTP's releases start theirs: if it stops, as its supervisor does once its children fail
  more often than it restarts them, the runtime stops with it, so the system starts the server again, rather than
  keeping a runtime with nothing in it that the network sees as gone.
  """
  def serve do
    case Application.ensure_all_started(:one_core, :permanent) do
      {:ok, _started} ->
        say(%{"running" => %{"key" => Server.key()}})
        read()

      {:error, {:one_core, {{:not_joined, key}, _start}}} ->
        IO.puts(:stderr, "This server, #{key}, isn't on the list of whoever it's for: `one serve on` puts it there.")
        System.halt(1)

      {:error, why} ->
        IO.puts(:stderr, "The server didn't start: #{inspect(why)}")
        System.halt(1)
    end
  end

  @doc """
  Reads what the program that started it tells it, a line at a time, until its input closes, and then stops the
  server: what changed of it (`told/1`), and to say the latest builds.
  """
  def read do
    case IO.read(:line) do
      line when is_binary(line) ->
        case told_in(line) do
          {:ok, %{"latest" => target}} ->
            latest(target)

          {:ok, said} ->
            told(said)

          {:error, why} ->
            Logger.warning("The program keeping this server running said what it doesn't read (#{why}): #{line}")
        end

        read()

      _closed ->
        System.stop()
        Process.sleep(:infinity)
    end
  end

  # Says the latest builds for the kind of machine `target`, as the program keeping this server running asks before it
  # takes them: read from the network's bucket by this server, so that taking a build asks no other server, and so
  # none of their apps, which may be restarting in the same release.
  defp latest(target) do
    say(%{"latest" => %{"builds" => Builds.download(target)}})
  rescue
    error -> say(%{"no_latest" => %{"why" => Exception.message(error)}})
  end

  # Says `message`, the terms of one of the `client` crate's `Said` as JSON has them, to the program keeping this
  # server running, or to the program checking this build: a line of JSON.
  defp say(message) do
    {:ok, line} = Iroh.said(JSON.encode!(message))
    IO.puts(line)
  end

  # What a server is as it starts, where nothing says otherwise: one nothing keeps running, as a network being built
  # runs one. The program keeping an installed one says all of it, in `ONE_SERVER_STATUS`.
  @status %{
    "server_membership" => "active",
    "server_daemon_status" => "starting",
    "server_update_status" => "up_to_date",
    "server_update_stage" => nil,
    "next_build" => nil,
    "program_name" => nil,
    "program" => nil,
    "next_program" => nil,
    "went_back" => nil,
    "deploys_edge" => false,
    "edge_went_back" => nil
  }

  # What a line the program keeping it running said is, a `Told`.
  defp told_in(line) do
    with {:ok, json} <- Iroh.told(line), do: {:ok, JSON.decode!(json)}
  end

  @doc """
  What this server is, as the program keeping it running last said: `server_membership`, `server_daemon_status`,
  `server_update_status`, with its `server_update_stage` while it's verifying, and `went_back`, why the last update
  didn't take, until one does.
  """
  def status do
    # As it was started, until the program says more.
    Map.merge(@status, :persistent_term.get({__MODULE__, :status}, nil) || Application.get_env(:one_core, :status, %{}))
  end

  @doc """
  Keeps what changed of this server, `said`, as the program keeping it running says it, or as it stops. It's said in
  `builds` and at the network's edge, even where the edge or the bucket can't be reached just now, and whoever waits
  for it (`await/1`) is told, as `OneCore.Apps` is, which tells the clients watching.
  """
  def told(said) do
    was = status()
    now = Map.merge(was, Map.take(said, Map.keys(@status)))

    if now != was do
      # Not the application's environment, which can't be changed while the server stops, when it says so.
      :persistent_term.put({__MODULE__, :status}, now)
      if now["server_update_status"] != was["server_update_status"], do: :persistent_term.erase({__MODULE__, :asked})
      # The edge first: the network's updater waits on it.
      Servers.report()
      Builds.say(Server.key())

      Registry.dispatch(OneCore.Waiting, :status, fn waiting ->
        for {process, _} <- waiting, do: send(process, :status)
      end)
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

  @doc """
  Says to the program keeping this server running that the network's edge took what it said it is
  (`OneCore.Servers.say/0`), as `build`: what it waits for to keep a build it started or loaded.
  """
  def reported(build) do
    if Application.get_env(:one_core, :installed, false), do: say(%{"reported" => %{"build" => build}})
    :ok
  end

  @doc """
  What the network's updater last said of the network's update (`core/edge/updater/updater.ts`), as every server is
  told: its `network_update_status`, the `build` it takes the network to, how many of its servers are `done` `of`
  those taking part, and what it waits for or why it's held; `nil` until it has said.
  """
  def network, do: :persistent_term.get({__MODULE__, :network}, nil)

  @doc """
  What the network's updater says, `update`, as the relay tells every server: kept for whoever asks (`network/0`), told
  to the clients watching this server (`notifications/one/update`), and what this server is to do of it said to the
  program keeping it running: to prepare the latest pair, its build and
  `one`, where it isn't on it or preparing it, and to go where it has checked the pair the network takes, now or once
  it's quiet. `deploys_edge` says whether an app here deploys the network's edge (the hook `edge`), which is said at
  the edge with the rest; `:deploy_edge` is given where this server is to deploy its build's edge, once until it says
  how that went (`edge_deployed/2`), and `:ok` otherwise.
  """
  def instructed(update, deploys_edge) do
    told(%{"deploys_edge" => deploys_edge})

    if update != network() do
      :persistent_term.put({__MODULE__, :network}, update)
      # The apps watching this server hear it too, as each takes a newer build of itself, or of its page, by itself.
      Server.tell("update", update)

      Registry.dispatch(OneCore.Waiting, :network, fn waiting ->
        for {process, _} <- waiting, do: send(process, :network)
      end)
    end

    status = status()
    {build, program} = target(update, status)
    at = status["server_update_status"]
    next = %{"build" => status["next_build"], "program" => status["next_program"]}
    checked = at == "checked" and on?({build, program}, next)

    # One that checked a pair older than the network's is asked again, as the list it read was older than the edge's;
    # one that checked a newer pair isn't, as it would check the same again, over and over (20 Nov, once a second for
    # an hour), and waits for the edge to read the newer list, within five minutes.
    behind =
      (at == "up_to_date" and not on?({build, program}, Map.put(status, "build", Builds.build()))) or
        (at == "checked" and not checked and older?(next, {build, program}))

    # Said once until the program says it began, though the updater says it again to every server as each moves on.
    asked = :persistent_term.get({__MODULE__, :asked}, nil) == {build, program}

    case Application.get_env(:one_core, :installed, false) && update["servers"][Server.key()] do
      "prepare" when behind and not asked ->
        :persistent_term.put({__MODULE__, :asked}, {build, program})
        say("prepare")

      "go" when checked ->
        say("go")

      "go_when_quiet" when checked ->
        quiet()

      _ ->
        :ok
    end

    if update["servers"][Server.key()] == "deploy_edge" and not :persistent_term.get({__MODULE__, :edge}, false) do
      :persistent_term.put({__MODULE__, :edge}, true)
      :deploy_edge
    else
      :ok
    end
  end

  @doc """
  The network's edge as this server's build has it, which `bun core/edge/edge.js build` made and `server-code`
  carries: the folder `deploy.js` deploys it from, or `nil` for a server with none, as one run from source.
  """
  def edge do
    folder = Path.join(to_string(:code.root_dir()), "edge")
    if File.exists?(Path.join(folder, "deploy.js")), do: folder
  end

  @doc """
  Says how deploying the edge of `build` went (the hook `edge`): `:ok`, or `{:error, why}`, which the network's update
  says of this server until a deploy of it goes. It's told again to deploy it as the update tries again.
  """
  def edge_deployed(build, result) do
    :persistent_term.erase({__MODULE__, :edge})
    told(%{"edge_went_back" => with({:error, why} <- result, do: %{"build" => build, "why" => why}, else: (:ok -> nil))})
    :ok
  end

  @doc """
  The pair the network's update, `update`, takes a server to, as the server says what it is (`status/0`, which says
  the `program_name` of the `one` running it): `{build, program}`, the latest server build and the latest of that
  `one`, as the updater says the latest of each name (`latest`), and `program` `nil`, any `one`, where it doesn't
  say, as one deployed before it did.
  """
  def target(update, server) do
    latest = update["latest"] || %{}
    {latest["server-code"] || update["build"], latest[server["program_name"]]}
  end

  # Whether the pair `next` a server checked is older than `target` in its build or its `one`.
  defp older?(next, {build, program}),
    do:
      Builds.count(next["build"]) < Builds.count(build) or
        (program != nil and Builds.count(next["program"]) < Builds.count(program))

  @doc """
  Whether a server on the pair `server`, its `build` and `program`, is on `target`, any `one` where either doesn't say,
  and any `one` as new as the target's or newer: the target is the `one` that first ran a server as it's run now
  (`OneCore.Builds`), and one taken since with a server build runs it the same.
  """
  def on?({build, program}, server),
    do: server["build"] == build and (program == nil or server["program"] == nil or Builds.count(server["program"]) >= Builds.count(program))

  # How long Update Now waits for the network's update to finish, in milliseconds: a restart is seconds, and each server
  # restarts in turn.
  @update_now 600_000

  @doc """
  Update Now: tells the network's updater to take every server to the latest build now, those that went back from it too, and gives
  what it says once it's up to date, as every server is told (`instructed/1`), or after
  `@update_now`, as it is then.
  """
  def update_now do
    Registry.register(OneCore.Waiting, :network, nil)

    with {:ok, update} when update != nil <- Servers.network(%{now: true}) do
      await_network(update, System.monotonic_time(:millisecond) + @update_now)
    else
      {:ok, nil} -> raise "This server has no edge address, so there's no network update to ask for."
      {:error, why} -> raise "The network's updater wasn't told: #{why}"
    end
  after
    Registry.unregister(OneCore.Waiting, :network)
  end

  defp await_network(update, until) do
    left = until - System.monotonic_time(:millisecond)

    if update["network_update_status"] == "up_to_date" or left <= 0 do
      update
    else
      receive do
        :network -> await_network(network(), until)
      after
        left -> network()
      end
    end
  end

  @doc """
  The network's update, `update`, as a person reads it, in Software Update's words: "Up to date · 0.20.1",
  "Updating… 2 of 4", "Waiting for a quiet moment", and after either, each server that went back and why: "surface didn't
  update: why".
  """
  def software_update(nil), do: "This network's edge has no updater yet."

  def software_update(update) do
    version = (update["build"] || "") |> String.split("+") |> hd()

    now =
      case {update["network_update_status"], update["waiting_for"]} do
        {"up_to_date", _} -> "Up to date · #{version}"
        {_, "quiet"} -> "Waiting for a quiet moment · #{version}"
        _ -> "Updating… #{update["done"]} of #{update["of"]}"
      end

    Enum.join(
      [now | for(%{"name" => name, "why" => why} <- update["went_back"] || [], do: "#{name} didn't update: #{why}")],
      " · "
    )
  end

  # How long this server goes without a call before it's quiet enough to restart, as the one ready server, in
  # milliseconds.
  @quiet 120_000

  @doc "A client called one of its tools, so it isn't quiet (`instructed/1`)."
  def called do
    case :persistent_term.get({__MODULE__, :called}, nil) do
      nil ->
        :persistent_term.put({__MODULE__, :called}, :atomics.new(1, signed: true))
        called()

      calls ->
        :atomics.put(calls, 1, System.monotonic_time(:millisecond))
    end
  end

  # Says go once nothing has called this server for `@quiet`, and it still has the network's build checked: one
  # waiting, which is all.
  defp quiet do
    if Process.whereis(OneCore.Release.Quiet) == nil do
      Task.start(fn ->
        Process.register(self(), OneCore.Release.Quiet)
        waiting()
      end)
    end

    :ok
  end

  defp waiting do
    since =
      case :persistent_term.get({__MODULE__, :called}, nil) do
        nil -> @quiet
        calls -> System.monotonic_time(:millisecond) - :atomics.get(calls, 1)
      end

    cond do
      status()["server_update_status"] != "checked" -> :ok
      since >= @quiet -> say("go")
      true -> receive(after: (@quiet - since -> waiting()))
    end
  end

  @doc """
  Says which build this is, once its code and NIFs have loaded: what's checked of a build before it's started.
  """
  def check do
    Application.load(:one_core)
    for module <- Application.spec(:one_core, :modules), do: Code.ensure_loaded!(module)

    for module <- Application.spec(:one_core, :modules),
        function_exported?(module, :__nif__, 0),
        do: OneCore.Nif.build(module)

    say(%{"checked" => %{"build" => Builds.build()}})
  end
end
