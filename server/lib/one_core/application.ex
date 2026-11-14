defmodule OneCore.Application do
  @moduledoc false
  use Application

  @impl true
  def start(_type, _args) do
    {:ok, key} = OneCore.Iroh.key(OneCore.Server.key_file())
    OneCore.Servers.first(key)

    children = [{Task.Supervisor, name: OneCore.Asks}, OneCore.Plugins, OneCore.Server, OneCore.Services]
    Supervisor.start_link(children, strategy: :one_for_one, name: OneCore.Supervisor)
  end
end
