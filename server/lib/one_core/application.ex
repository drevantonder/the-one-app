defmodule OneCore.Application do
  @moduledoc false
  use Application

  @impl true
  def start(_type, _args) do
    children = [{Task.Supervisor, name: OneCore.Asks}, OneCore.Plugins, OneCore.Server]
    Supervisor.start_link(children, strategy: :one_for_one, name: OneCore.Supervisor)
  end
end
