defmodule OneCore.DatabaseTest do
  @moduledoc """
  The database plugin's lease, simulated. Servers write one database, stop without warning and start again, in an
  order a seed decides, on a bucket that does one operation at a time: after each, the seed picks which server goes
  next, or stops one, before its operation or after it. A server that doesn't hold the lease takes it, whether or
  not the one holding it is still running.

  No write that was answered may be lost, and none may be there twice. A seed always runs the same way, so one
  that fails is a bug to find, never a flake. `SEEDS=5000 mix test test/database_test.exs` runs more of them.
  """
  use ExUnit.Case
  alias OneCore.{Artifact, Call}
  alias OneCore.Plugins.Version

  @servers 3
  @steps 1000
  @schema ["create table writes (id integer, filler blob)"]

  defmodule Bucket do
    @moduledoc "A bucket whose every operation waits for the simulation to do it."
    @behaviour OneCore.Bucket

    @impl true
    def read(simulation, key), do: wait(simulation, {:read, key})
    @impl true
    def write(simulation, key, data), do: wait(simulation, {:write, key, data})
    @impl true
    def replace(simulation, key, data, tag), do: wait(simulation, {:replace, key, data, tag})
    @impl true
    def list(simulation, prefix), do: wait(simulation, {:list, prefix})

    # A server's operations come from the process that is the server, or from one that process started.
    defp wait({blobs, simulation}, operation) do
      server = List.first(Process.get(:"$ancestors", []), self())

      if self() == simulation do
        perform(blobs, operation)
      else
        send(simulation, {:operation, server, self(), operation})

        receive do
          {:done, result} -> result
        end
      end
    end

    def perform(blobs, {:read, key}) do
      case :ets.lookup(blobs, key) do
        [{_key, data}] -> {:ok, data, data}
        [] -> :absent
      end
    end

    def perform(blobs, {:write, key, data}), do: if(:ets.insert_new(blobs, {key, data}), do: :ok, else: :exists)

    def perform(blobs, {:replace, key, data, tag}) do
      if :ets.lookup(blobs, key) == [{key, tag}] and :ets.insert(blobs, {key, data}), do: :ok, else: :changed
    end

    def perform(blobs, {:list, prefix}) do
      Enum.sort(for {key, _data} <- :ets.tab2list(blobs), String.starts_with?(key, prefix), do: key)
    end
  end

  setup_all %{} do
    # A server of this network, so that asking another server is what it is anywhere: here none of them answers.
    home = Path.join(System.tmp_dir!(), "one-core-test-#{System.pid()}")
    Application.put_env(:one_core, :home, home)
    Application.put_env(:one_core, :bucket, {OneCore.Bucket.Folder, Path.join(home, "bucket")})
    start_supervised!(OneCore.Server)
    on_exit(fn -> File.rm_rf!(home) end)

    folder = Path.expand("../../plugins/database", __DIR__)

    files =
      for file <- Path.wildcard(folder <> "/**/*.ex"), into: %{}, do: {Path.relative_to(file, folder), File.read!(file)}

    {:ok, database, _warnings} = Version.load(files)
    on_exit(fn -> Version.unload(database) end)
    %{database: database}
  end

  test "no answered write is lost or doubled, whoever stops and whoever takes the lease", %{database: database} do
    seeds = String.to_integer(System.get_env("SEEDS", "40"))
    assert run(database, 1) == run(database, 1), "seed 1 ran two ways"
    for seed <- 2..seeds//1, do: run(database, seed)
  end

  # One seed's run. Gives everything that happened, in order.
  defp run(database, seed) do
    blobs = :ets.new(:blobs, [:public])
    Application.put_env(:one_core, :bucket, {Bucket, {blobs, self()}})

    run = %{
      seed: seed,
      database: database,
      blobs: blobs,
      random: :rand.seed_s(:exsss, seed),
      servers: Map.new(1..@servers, &{&1, nil}),
      waiting: %{},
      next: 1,
      answered: [],
      refused: [],
      happened: []
    }

    run = Enum.reduce(1..@steps, run, fn _step, run -> step(run) end)
    for {_server, process} when process != nil <- run.servers, do: stop(process)

    # A server that has never seen the database reads it from the bucket.
    there = check(run, "after")
    assert there == Enum.uniq(there), "seed #{seed}: a write is there twice: #{inspect(there -- Enum.uniq(there))}"
    assert run.answered -- there == [], "seed #{seed}: answered writes are lost: #{inspect(run.answered -- there)}"
    assert there -- Enum.to_list(1..run.next) == [], "seed #{seed}: a write nobody made is there"
    assert there -- run.refused == there, "seed #{seed}: a write that was refused is there"

    # The log alone, without its snapshots, is the same database.
    for {key, _data} <- :ets.tab2list(blobs), key =~ "/snapshot/", do: :ets.delete(blobs, key)
    assert check(run, "log") == there, "seed #{seed}: a snapshot isn't what the log before it makes"

    :ets.delete(blobs)
    Enum.reverse([{:there, there} | run.happened])
  end

  defp step(run) do
    {server, run} = random(run, @servers)
    {chance, run} = random(run, 100)
    process = run.servers[server]

    cond do
      process == nil ->
        start(run, server)

      is_map_key(run.waiting, server) ->
        {from, operation} = run.waiting[server]
        writes = elem(operation, 0) in [:write, :replace]

        cond do
          chance <= 2 ->
            stopped(run, server, :before, operation)

          chance <= 5 and writes ->
            Bucket.perform(run.blobs, operation)
            stopped(run, server, :after, operation)

          true ->
            send(from, {:done, Bucket.perform(run.blobs, operation)})
            await(%{run | waiting: Map.delete(run.waiting, server)}, server)
        end

      chance <= 1 ->
        stopped(run, server, :idle, nil)

      # One write in ten is refused after it has written more than SQLite holds in memory.
      true ->
        send(process, {:write, run.next, chance > 90})
        await(happened(%{run | next: run.next + 1}, {:write, server, run.next}), server)
    end
  end

  # Waits for what the one server that is running does next: a bucket operation, or the answer to its write.
  defp await(run, server) do
    process = run.servers[server]

    receive do
      {:operation, ^process, from, operation} ->
        happened(%{run | waiting: Map.put(run.waiting, server, {from, operation})}, {server, operation_name(operation)})

      {:wrote, ^process, id, :ok} ->
        happened(%{run | answered: [id | run.answered]}, {:answered, id})

      {:wrote, ^process, id, :refused} ->
        happened(%{run | refused: [id | run.refused]}, {:refused, id})

      {:wrote, ^process, id, why} ->
        flunk("seed #{run.seed}: write #{id} failed, which nothing here should make happen: #{why}")
    after
      10_000 -> flunk("seed #{run.seed}: server #{server} waits for something that never comes")
    end
  end

  defp operation_name(operation), do: {elem(operation, 0), elem(operation, 1)}

  defp start(run, server) do
    simulation = self()
    %{database: %{plugin: database}} = run

    process = spawn(fn -> write(database, call("s#{server}"), simulation) end)
    happened(%{run | servers: Map.put(run.servers, server, process)}, {:started, server})
  end

  # A server's whole work: each write it's given, answered once the database plugin has answered it.
  defp write(database, call, simulation) do
    receive do
      {:write, id, refuse} ->
        wrote =
          try do
            database.transaction(call, @schema, fn db ->
              filler = if refuse, do: 3_000_000, else: 9000
              database.query(db, "insert into writes (id, filler) values (?, zeroblob(?))", [id, filler])
              if refuse, do: throw(:refused), else: :ok
            end)
          catch
            :throw, :refused -> :refused
            kind, reason -> Exception.format(kind, reason, __STACKTRACE__)
          end

        send(simulation, {:wrote, self(), id, wrote})
    end

    write(database, call, simulation)
  end

  defp stopped(run, server, at, operation) do
    stop(run.servers[server])

    run = %{
      run
      | servers: Map.put(run.servers, server, nil),
        waiting: Map.delete(run.waiting, server)
    }

    happened(run, {:stopped, server, at, operation && operation_name(operation)})
  end

  # Stops a server as `kill -9` would: its process and the one holding its copy of the database, at once.
  defp stop(process) do
    holding =
      for {:database, _server, _prefix} = name <- :global.registered_names(),
          pid = :global.whereis_name(name),
          is_pid(pid) and process in ancestors(pid),
          do: pid

    for pid <- [process | holding] do
      watching = Process.monitor(pid)
      Process.exit(pid, :kill)

      receive do
        {:DOWN, ^watching, :process, _pid, _why} -> :ok
      end
    end
  end

  defp ancestors(pid) do
    {:dictionary, dictionary} = Process.info(pid, :dictionary)
    Keyword.get(dictionary, :"$ancestors", [])
  end

  # The ids in the database as a server that never saw it finds it, taking its lease.
  defp check(run, server) do
    %{database: %{plugin: database}} = run
    simulation = self()

    process =
      spawn(fn ->
        there =
          try do
            database.transaction(call(server), @schema, fn db ->
              [%{"integrity_check" => "ok"}] = database.query(db, "pragma integrity_check")
              for %{"id" => id} <- database.query(db, "select id from writes order by rowid"), do: id
            end)
          catch
            kind, reason -> Exception.format(kind, reason, __STACKTRACE__)
          end

        send(simulation, {:there, there})
      end)

    there = serve(run, process)
    stop(process)
    assert is_list(there), "seed #{run.seed}: the database can't be read from the bucket: #{there}"
    there
  end

  defp serve(run, process) do
    receive do
      {:operation, ^process, from, operation} ->
        send(from, {:done, Bucket.perform(run.blobs, operation)})
        serve(run, process)

      {:there, there} ->
        there
    after
      10_000 -> flunk("seed #{run.seed}: reading the database from the bucket waits for something that never comes")
    end
  end

  defp call(server) do
    %Call{
      own: %Artifact{prefix: "users/anyone/own/simulated/"},
      server: server,
      plugin: "simulated",
      tool: "write",
      inputs: %{},
      uses: %{},
      passed_on: false
    }
  end

  defp random(run, upto) do
    {number, random} = :rand.uniform_s(upto, run.random)
    {number, %{run | random: random}}
  end

  defp happened(run, what), do: %{run | happened: [what | run.happened]}
end
