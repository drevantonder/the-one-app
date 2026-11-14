defmodule Mix.Tasks.Network do
  @shortdoc "Starts a network's server on this machine, in a folder"
  @moduledoc """
  Starts a server of a network kept in a folder on this machine, to build or test with:

      mix network <folder>

  The folder's `bucket` is the network's bucket, `server` is the server's home, and `client` is a client's home,
  told where its servers are. A new network starts with the plugins The One App ships with, as this repo has
  them. Starting it again in the same folder carries on with what's there.

      mix network <folder> <server>

  starts another server of that network, whose home is the folder's `<server>`. It doesn't start until a server
  of the network has let it in, and says how.

      mix network <folder> --address <address> --first <secret>

  starts a server of a network that has an address (`address/`). Its bucket is the one the signer there signs
  for, and it and its client reach each other through the relay there when they can't directly. `--first` is the
  secret the address was deployed with, which a network's first server shows the first time it starts.

  `./network`, at the root, starts a whole network with this.
  """
  use Mix.Task
  alias OneCore.{Address, Iroh, Plugins, Server, Servers}

  @root Path.expand("../../../..", __DIR__)

  # Until signing in decides it, how long a client's pass to the relay lasts. Each start makes a new one.
  @pass_for :timer.hours(24 * 30)

  @impl true
  def run(args) do
    case OptionParser.parse!(args, strict: [address: :string, first: :string]) do
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
    Application.put_env(:one_core, :home, Path.join(folder, server))
    Application.put_env(:one_core, :shipped, shipped())
    Mix.Task.run("app.config")

    case Application.ensure_all_started(:one_core) do
      {:ok, _started} ->
        :ok

      {:error, {:one_core, {{:not_joined, key}, _start}}} ->
        Mix.raise("""
        This server isn't one of the network's yet. Let it in from one of the network's clients, then start it again:

            one add_server --key #{key}
        """)
    end

    for {name, {:error, why}} <- Plugins.ship(), do: Mix.shell().error("#{name} wasn't deployed: #{why}")

    # A client of a network with an address asks there which servers to reach. Without one, it's told here.
    client = Path.join(folder, "client")
    File.mkdir_p!(client)

    if address do
      {:ok, key} = Iroh.key(Path.join(client, "key"))
      pass = Address.pass(key, System.system_time(:millisecond) + @pass_for)
      File.write!(Path.join(client, "network"), JSON.encode!(%{address: address, pass: pass}))
    else
      File.write!(Path.join(client, "servers"), "[#{Enum.join(Servers.addresses(), ",")}]")
    end

    Mix.shell().info("A server is running in #{folder}: #{Server.key()}. Its client: ONE_HOME=#{client}")
    System.no_halt(true)
  end
end
