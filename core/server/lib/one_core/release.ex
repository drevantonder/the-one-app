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
  apps have started.
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
        __MODULE__.read()

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
  server: what changed of it (`told/1`), and a build to load (`load/1`). Each line is read by a call to the latest code
  of this module, so a hot load moves this loop onto the code it loaded, and the next loads nothing it runs.
  """
  def read do
    case IO.read(:line) do
      line when is_binary(line) ->
        case JSON.decode(line) do
          {:ok, %{"load" => build}} -> load(build)
          {:ok, %{} = said} -> told(said)
          _ -> Logger.warning("The program keeping this server running said what it doesn't read: #{String.trim(line)}")
        end

        __MODULE__.read()

      _closed ->
        System.stop()
        Process.sleep(:infinity)
    end
  end

  @status %{
    "server_membership" => "active",
    "server_daemon_status" => "starting",
    "server_update_status" => "up_to_date",
    "server_update_stage" => nil,
    "update_method" => nil,
    "went_back" => nil
  }

  @doc """
  What this server is, as the program keeping it running last said: `server_membership`, `server_daemon_status`,
  `server_update_status`, with its `server_update_stage` while it's verifying (`checking`, then `loading` or
  `starting`) and the `update_method` checking chose (`hot_load` or `restart`, `compare/2`), and `went_back`, why
  the last update didn't take, until one does.
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

      Registry.dispatch(OneCore.Waiting, :status, fn waiting ->
        for {process, _} <- waiting, do: send(process, :status)
      end)

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
  Has the program keeping this server running take the latest builds, unless an update is under way already, and
  gives `status/0` once it's done: up to date again, on the new build or gone back. One that restarts the server
  (`update_method`) gives it as that begins, as does one under way already, and one that hasn't begun within
  `@begun`.
  """
  def update do
    if status()["server_update_status"] == "up_to_date" do
      Registry.register(OneCore.Waiting, :status, nil)
      IO.puts(update_asked())

      with :ok <- awaiting(&(&1["server_update_status"] != "up_to_date"), @begun),
           do: awaiting(&(&1["server_update_status"] == "up_to_date" or restarting?(&1)), :infinity)

      Registry.unregister(OneCore.Waiting, :status)
    end

    status()
  end

  defp restarting?(status), do: status["update_method"] == "restart" or status["server_daemon_status"] == "stopping"

  @doc "Says which build this is, once its code and NIFs have loaded: what's checked of a build before it's started."
  def check do
    Application.load(:one_core)
    for module <- Application.spec(:one_core, :modules), do: Code.ensure_loaded!(module)

    for module <- Application.spec(:one_core, :modules),
        function_exported?(module, :__nif__, 0),
        do: OneCore.Nif.build(module)

    IO.puts(Builds.build())
  end

  @doc """
  Whether the build in the folder `new` can be loaded into a server running the one in `old` while it runs, as
  `one serve run` asks before it takes a build (both whole releases, as it keeps them): says `hot_load`, or
  `restart: <why>`, and gives it. It's run in a fresh VM of `new`, which nothing else runs in: to tell what changed,
  it loads the code of `old` there too.

  A hot load replaces only One Core's own code (`load/1`), so it restarts for anything else, and for what its code
  can't be swapped under: another runtime (`server-runtime/<kind>`: the VM, Elixir, the deps, One Core's NIFs, its
  config and `rel/`), a breaking change (another 0.minor, which doesn't talk with the build before), an `.app` that
  changed beyond its modules, and among the modules that changed, `OneCore.Application`, a supervisor, a module with
  `-on_load` or one of One Core's NIFs (`use OneCore.Nif`), a struct and a macro. A supervisor is one by its behaviour
  or by the child spec it gives (`child_spec/1`), whose children differ. A macro's own code can't be told apart from
  the rest of its module's, so any change to a module with macros restarts.
  """
  def compare(old, new) do
    said =
      case restart(old, new) do
        nil -> "hot_load"
        why -> "restart: " <> why
      end

    IO.puts(said)
    said
  end

  defp restart(old, new) do
    apps = Enum.map([old, new], &app/1)
    [was, now] = Enum.map([old, new], &named/1)

    cond do
      runtime(old) == nil or runtime(old) != runtime(new) ->
        "its runtime is #{runtime(new)}, and the server's #{runtime(old) || "a whole build's"}"

      not Builds.talk?(was, now) ->
        "#{now} is a breaking change from #{was}"

      not same_app?(apps) ->
        "One Core's .app changed beyond its modules"

      true ->
        changed(changes(old, new), old, new)
    end
  end

  @doc """
  One Core's modules that differ from the code `old` to the code `new`, as `{removed, added, changed}`, and why a
  running server can't load them, or `nil`: the rule `compare/2` applies to them. `old` and `new` are whole builds, or
  the folders a developer's `_build` keeps One Core's code in, `%{ebin: <folder>, consolidated: <folder>}`, as a local
  network's code reloader has them (`tooling/code_reloader.exs`). It loads the code of both into the VM it runs
  in, as `compare/2` does.
  """
  def modules(old, new) do
    changes = changes(old, new)
    {changes, changed(changes, old, new)}
  end

  defp changed({removed, added, changed} = changes, old, new) do
    modules = removed ++ added ++ changed

    if OneCore.Application in modules do
      "OneCore.Application changed"
    else
      [before, now] = [facts(changes, old), facts(changes, new)]

      Enum.find_value(modules, fn module ->
        [a, b] = for facts <- [before, now], do: Map.get(facts, module, %{})

        cond do
          a[:on_load] || b[:on_load] ->
            "#{inspect(module)} has -on_load"

          a[:nif] || b[:nif] ->
            "#{inspect(module)} is One Core's NIF"

          a[:supervisor] || b[:supervisor] ->
            "#{inspect(module)} is a supervisor"

          a[:macros] || b[:macros] ->
            "#{inspect(module)} has macros"

          a[:struct] != b[:struct] ->
            "#{inspect(module)}'s struct changed"

          match?(%{type: :supervisor}, a[:child_spec]) or match?(%{type: :supervisor}, b[:child_spec]) ->
            if a[:child_spec] != b[:child_spec], do: "#{inspect(module)} is a supervisor whose children changed"

          true ->
            nil
        end
      end)
    end
  end

  # The runtime a build was put together from, as `tooling/release` names it in the build's file `runtime`: `nil` for a
  # build from before there were runtimes.
  defp runtime(build) do
    case File.read(Path.join(build, "runtime")) do
      {:ok, runtime} -> String.trim(runtime)
      {:error, _} -> nil
    end
  end

  # The build a release is, its version, as its `start_erl.data` says it beside the VM's.
  defp named(build) do
    [_erts, version] = build |> Path.join("releases/start_erl.data") |> File.read!() |> String.split()
    version
  end

  defp app(build) do
    {:ok, [app]} = :file.consult(Path.join(ebin(build), "one_core.app"))
    app
  end

  defp same_app?(apps) do
    [a, b] =
      for {:application, name, keys} <- apps do
        {name, keys |> Keyword.delete(:modules) |> Enum.sort()}
      end

    a == b
  end

  defp ebin(%{ebin: ebin}), do: ebin
  defp ebin(build), do: build |> Path.join("lib/one_core-*/ebin") |> Path.wildcard() |> hd()
  defp consolidated(%{consolidated: consolidated}), do: consolidated
  defp consolidated(build), do: build |> Path.join("releases/*/consolidated") |> Path.wildcard() |> hd()

  # One Core's modules that differ from `old` to `new`, with its consolidated protocols: those only `old` has, those
  # only `new` has, and those both have whose code differs, by its MD5 (`:beam_lib.md5/1`), which leaves out the
  # attributes a build in another folder says otherwise, as `@external_resource`'s path.
  defp changes(old, new) do
    [{removed, added, changed}, {gone, new_protocols, protocols}] =
      for dir <- [&ebin/1, &consolidated/1] do
        [was, now] = for build <- [old, new], do: md5s(dir.(build))

        {Map.keys(Map.drop(was, Map.keys(now))), Map.keys(Map.drop(now, Map.keys(was))),
         for({module, md5} <- now, Map.has_key?(was, module), was[module] != md5, do: module)}
      end

    {removed ++ gone, added ++ new_protocols, changed ++ protocols}
  end

  defp md5s(dir) do
    for file <- Path.wildcard(Path.join(dir, "*.beam")), into: %{} do
      {:ok, {module, md5}} = :beam_lib.md5(to_charlist(file))
      {module, md5}
    end
  end

  # The beam file of each of `modules` in the build `build`, as `{module, file, binary}`, where it has one.
  defp files(modules, build) do
    for module <- modules,
        file <-
          Path.wildcard(Path.join(ebin(build), "#{module}.beam")) ++
            Path.wildcard(Path.join(consolidated(build), "#{module}.beam")),
        do: {module, to_charlist(file), File.read!(file)}
  end

  # What tells whether a hot load can swap each module that changed, as the build `build` has it: whether it has
  # `-on_load`, is a supervisor by its behaviour or has macros, read from its file, and its struct and child spec,
  # which only running its code gives. So the modules that changed are loaded from `build` in place of what's loaded,
  # all of them first, as one's child spec can name another's children: all but this one, which is running, and
  # one with `-on_load`, which loading would run.
  defp facts({removed, added, changed}, build) do
    files = files(removed ++ added ++ changed, build)

    told =
      for {module, _file, binary} <- files, into: %{} do
        {:ok, {_, [attributes: attributes, exports: exports]}} = :beam_lib.chunks(binary, [:attributes, :exports])
        behaviours = for {:behaviour, names} <- attributes, name <- names, do: name

        {module,
         %{
           module: module,
           on_load: :erlang.has_prepared_code_on_load(:erlang.prepare_loading(module, binary)),
           supervisor: Enum.any?(behaviours, &(&1 in [Supervisor, :supervisor, DynamicSupervisor])),
           macros: Enum.any?(exports, fn {name, _arity} -> String.starts_with?(Atom.to_string(name), "MACRO-") end),
           # Its Elixir is compiled into the NIF's build as that loads (`OneCore.Nif`), once for each build.
           nif: {:__nif__, 0} in exports,
           exports: exports
         }}
      end

    for {module, file, binary} <- files, module != __MODULE__, not told[module].on_load do
      :code.purge(module)
      {:module, ^module} = :code.load_binary(module, file, binary)
    end

    Map.new(told, fn {module, facts} ->
      {module, Map.merge(facts, %{struct: ran(facts, :__struct__, []), child_spec: ran(facts, :child_spec, [nil])})}
    end)
  end

  # What `module`'s function gives, where it has it and was loaded: `:raised` where it raised.
  defp ran(%{module: module, on_load: false, exports: exports}, name, args) do
    if {name, length(args)} in exports, do: apply(module, name, args)
  rescue
    _ -> :raised
  end

  defp ran(_facts, _name, _args), do: nil

  @doc """
  Loads the build in the folder `build` into this server while it runs, as `one serve run` asks once `compare/2` said
  `hot_load`, and says `Loaded <build>`; or where it can't, leaves this server on the code it had and says
  `Didn't load <build>: <why>`. Going back is the same: loading the build before.

  Each process of its supervision tree whose code changed is stopped first, then the modules that changed are loaded
  all at once or none (`:code.atomic_load/1`), and those it no longer has are deleted, and the processes are started
  again on the new code, so their state is built again as a restart builds it, and nothing needs a `code_change`.
  The children a `Task.Supervisor` runs are calls, which aren't stopped: a call that began on the code before ends on
  it. What a call holds of the code before the one before is purged as this loads, which ends it. A dynamic
  supervisor's children are started whenever they're next asked for, as they are at any start.

  Then the code path, the `.app` and the UI library are the new build's, and the apps built against another UI library
  are built again, as at a start. The VM goes on running from the build it started from, whose runtime is this one's.
  """
  def load(build) do
    case loaded(build) do
      :ok ->
        IO.puts("Loaded #{Builds.build()}")
        :ok

      {:error, why} ->
        IO.puts("Didn't load #{named(build)}: #{why}")
        {:error, why}
    end
  end

  defp loaded(build) do
    was = :code.lib_dir(:one_core) |> to_string() |> Path.join("../..") |> Path.expand()
    swap(changes(was, build), build, fn -> moved(build) end)
  end

  @doc """
  Loads `changes` of One Core's modules, as `modules/2` gives them, from the code `build` into this server while it
  runs, as `load/1` does, running `then` once they're loaded and before its processes start again: `:ok`, or
  `{:error, why}`.
  """
  def swap({removed, added, changed}, build, then \\ fn -> :ok end) do
    stop(MapSet.new(removed ++ changed))
    for module <- changed, do: :code.purge(module)

    loaded =
      case :code.atomic_load(files(added ++ changed, build)) do
        :ok ->
          for module <- removed, do: {:code.purge(module), :code.delete(module)}
          then.()

        {:error, why} ->
          {:error, "its code didn't load: #{inspect(why)}"}
      end

    case {loaded, started(OneCore.Supervisor)} do
      {:ok, []} ->
        :ok

      {:ok, failed} ->
        {:error, Enum.map_join(failed, ", ", fn {id, why} -> "#{inspect(id)} didn't start again: #{said(why)}" end)}

      {error, _failed} ->
        error
    end
  rescue
    error -> {:error, Exception.message(error)}
  end

  # Why a process didn't start, as what it raised says it, or as OTP gives it.
  defp said({%{__exception__: true} = error, _stacktrace}), do: Exception.message(error)
  defp said(why), do: inspect(why)

  # Stops each process in the supervision tree whose module, as it was started, is one of `modules`, as its
  # supervisor stops it. One that a dynamic supervisor runs is started by whoever asks for it next, and the rest
  # are started again once the code is loaded (`started/1`).
  defp stop(modules) do
    for {supervisor, id, pid} <- tree(OneCore.Supervisor),
        {module, _function, _arity} = :proc_lib.translate_initial_call(pid),
        module in modules do
      if id == :undefined,
        do: DynamicSupervisor.terminate_child(supervisor, pid),
        else: :ok = Supervisor.terminate_child(supervisor, id)
    end
  end

  # Every worker in the tree under `supervisor` with its supervisor and id, past those of a `Task.Supervisor`.
  defp tree(supervisor) do
    for {id, pid, type, modules} <- Supervisor.which_children(supervisor), is_pid(pid) do
      if type == :supervisor do
        if modules == [Task.Supervisor], do: [], else: tree(pid)
      else
        [{supervisor, id, pid}]
      end
    end
    |> List.flatten()
  end

  # Starts again each child in the tree under `supervisor` that was stopped, as a load stops them: also one a load
  # before couldn't start, which a load of the build before then does. Gives the ids of those that didn't start, and
  # why.
  defp started(supervisor) do
    for {id, pid, type, modules} <- Supervisor.which_children(supervisor) do
      cond do
        pid == :undefined ->
          case Supervisor.restart_child(supervisor, id) do
            {:error, why} -> [{id, why}]
            _started -> []
          end

        is_pid(pid) and type == :supervisor and modules != [Task.Supervisor] ->
          started(pid)

        true ->
          []
      end
    end
    |> Enum.concat()
  end

  # Has every application's code path, One Core's `.app` and the UI library be `build`'s, and the apps built against
  # another UI library built again, by the server that builds them.
  defp moved(build) do
    for ebin <- Path.wildcard(Path.join(build, "lib/*/ebin")) ++ [consolidated(build)] do
      name = ebin |> Path.dirname() |> Path.basename() |> String.replace(~r/-[^-]*$/, "")
      name = if Path.basename(ebin) == "consolidated", do: "consolidated", else: name
      :code.replace_path(String.to_atom(name), to_charlist(ebin))
    end

    {:application, :one_core, keys} = app(build)

    :ok =
      :application_controller.change_application_data(
        [{:application, :one_core, keys}],
        [{:one_core, Application.get_all_env(:one_core)}]
      )

    Application.put_env(:one_core, :build, named(build))
    Application.put_env(:one_core, :ui, Path.join(build, "ui"))
    :persistent_term.erase({OneCore.Entrypoint, :library})
    Task.Supervisor.start_child(OneCore.Asks, &OneCore.Apps.Deploy.rebuild/0)
    :ok
  end
end
