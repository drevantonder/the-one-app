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

      mix network <folder> <server> --name <name>

  calls the server that name, where it would otherwise be called what its machine is: a server says so to the
  others, and plugins name a machine by it. Two servers on one machine each need one.

      mix network <folder> --address <address> --first <secret>

  starts a server of a network that has an edge (`edge/`). Its bucket is the one the signer there signs
  for, and it and its client reach each other through the relay there when they can't directly. `--first` is the
  secret the edge was deployed with, which a network's first server shows the first time it starts.

      mix network <folder> --owner <name>

  makes the user of that name the network's first owner, which a network gets once (`OneCore.User`), and where
  people sign in, says where they join.

      mix network <folder> --development

  starts a server of a network being built: nobody signs in, a request says which user it's from, and where no
  plugin decides who may do what, everything is let through (`OneCore.User`, `OneCore.Hooks`). No network anyone
  relies on is started with it.

      mix network <folder> <server> --without <plugin>

  starts a server after removing that plugin from what's added for whoever the server is for. It's the way back
  for whoever runs one of their servers, when a plugin that decides who may do what has broken and so refuses
  its own removal: nothing is asked of any plugin. With none left deciding, nobody does anything but the
  network's first owner, who puts it back.

  `./network`, at the root, starts a whole network with this.
  """
  use Mix.Task
  alias OneCore.{Edge, Iroh, Server, Servers, User}
  alias OneCore.Plugins.Added

  @root Path.expand("../../../..", __DIR__)

  @impl true
  def run(args) do
    case OptionParser.parse!(args,
           strict: [
             address: :string,
             first: :string,
             development: :boolean,
             for: :string,
             without: :string,
             owner: :string,
             name: :string
           ]
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
    Application.put_env(:one_core, :name, options[:name])
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
      case Added.remove(options[:for] || "network", without) do
        :ok -> Mix.shell().info("#{without} is removed from what's added for #{options[:for] || "the network"}.")
        {:error, why} -> Mix.shell().error("#{without} wasn't removed: #{why}")
      end
    end

    # The plugins are there first, so the ones that keep who may do what and who signs in are told who the first
    # owner is, and the server's log says where they join.
    if owner = options[:owner] do
      case User.make_first_owner("users/" <> owner) do
        :ok -> Mix.shell().info("users/#{owner} is the network's first owner.")
        {:error, why} -> Mix.shell().error("users/#{owner} wasn't made the network's first owner: #{why}")
      end
    end

    # A client of a network with an address asks there which servers to reach, and gets its pass by signing in:
    # one of a network being built is given its pass here, since nobody signs in to that. A server's pass for a
    # client says the server's list, so the one a network's own server gives lets a client in whoever its user
    # is. Without an address, a client is told here where the servers acting for the network are.
    client = Path.join(folder, "client")
    File.mkdir_p!(client)

    cond do
      not Servers.here?("network") ->
        :ok

      address && options[:development] ->
        {:ok, key} = Iroh.key(Path.join(client, "key"))
        File.write!(Path.join(client, "network"), JSON.encode!(%{address: address, pass: Edge.pass(key)}))

      address ->
        File.write(Path.join(client, "network"), JSON.encode!(%{address: address}), [:exclusive])

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
