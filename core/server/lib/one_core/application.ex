defmodule OneCore.Application do
  @moduledoc false
  use Application
  alias OneCore.{Iroh, Jobs, Plugins, Server, Servers, Services}
  alias OneCore.Plugins.Deploy

  # A server is started for its owner, its `:for` setting: a user, a group or the network. One that isn't on
  # their list doesn't start: `{:not_joined, key}` says so, with its key. The plugins The One App ships with are
  # the network's, so a server acting for it deploys the ones the network has never had, and those whose
  # latest version a server bundled from other files, once it's started (`OneCore.Plugins.Deploy.deploy_bundled/0`), saying
  # each in the log and to `builds` as it goes. Then every server builds again the apps built against another UI library
  # than the one it ships with (`OneCore.Plugins.Deploy.rebuild/0`), and `OneCore.Plugins.Deploy.await_bundled/0` waits
  # for both.
  @impl true
  def start(_type, _args) do
    # What a crash reports holds what it was working with, which may be a person's tokens: none of it reaches the log.
    OneCore.Redact.install()
    # Plugins tell the time in any zone: `DateTime.new(date, time, "Australia/Sydney")`.
    Calendar.put_time_zone_database(Tz.TimeZoneDatabase)
    {:ok, key} = Iroh.key(Server.key_file())

    if Servers.joined?(key, Application.get_env(:one_core, :for, "network")) do
      children = [{Task.Supervisor, name: OneCore.Asks}, Deploy, Plugins, Server, Services, Jobs]

      with {:ok, _supervisor} = started <-
             Supervisor.start_link(children, strategy: :one_for_one, name: OneCore.Supervisor) do
        shipped = fn ->
          if Servers.here?("network"), do: Deploy.deploy_bundled()
          Deploy.rebuild()
        end

        Supervisor.start_child(OneCore.Supervisor, Supervisor.child_spec({Task, shipped}, id: :bundled))
        started
      end
    else
      {:error, {:not_joined, key}}
    end
  end

  # A server that's stopping, as a system stops one with SIGTERM, first takes nothing new, so whoever asks it asks
  # another, and answers what it took already, before anything of it stops. That's within the time a system gives
  # a program to stop: launchd gives 20 s.
  @impl true
  def prep_stop(state) do
    Server.close(:timer.seconds(15))
    state
  end
end
