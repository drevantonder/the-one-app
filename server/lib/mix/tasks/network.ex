defmodule Mix.Tasks.Network do
  @shortdoc "Starts a network's server on this machine, in a folder"
  @moduledoc """
  Starts a server of a network kept in a folder on this machine, to build or test with:

      mix network <folder>

  The folder's `bucket` is the network's bucket, `server` is the server's home, and `client` is a client's home, told
  where its servers are. `only-server` is a client that asks that one server and no other. A new network starts with
  the plugins The One App ships with, as this repo has them. Starting it again in the same folder carries on with
  what's there.

      mix network <folder> <server>

  starts another server of that network, whose home is the folder's `<server>`. It doesn't start until whoever
  it's for has let it in, and says how.

      mix network <folder> <server> --for users/alice

  starts a server that is that user's, or that group's, and acts for them alone. Without it a server is the
  network's, and acts for everyone in it.

      mix network <folder> --address <address> --first <secret>

  starts a server of a network that has an address (`address/`). Its bucket is the one the signer there signs
  for, and it and its client reach each other through the relay there when they can't directly. `--first` is the
  secret the address was deployed with, which a network's first server shows the first time it starts.

      mix network <folder> --development

  starts a server of a network being built, which lets a request say which user it's from (`OneCore.User`). No
  network anyone relies on is started with it.

      mix network <folder> <server> --without <plugin>

  starts a server after removing that plugin from what's added for whoever the server is for. It's the way back
  for whoever runs one of their servers, when a plugin that decides who may do what has broken and so refuses
  its own removal: nothing is asked of any plugin.

  `./network`, at the root, starts a whole network with this.
  """
  use Mix.Task
  alias OneCore.{Address, Iroh, Plugins, Server, Servers}

  @root Path.expand("../../../..", __DIR__)

  # Until signing in decides it, how long a client's pass to the relay lasts. Each start makes a new one.
  @pass_for :timer.hours(24 * 30)

  @impl true
  def run(args) do
    case OptionParser.parse!(args,
           strict: [address: :string, first: :string, development: :boolean, for: :string, without: :string]
         ) do
      {options, [folder | server]} when length(server) <= 1 ->
        start(Path.expand(folder), List.first(server, "server"), options)

      _ ->
        Mix.raise("mix network <folder> starts a network's server in that folder")
    end
  end

  # The plugins The One App ships with, as this repo has them: each one's folder, and the kit as it's built.
  defp shipped do
    plugins = Map.new(Path.wildcard(Path.join(@root, "plugins/*")), &{Path.basename(&1), &1})
    Map.put(plugins, "ui", Path.join(@root, "ui/dist"))
  end

  defp start(folder, server, options) do
    address = options[:address]
    bucket = if address, do: {OneCore.Bucket.S3, address}, else: {OneCore.Bucket.Folder, Path.join(folder, "bucket")}
    Application.put_env(:one_core, :bucket, bucket)
    Application.put_env(:one_core, :address, address)
    Application.put_env(:one_core, :first, options[:first])
    Application.put_env(:one_core, :development, options[:development] == true)
    Application.put_env(:one_core, :for, options[:for] || "network")
    Application.put_env(:one_core, :home, Path.join(folder, server))
    Application.put_env(:one_core, :shipped, shipped())
    Mix.Task.run("app.config")

    case Application.ensure_all_started(:one_core) do
      {:ok, _started} ->
        :ok

      {:error, {:one_core, {{:not_joined, key}, _start}}} ->
        Mix.raise("""
        This server isn't one of #{options[:for] || "the network"}'s yet. Whoever owns them lets it in from a client,
        asking for them (ONE_FOR=#{options[:for] || "network"}), and then it's started again:

            one add_server --key #{key}
        """)
    end

    if without = options[:without] do
      case Plugins.remove(options[:for] || "network", without) do
        :ok -> Mix.shell().info("#{without} is removed from what's added for #{options[:for] || "the network"}.")
        {:error, why} -> Mix.shell().error("#{without} wasn't removed: #{why}")
      end
    end

    # The plugins The One App ships with are the network's, so a server acting for it deploys them.
    if Servers.here?("network"),
      do: for({name, {:error, why}} <- Plugins.ship(), do: Mix.shell().error("#{name} wasn't deployed: #{why}"))

    # A client of a network with an address asks there which servers to reach. Without one, it's told here: the
    # ones acting for the network, which is everyone. A server's pass for a client says the server's list, so the
    # one a network's own server gives lets a client in whoever its user is.
    client = Path.join(folder, "client")
    File.mkdir_p!(client)

    cond do
      not Servers.here?("network") ->
        :ok

      address ->
        {:ok, key} = Iroh.key(Path.join(client, "key"))
        pass = Address.pass(key, System.system_time(:millisecond) + @pass_for)
        File.write!(Path.join(client, "network"), JSON.encode!(%{address: address, pass: pass}))

      true ->
        addresses = for key <- Servers.listed("network"), {:ok, address} <- [Servers.address(key)], do: address
        File.write!(Path.join(client, "servers"), "[#{Enum.join(addresses, ",")}]")
    end

    # A client that asks this server and no other, to test with.
    only = Path.join(folder, "only-" <> server)
    File.mkdir_p!(only)
    {:ok, reached} = Servers.address(Server.key())
    File.write!(Path.join(only, "servers"), "[#{reached}]")

    Mix.shell().info("A server is running in #{folder}: #{Server.key()}. Its client: ONE_HOME=#{client}")
    System.no_halt(true)
  end
end
