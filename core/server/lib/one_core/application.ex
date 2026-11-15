defmodule OneCore.Application do
  @moduledoc false
  use Application
  alias OneCore.{Iroh, Jobs, Plugins, Server, Servers, Services}
  alias OneCore.Plugins.Deploy

  # A server is started for its owner, its `:for` setting: a user, a group or the network. One that isn't on
  # their list doesn't start: `{:not_joined, key}` says so, with its key. The plugins The One App ships with are
  # the network's, so a server acting for it deploys the ones the network has never had, and those whose
  # newest version a server shipped from other files, once it's started (`OneCore.Plugins.Deploy.ship/0`), saying
  # each in the log and to `builds` as it goes.
  @impl true
  def start(_type, _args) do
    # Plugins tell the time in any zone: `DateTime.new(date, time, "Australia/Sydney")`.
    Calendar.put_time_zone_database(Tz.TimeZoneDatabase)
    {:ok, key} = Iroh.key(Server.key_file())

    if Servers.joined?(key, Application.get_env(:one_core, :for, "network")) do
      children = [{Task.Supervisor, name: OneCore.Asks}, Deploy, Plugins, Server, Services, Jobs]

      with {:ok, _supervisor} = started <-
             Supervisor.start_link(children, strategy: :one_for_one, name: OneCore.Supervisor) do
        if Servers.here?("network"), do: Task.Supervisor.start_child(OneCore.Asks, &Deploy.ship/0)

        started
      end
    else
      {:error, {:not_joined, key}}
    end
  end
end
