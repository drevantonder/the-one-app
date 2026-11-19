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
  alias OneCore.Apps.{Calls, Installed, Store, Version}

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
  Who the service that asks runs for on its server, each as `app/4` takes it: everyone its app was installed
  for, a user, a group or the network, whose apps its server runs (`OneCore.Apps.runs?/1`), and where it keeps
  each person's data apart, each who keeps something with it (`OneCore.Apps.Installed.subjects/2`). Whoever the server
  is for comes first, as its owner chose them, so a service that's for one of them at a time, its machine's person,
  takes the first.
  """
  def acts_for do
    {app, _name, _version} = which(self())
    members = Servers.members(Server.key())

    whos =
      for at <- Apps.installed_for(app),
          Apps.runs?(at),
          {name, %Version{artifact: ^app} = version} <- Apps.versions([at]),
          who <- [at | if(version.own == :subject, do: Installed.subjects(at, name), else: [])],
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
