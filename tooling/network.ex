defmodule Mix.Tasks.Network do
  @shortdoc "Starts a network's server on this machine, in a folder"
  @moduledoc """
  Starts a server of a network kept in a folder on this machine, to build or test with:

      mix network <folder>

  The folder's `bucket` is the network's bucket, `server` is the server's home, and `client` is a client's home, told
  where its servers are. `only-server` is a client that asks that one server and no other. Starting it again in the
  same folder carries on with what's there.

      mix network <folder> <server>

  starts another server of that network, whose home is the folder's `<server>`. It doesn't start until whoever
  it's for has let it in, and says how.

      mix network <folder> <server> --subject users/alice

  starts a server that is that user's, or that group's, and acts for them alone. Without it a server is the
  network's, and acts for everyone in it.

      mix network <folder> <server> --name <name>

  calls the server that name, where it would otherwise be called what its machine is: a server says so to the
  others, and apps name a machine by it. Two servers on one machine each need one.

      mix network <folder> --address <address> --first <secret>

  starts a server of a network that has an edge (`edge/`). Its bucket is the one the presigner there signs
  for, and it and its client reach each other through the relay there when they can't directly. `--first` is the
  secret the edge was deployed with, which a network's first server shows the first time it starts.

      mix network <folder> --owner <name>

  makes the user of that name the network's first owner, which a network gets once (`OneCore.User`), and where
  people sign in, says where they join. Before it has one, every app of this repo is deployed for the network, as
  their author publishes them, so it has them to install: making its first owner installs the ones a network starts
  with. After that, an app changed here is deployed into it as any is (`one deploy`).

      mix network <folder> --development

  starts a server of a network being built: nobody signs in, a request says which user it's from, and where no
  app decides who may do what, everything is let through (`OneCore.User`, `OneCore.Hooks`). No network anyone
  relies on is started with it.

      mix network <folder> --reloader <node>

  has the server ask the code reloader running as that node, by its short name, before it answers a client's request,
  which loads what changed in this checkout into it (`tooling/code_reloader.exs`).

      mix network <folder> <server> --without <app>

  starts a server after removing that app from what's installed for whoever the server is for. It's the way back
  for whoever runs one of their servers, when an app that decides who may do what has broken and so refuses
  its own removal: nothing is asked of any app. With none left deciding, nobody does anything but the
  network's first owner, who puts it back.

  `tooling/network`, at the root, starts a whole network with this.
  """
  use Mix.Task
  require Logger
  alias OneCore.{Folder, Server, Servers, User}
  alias OneCore.Apps.Deploy

  @root Path.expand("..", __DIR__)

  @impl true
  def run(args) do
    case OptionParser.parse!(args,
           strict: [
             address: :string,
             first: :string,
             development: :boolean,
             subject: :string,
             without: :string,
             owner: :string,
             name: :string,
             reloader: :string
           ]
         ) do
      {options, [folder | server]} when length(server) <= 1 ->
        start(Path.expand(folder), List.first(server, "server"), options)

      _ ->
        Mix.raise("mix network <folder> starts a network's server in that folder")
    end
  end

  # Deploys each of this repo's apps for the network from its folder, as `one deploy --files-from` sends it,
  # installing none: each is the network's to install.
  defp publish do
    for folder <- Enum.sort(Path.wildcard(Path.join(@root, "apps/*"))),
        name = Path.basename(folder),
        {:ok, paths} <- [Folder.files(folder)] do
      files = Map.new(paths, &{&1, File.read!(Path.join(folder, &1))})

      case Deploy.deploy("network", name, files, nil, true, fn _version -> false end) do
        {:ok, _number, _warnings} -> Logger.info("#{name} is deployed for the network")
        {:error, why} -> Logger.error("#{name} wasn't deployed for the network: #{why}")
      end
    end
  end

  defp start(folder, server, options) do
    address = options[:address]
    bucket = if address, do: {OneCore.Bucket.S3, address}, else: {OneCore.Bucket.Folder, Path.join(folder, "bucket")}
    Application.put_env(:one_core, :bucket, bucket)
    Application.put_env(:one_core, :address, address)
    Application.put_env(:one_core, :first, options[:first])
    Application.put_env(:one_core, :development, options[:development] == true)
    Application.put_env(:one_core, :owner, options[:subject] || "network")
    Application.put_env(:one_core, :home, Path.join(folder, server))
    Application.put_env(:one_core, :name, options[:name])
    Application.put_env(:one_core, :ui, Path.join(@root, "core/ui/dist"))
    Mix.Task.run("app.config")

    case Application.ensure_all_started(:one_core) do
      {:ok, _started} ->
        :ok

      {:error, {:one_core, {{:not_joined, key}, _start}}} ->
        Mix.raise("""
        This server isn't one of #{options[:subject] || "the network"}'s yet. Whoever owns them lets it in from a client,
        asking for them (ONE_SUBJECT=#{options[:subject] || "network"}), and then it's started again:

            one add_server --key #{key}
        """)
    end

    if without = options[:without] do
      owner = options[:subject] || "network"

      case OneCore.Apps.Lifecycle.uninstall(owner, without, {nil, owner}) do
        %{} ->
          Mix.shell().info("#{without} is removed from what's installed for #{options[:subject] || "the network"}.")

        {:error, why} ->
          Mix.shell().error("#{without} wasn't removed: #{why}")
      end
    end

    if owner = options[:owner] do
      if User.first_owner() == nil and Servers.here?("network"), do: publish()

      case OneCore.Apps.Lifecycle.make_first_owner("users/" <> owner) do
        :ok -> Mix.shell().info("users/#{owner} is the network's first owner.")
        {:error, why} -> Mix.shell().error("users/#{owner}, the network's first owner: #{why}")
      end
    end

    # A client of a network with an address asks there which servers to reach, and gets its token by signing in
    # (`tooling/network` signs a network being built's in as its first owner). Without an address, a client is told
    # here where the servers acting for the network are.
    client = Path.join(folder, "client")
    File.mkdir_p!(client)

    cond do
      not Servers.here?("network") ->
        :ok

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

    if reloader = options[:reloader] do
      [_name, host] = String.split(Atom.to_string(node()), "@")
      reloader = String.to_atom("#{reloader}@#{host}")
      Application.put_env(:one_core, :reloader, {:erpc, :call, [reloader, CodeReloader, :reload, [node()], :infinity]})
      :ok = :erpc.call(reloader, CodeReloader, :serving, [node()], :infinity)
    end

    Mix.shell().info("A server is running in #{folder}: #{Server.key()}. Its client: ONE_HOME=#{client}")
    System.no_halt(true)
  end
end
