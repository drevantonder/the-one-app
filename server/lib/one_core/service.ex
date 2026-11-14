defmodule OneCore.Service do
  @moduledoc """
  The interface a plugin's service is written against. `OneCore.Plugin.guide/0` says how to write one.

  A service is a module of its plugin that says `use GenServer` and `@behaviour OneCore.Service`. Its process runs
  on each server whose owner installed it, and its public functions are what the rest of its plugin calls there.
  """
  alias OneCore.{Call, Plugins, Server, Services}
  alias OneCore.Plugins.Version

  @doc "What the service needs of a machine, for its owner to read before installing it."
  @callback needs() :: String.t()

  @doc """
  Calls a function of a service of this call's plugin, on a machine that runs it, and gives its answer. The
  function is called with the service's process and then `arguments`. Arguments and answer are what JSON can hold,
  as they arrive wherever the service runs.

  `server` is the key of the server to call it on. Left out, it's this server if the service runs here, and
  otherwise the first of `servers/2` that is up.

  Raises with what went wrong: the function raised, no server runs the service, or the one that does isn't up.
  """
  def call(%Call{} = call, service, function, arguments \\ [], server \\ nil) when is_list(arguments) do
    service = Version.own_name(service)
    servers = if server, do: [server], else: Services.servers(call.who, service)

    if servers == [] do
      raise "No server runs #{call.plugin}'s #{service}. `services` says what it needs, and `install_service` runs it on one."
    end

    ask = %{plugin: call.who, service: service, function: "#{function}", arguments: json(arguments)}

    case ask(servers, call.server, ask) do
      {:ok, answer} -> answer
      {:error, why} -> raise why
    end
  end

  @doc "The keys of the servers that run a service of this call's plugin, for a plugin that chooses which to call."
  def servers(%Call{who: plugin}, service), do: Services.servers(plugin, Version.own_name(service))

  @doc """
  Calls a function of the service's plugin, for the user or group `who`, asked by nobody, and gives its answer.
  `service` is the
  service's process: `self()` in its callbacks, and what its functions are called with. The function is called
  with an `OneCore.Call` for `who` and then `arguments`, on a server acting for them, which this server must be:
  one on the network's one list acts for every user in it.
  """
  def plugin(service, who, function, arguments \\ []) when is_pid(service) and is_list(arguments) do
    {plugin, _name, _version} = which(service)
    if Services.acts_for() == [], do: raise("This server doesn't act for #{who}")

    {added, name} =
      Plugins.added(who, plugin) || raise "#{plugin} isn't added for #{who}, or for the network"

    answer =
      case Plugins.function(added, name, "#{function}", json(arguments), {nil, who}) do
        {:ok, answer} -> json(answer)
        {:passed_on, %{"answer" => answer}} -> answer
        {:error, why} -> raise why
      end

    Server.changed()
    answer
  end

  @doc "The users and groups a service's server acts for, each as `plugin/4` takes it."
  def acts_for, do: Services.acts_for()

  @doc "The key of the server a service runs on, as `call/5` takes it."
  def server, do: Server.key()

  @doc """
  A folder on this machine holding the plugin's files under `path`, by their paths within it, for a program the
  service brings. It's the service's own to build in, and to start the program from.

  The folder stays the same for as long as those files do. Once they change it's a new one, and the one before is
  removed. So a service that kept the folder it started its program from can tell, in `code_change/3`, whether
  the program changed.
  """
  def folder(service, path) when is_pid(service) do
    {plugin, name, version} = which(service)
    files = Enum.sort(for {file, hash} <- version.hashes, String.starts_with?(file, path <> "/"), do: {file, hash})
    if files == [], do: raise("#{plugin} has no files under #{path}/")

    within = Path.join(Services.folder(plugin, name), URI.encode_www_form(path))
    folder = Path.join(within, binary_part(Version.code(files), 0, 16))

    # The files are written beside the folder and moved in whole, so a folder that's there has all of them.
    if not File.dir?(folder) do
      writing = folder <> ".writing"
      File.rm_rf!(writing)

      for {file, _hash} <- files do
        {:ok, text} = Plugins.file(plugin, version.code, file)
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
    Plugins.Process.which(service) || raise ArgumentError, "#{inspect(service)} isn't a service's process"
  end

  # What arrives after going through JSON, which is how it reaches another server.
  defp json(term), do: JSON.decode!(JSON.encode!(term))

  # Asks each server in turn until one is up. A server that couldn't be reached was asked nothing.
  defp ask([], _here, %{plugin: plugin, service: service}) do
    {:error, "The server that runs #{plugin}'s #{service} isn't up"}
  end

  defp ask([here | _], here, %{plugin: plugin, service: service, function: function, arguments: arguments}) do
    with {:ok, answer} <- Services.run(plugin, service, function, arguments), do: {:ok, json(answer)}
  end

  defp ask([server | servers], here, ask) do
    case Server.request(server, "services/call", ask) do
      {:ok, %{"answer" => answer}} -> {:ok, answer}
      {:error, _code, why} -> {:error, why}
      :unreachable -> ask(servers, here, ask)
      :stopped -> {:error, "The server that runs #{ask.plugin}'s #{ask.service}: " <> Server.said(:stopped)}
    end
  end
end
