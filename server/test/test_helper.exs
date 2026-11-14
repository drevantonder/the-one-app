# What a server logs is shown with the test that failed, and otherwise not at all.
ExUnit.start(capture_log: true)

defmodule OneCore.Test do
  @doc """
  Starts a network's first server for the test that calls this, kept in `folder`: its bucket there, its home,
  and what answers and runs plugins. It acts for the network, so for everyone.
  """
  def server(folder) do
    Application.put_env(:one_core, :bucket, {OneCore.Bucket.Folder, Path.join(folder, "bucket")})
    Application.put_env(:one_core, :home, Path.join(folder, "server"))
    {:ok, key} = OneCore.Iroh.key(OneCore.Server.key_file())
    true = OneCore.Servers.joined?(key, "network")
    for child <- [OneCore.Plugins, OneCore.Server, OneCore.Services], do: ExUnit.Callbacks.start_supervised!(child)
    key
  end
end
