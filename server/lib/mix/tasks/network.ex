defmodule Mix.Tasks.Network do
  @shortdoc "Starts a network's server on this machine, in a folder"
  @moduledoc """
  Starts a whole network on this machine, in a folder, to build or test with:

      mix network <folder>

  The folder's `bucket` is the network's bucket, `server` is where its server keeps its key, and `client` is a
  client's home, told where its servers are. Starting again in the same folder carries on with what's there.

      mix network <folder> <server>

  starts another server on the same bucket, which keeps its key in the folder's `<server>`. It acts for the
  network's one user too, as a spare.

      mix network <folder> --address <address>

  starts the server of a network that has an address (`address/`). Its bucket is the one the signer there signs
  for, and it and its client reach each other through the relay there when they can't directly.
  """
  use Mix.Task
  alias OneCore.{Address, Iroh, Server, Servers}

  # The plugins The One App ships with, as this repo has them.
  @shipped Path.expand("../../../../plugins", __DIR__)

  # Until signing in decides it, how long a client's pass to the relay lasts. Each start makes a new one.
  @pass_for :timer.hours(24 * 30)

  @impl true
  def run(args) do
    case OptionParser.parse!(args, strict: [address: :string]) do
      {options, [folder | server]} when length(server) <= 1 ->
        start(Path.expand(folder), List.first(server, "server"), options[:address])

      _ ->
        Mix.raise("mix network <folder> starts a network in that folder")
    end
  end

  defp start(folder, server, address) do
    bucket = if address, do: {OneCore.Bucket.S3, address}, else: {OneCore.Bucket.Folder, Path.join(folder, "bucket")}
    Application.put_env(:one_core, :bucket, bucket)
    Application.put_env(:one_core, :address, address)
    Application.put_env(:one_core, :home, Path.join(folder, server))
    Application.put_env(:one_core, :shipped, @shipped)
    Mix.Task.run("app.start")

    # A server on the list adds another when its user asks it to, which here is whoever starts this one.
    if Server.key() not in Servers.acting() do
      params = %{name: "add_server", arguments: %{key: Server.key()}}
      add = JSON.encode!(%{jsonrpc: "2.0", id: 1, method: "tools/call", params: params})

      Enum.any?(Servers.acting(), &match?({:ok, _}, Server.ask(&1, add))) ||
        Mix.raise("None of this network's servers answered, and one of them has to add this one")
    end

    client = Path.join(folder, "client")
    File.mkdir_p!(client)
    File.write!(Path.join(client, "servers"), "[#{Enum.join(Servers.addresses(), ",")}]")

    if address do
      {:ok, key} = Iroh.key(Path.join(client, "key"))
      pass = Address.pass(key, System.system_time(:millisecond) + @pass_for)
      File.write!(Path.join(client, "network"), JSON.encode!(%{address: address, pass: pass}))
    end

    Mix.shell().info("A server is running in #{folder}: #{Server.key()}. Its client: ONE_HOME=#{client}")
    System.no_halt(true)
  end
end
