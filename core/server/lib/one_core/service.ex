defmodule OneCore.Service do
  @moduledoc """
  The interface an app's service is written against. `OneCore.App` says how to write one.

  A service is a module of its app that says `@behaviour OneCore.Service`, and `use GenServer` or, when it has
  stages, `@behaviour :gen_statem`. Its process runs on each server whose owner installed it, and its public
  functions are what the rest of its app calls there.

  One no longer to run on its server, removed from it or its app uninstalled, stops with `{:shutdown, :removed}`,
  which its `terminate/2` is given while its folder is still there, to remove what it put on the machine.
  """
  alias OneCore.{Call, Apps, Server, Servers, Services}
  alias OneCore.Apps.{Calls, Store, Version}

  @doc "What the service needs of a machine, for its owner to read before installing it."
  @callback needs() :: String.t()

  @doc "What the service is called, in words its owner reads: its app's name, and what it is of it."
  @callback called() :: String.t()

  @doc """
  The operating systems the service runs on, as npm's `os` field in package.json names them (`"darwin"`, `"linux"`,
  `"win32"`). Left out, it runs on any. Settings shows it only on servers of these, and a server of another refuses it.
  """
  @callback os() :: [String.t()]
  @optional_callbacks os: 0

  @doc """
  Calls a function of a service of this call's app, on a machine that runs it, and gives its answer. The
  function is called with the service's process and then `arguments`. Arguments and answer are what JSON can hold,
  as they arrive wherever the service runs.

  `server` is the key of the server to call it on. Left out, it's this server if the service runs here, and
  otherwise the first of `servers/2` that is up.

  Raises with what went wrong: the function raised, or no server runs the service. Where the servers that run it
  aren't up, as a Mac that's asleep, it raises `OneCore.Call.Refused`: that's what was asked needing a device that's
  off, not something failing.
  """
  def call(%Call{} = call, service, function, arguments \\ [], server \\ nil) when is_list(arguments) do
    app = whose(call, service)
    service = Version.own_name(service)
    servers = if server, do: [server], else: Services.servers(app, service, call.subject)

    if servers == [] do
      raise "None of your network's computers can do this yet. Ask your network's admin to set one up.\n\nNo server runs #{service}: `services` says what it needs, and `install_service` runs it on one."
    end

    meta = %{actor: call.actor, subject: call.subject, forwarded: true}

    ask = %{
      app: app,
      service: service,
      function: "#{function}",
      arguments: OneCore.JSON.normalize(arguments),
      _meta: meta
    }

    case ask(servers, call.server, ask) do
      {:ok, response} -> response
      {:off, why} -> raise Call.Refused, why
      {:error, why} -> raise why
    end
  end

  @doc "The keys of the servers that run a service of this call's app, for an app that chooses which to call."
  def servers(%Call{subject: subject} = call, service),
    do: Services.servers(whose(call, service), Version.own_name(service), subject)

  @doc "What the machine a server runs on is called, by the server's key: its name, which the server says as it starts."
  def name(server), do: Servers.name(server)

  @doc """
  When the machine a server runs on was last seen, by the server's key, in milliseconds since 1970, if it's stopped:
  `nil` while it's on, or where that isn't known. `call/5` doesn't wait for one that's stopped, and says when it was
  last seen.
  """
  def seen(server), do: (Servers.presence(server) || %{})["last_seen"]

  @doc """
  The machines that run a service of this call's app, as an app's `machines` tool answers them: each by its name,
  whether it's on, and when it was last seen where it's off.
  """
  def machines(%Call{} = call, service) do
    for server <- servers(call, service) do
      case seen(server) do
        at when is_integer(at) -> %{machine: name(server), on: false, last_on: at}
        _ -> %{machine: name(server), on: true}
      end
    end
  end

  # How long each server has to answer `each/4`, in milliseconds: short of the ten seconds an app has to answer a hook
  # (`OneCore.Hooks.within/3`), so a hook that asks every server still has what the others said when one doesn't.
  @each_deadline 8_000

  @doc """
  Calls `function` of a service of this call's app, as `call/5` does, on each of the servers running it that's on,
  all at once, each under its own deadline of eight seconds, as `Promise.allSettled` does. Gives `{answers, failed}`:
  `answers` is each of what they gave with its server's key, `{server, item}`, for a function that gives a list: what
  each machine has; and `failed` is why each server that couldn't answer couldn't, by its machine's name, as it raised
  or didn't answer in time. One that doesn't answer takes away only its own part. A machine that's off has nothing to
  give now: `machines/2` says which.
  """
  def each(%Call{} = call, service, function, arguments \\ []) do
    servers = for server <- servers(call, service), not is_integer(seen(server)), do: server
    {trace, id} = {OpenTelemetry.Ctx.get_current(), Call.State.id()}

    asked = fn server ->
      OpenTelemetry.Ctx.attach(trace)
      Call.State.follow(id)

      try do
        {:ok, call(call, service, function, arguments, server)}
      rescue
        problem -> {:error, Exception.message(problem)}
      end
    end

    results =
      servers
      |> Task.async_stream(asked, max_concurrency: max(length(servers), 1), timeout: @each_deadline, on_timeout: :kill_task)
      |> Enum.zip(servers)

    answers = for {{:ok, {:ok, items}}, server} <- results, item <- items, do: {server, item}

    failed =
      for {result, server} <- results, why = why(result), into: %{}, do: {name(server) || server, why}

    {answers, failed}
  end

  @doc """
  What to add to saying that something isn't there, where `failed` (from `each/4`) holds the machines that didn't
  answer, as it may be on one of them: `""` where none.
  """
  def unanswered(failed) when failed == %{}, do: ""
  def unanswered(failed), do: ", and #{Enum.map_join(failed, ", ", fn {machine, why} -> "#{machine} didn't answer: #{why}" end)}"

  defp why({:ok, {:ok, _items}}), do: nil
  defp why({:ok, {:error, why}}), do: why
  defp why({:exit, :timeout}), do: "didn't answer within #{div(@each_deadline, 1000)} seconds"

  @doc "The key of the server running a service of this call's app on the machine called `name`, whatever its case."
  def machine!(%Call{} = call, service, name) do
    servers = servers(call, service)

    Enum.find(servers, &(String.downcase(name(&1) || "") == String.downcase(name))) ||
      raise "No machine called #{name} runs #{service.called()}. They're on #{Enum.map_join(servers, ", ", &name/1)}."
  end

  @doc """
  Calls the service's `stop_idle(service, idle)` on each of its machines that's on, as `each/4` does, which stops what
  it runs there that's gone unused for `idle` ms (`used/1`) and gives their names. Gives `{stopped, failed}`: each
  name with its machine, and why each machine that couldn't answer couldn't.
  """
  def stop_idle(%Call{} = call, service, idle) do
    {stopped, failed} = each(call, service, :stop_idle, [idle])
    {for({server, name} <- stopped, do: %{machine: name(server), name: name}), failed}
  end

  @doc """
  Says the service's `id`, one of what it runs, was used now, as a file's time in its folder, `used/<id>`, which
  `used/1` reads.
  """
  def touch(service, id) when is_pid(service) do
    folder = Path.join(folder(service), "used")
    File.mkdir_p!(folder)
    File.touch!(Path.join(folder, id))
  end

  @doc "When each of what the service runs was last used, by its id, in milliseconds since 1970, as `touch/2` said."
  def used(service) when is_pid(service) do
    folder = Path.join(folder(service), "used")

    for file <- if(File.dir?(folder), do: File.ls!(folder), else: []), into: %{} do
      {file, File.stat!(Path.join(folder, file), time: :posix).mtime * 1000}
    end
  end

  @doc """
  Tells every client watching the service's app that what it shows changed, by calling the app's `changed(call)`
  for the first of `acts_for/0`, which says so with `OneCore.Call.changed/1`, in a process of its own.
  """
  def changed(service) when is_pid(service) do
    for who <- Enum.take(acts_for(), 1), do: Task.start(fn -> app(service, who, :changed) end)
    :ok
  end

  # The app whose service it is: this call's, or one whose code runs inside it, whose function this call is running.
  defp whose(%Call{app_artifact: app, loaded: loaded}, service) do
    under = Version.under(service)
    Enum.find_value(loaded, app, fn {_name, {other, module}} -> Version.under(module) == under && other end)
  end

  @doc """
  Calls a function of the service's app, for the user or group `who`, asked by nobody, and gives its answer.
  `service` is the
  service's process: `self()` in its callbacks, and what its functions are called with. The function is called
  with an `OneCore.Call` for `who` and then `arguments`, on a server acting for whoever the app was installed for.
  Nobody is told it ran: a function that changes what a screen shows says so itself (`OneCore.Call.changed/1`).
  """
  def app(service, who, function, arguments \\ []) when is_pid(service) and is_list(arguments) do
    {app, _name, _version} = which(service)

    {installed, name} =
      Apps.installed(who, app) || raise "#{app} isn't installed for #{who}, or for the network"

    response =
      case Calls.function(installed, name, "#{function}", OneCore.JSON.normalize(arguments), {nil, who}) do
        {:ok, response} -> OneCore.JSON.normalize(response)
        {:forwarded, %{"answer" => response}} -> response
        {:error, why} -> raise why
      end

    response
  end

  @doc """
  Who the service that asks runs for on its server, each as `app/4` takes it, as the one rule has a call run
  (`OneCore.Apps.runs?/3`): whoever its server acts for, of everyone its app was installed for, a user, a group or the
  network, and where it keeps each person's data apart, whoever the server is for. Whoever the server is for comes first, as its
  owner chose them, so a service that's for one of them at a time, its machine's person, takes the first.
  """
  def acts_for do
    {app, _name, _version} = which(self())
    members = Servers.members(Server.key())

    whos =
      for at <- Apps.installed_for(app),
          {_name, %Version{artifact: ^app} = version} <- Apps.versions([at]),
          who <- Apps.runs_for(at, version.own),
          Apps.runs?(at, version.trusted, who),
          uniq: true,
          do: who

    Enum.sort_by(whos, &(Enum.find_index(members, fn member -> member == &1 end) || length(members)))
  end


  @doc "The key of the server a service runs on, as `call/5` takes it."
  def server, do: Server.key()

  @doc """
  A folder on this machine that is the service's own, for what it keeps there. It stays for as long as the service
  is installed on this server, through its app's updates, and goes when it's removed. It also holds the folders
  `folder/2` gives, each named after its path.
  """
  def folder(service) when is_pid(service) do
    {app, name, _version} = which(service)
    folder = Services.folder(app, name)
    File.mkdir_p!(folder)
    folder
  end

  @doc """
  A folder on this machine holding the app's files under `path`, by their paths within it, for a program the
  service brings. It's the service's own to build in, and to start the program from.

  The folder stays the same for as long as those files do. Once they change it's a new one, and the one before is
  removed. So a service that kept the folder it started its program from can tell, in `code_change/3`, whether
  the program changed.
  """
  def folder(service, path) when is_pid(service) do
    {app, name, version} = which(service)

    files =
      Enum.sort(for {file, hash} <- Version.hashes(version), String.starts_with?(file, path <> "/"), do: {file, hash})

    if files == [], do: raise("#{app} has no files under #{path}/")

    within = Path.join(Services.folder(app, name), path)
    folder = Path.join(within, binary_part(Version.code(files), 0, 16))

    # The files are written beside the folder and moved in whole, so a folder that's there has all of them.
    if not File.dir?(folder) do
      writing = folder <> ".writing"
      File.rm_rf!(writing)

      for {file, _hash} <- files do
        {:ok, text} = Store.file(app, version.code, file)
        written = Path.join(writing, Path.relative_to(file, path))
        File.mkdir_p!(Path.dirname(written))
        File.write!(written, text)
      end

      File.rename!(writing, folder)
    end

    for other <- File.ls!(within), other != Path.basename(folder), do: File.rm_rf!(Path.join(within, other))
    folder
  end

  defp which(service) do
    Apps.Process.which(service) || raise ArgumentError, "#{inspect(service)} isn't a service's process"
  end

  # What arrives after going through JSON, which is how it reaches another server.
  # Asks each server in turn until one is ready, and then the first that's back of those that were starting or
  # stopping. A server that couldn't be reached was asked nothing.
  defp ask(servers, here, ask, down \\ [])

  defp ask([], here, %{app: app, service: service} = ask, down) do
    case Servers.back(down) do
      nil ->
        why = Enum.map_join(down, ", ", &Server.error_message(:unreachable, &1))
        {:off, "No server that runs #{app}'s #{service} is up: " <> why}

      back ->
        ask([back], here, ask, down -- [back])
    end
  end

  defp ask([here | _], here, %{app: app, service: service, function: function, arguments: arguments}, _down) do
    with {:ok, response} <- Services.run(app, service, function, arguments), do: {:ok, OneCore.JSON.normalize(response)}
  end

  defp ask([server | servers], here, ask, down) do
    case Server.request(server, "services/call", ask) do
      {:ok, %{"answer" => response}} -> {:ok, response}
      {:error, _code, why} -> {:error, why}
      :unreachable -> ask(servers, here, ask, down ++ [server])
      :stopped -> {:off, Server.error_message(:stopped, server)}
    end
  end
end
