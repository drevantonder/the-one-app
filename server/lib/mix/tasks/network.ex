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
  """
  use Mix.Task
  alias OneCore.{Server, Servers}

  # The plugins The One App ships with, as this repo has them.
  @shipped Path.expand("../../../../plugins", __DIR__)

  @impl true
  def run([folder | server]) when length(server) <= 1 do
    folder = Path.expand(folder)
    Application.put_env(:one_core, :bucket, {OneCore.Bucket.Folder, Path.join(folder, "bucket")})
    Application.put_env(:one_core, :home, Path.join(folder, List.first(server, "server")))
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
    Mix.shell().info("A server is running in #{folder}: #{Server.key()}. Its client: ONE_HOME=#{client}")
    System.no_halt(true)
  end

  def run(_), do: Mix.raise("mix network <folder> starts a network in that folder")
end
