defmodule OneCore.Application do
  @moduledoc false
  use Application
  alias OneCore.{Iroh, Jobs, Plugins, Server, Servers, Services}

  # A server is started for its owner, its `:for` setting: a user, a group or the network. One that isn't on
  # their list doesn't start: `{:not_joined, key}` says so, with its key.
  @impl true
  def start(_type, _args) do
    {:ok, key} = Iroh.key(Server.key_file())

    if Servers.joined?(key, Application.get_env(:one_core, :for, "network")) do
      children = [{Task.Supervisor, name: OneCore.Asks}, Plugins, Server, Services, Jobs]
      Supervisor.start_link(children, strategy: :one_for_one, name: OneCore.Supervisor)
    else
      {:error, {:not_joined, key}}
    end
  end
end
