defmodule Mix.Tasks.Network do
  @shortdoc "Starts a network on this machine, in a folder"
  @moduledoc """
  Starts a whole network on this machine, in a folder, to build or test with:

      mix network <folder>

  The folder's `bucket` is the network's bucket, `server` is where its server keeps its key, and `client` is a
  client's home, told where the server is. Starting again in the same folder carries on with what's there.
  """
  use Mix.Task

  @impl true
  def run([folder]) do
    folder = Path.expand(folder)
    Application.put_env(:one_core, :bucket, {OneCore.Bucket.Folder, Path.join(folder, "bucket")})
    Application.put_env(:one_core, :home, Path.join(folder, "server"))
    Mix.Task.run("app.start")

    client = Path.join(folder, "client")
    File.mkdir_p!(client)
    File.write!(Path.join(client, "server"), OneCore.Server.address())
    Mix.shell().info("A network is running in #{folder}. Its client: ONE_HOME=#{client}")
    System.no_halt(true)
  end

  def run(_), do: Mix.raise("mix network <folder> starts a network in that folder")
end
