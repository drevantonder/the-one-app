defmodule OneCore.Application do
  @moduledoc false
  use Application
  alias OneCore.{Iroh, Plugins, Server, Servers, Services}

  # A server is told when another puts something in the bucket for it to run. It's told nothing while it can't be
  # reached, and nothing says when it can be again, so it also looks by itself this often, in milliseconds.
  @look_every :timer.minutes(1)

  # A server that isn't one of its network's doesn't start: `{:not_joined, key}` says so, with its key.
  @impl true
  def start(_type, _args) do
    {:ok, key} = Iroh.key(Server.key_file())

    if Servers.joined?(key) do
      looks = Supervisor.child_spec({Task, &look/0}, restart: :permanent)
      children = [{Task.Supervisor, name: OneCore.Asks}, Plugins, Server, Services, looks]
      Supervisor.start_link(children, strategy: :one_for_one, name: OneCore.Supervisor)
    else
      {:error, {:not_joined, key}}
    end
  end

  defp look do
    Process.sleep(@look_every)
    Server.look()
    look()
  end
end
