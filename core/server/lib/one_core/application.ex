defmodule OneCore.Application do
  @moduledoc false
  use Application
  alias OneCore.{Iroh, Jobs, Plugins, Server, Servers, Services}
  alias OneCore.Plugins.Deploy

  # A server is started for its owner, its `:owner` setting: a user, a group or the network. One that isn't on
  # their list doesn't start: `{:not_joined, key}` says so, with its key. The plugins The One App ships with are
  # the network's, so a server acting for it deploys the ones the network has never had, and those whose
  # latest version a server bundled from other files, once it's started (`OneCore.Plugins.Deploy.deploy_bundled/0`), saying
  # each in the log and to `builds` as it goes: a build being tried may need them to answer at all. Then, once it's up
  # (`OneCore.Server.up?/0`), one server builds again the apps built against another UI library than the one it ships
  # with, for everyone (`OneCore.Plugins.Deploy.rebuild/0`), and `OneCore.Plugins.Deploy.await_bundled/0` waits for
  # both.
  @impl true
  def start(_type, _args) do
    # What a crash reports holds what it was working with, which may be a person's tokens: none of it reaches the log.
    OneCore.Redact.install()
    # Plugins tell the time in any zone: `DateTime.new(date, time, "Australia/Sydney")`.
    Calendar.put_time_zone_database(Tz.TimeZoneDatabase)
    {:ok, key} = Iroh.key(Server.key_file())

    if Servers.joined?(key, Application.get_env(:one_core, :owner, "network")) do
      # `OneCore.Waiting` is who waits for something to happen, by what: `:seen`, another server coming or going
      # (`OneCore.Servers.back/1`), and `:update`, this one's update moving on (`OneCore.Builds.await_update/1`).
      children = [
        {Registry, keys: :duplicate, name: OneCore.Waiting},
        {Task.Supervisor, name: OneCore.Asks},
        Deploy,
        Plugins,
        Server,
        Services,
        Jobs
      ]

      with {:ok, _supervisor} = started <-
             Supervisor.start_link(children, strategy: :one_for_one, name: OneCore.Supervisor) do
        bundled = fn ->
          if Servers.here?("network"), do: Deploy.deploy_bundled()
          Server.await_up()
          Deploy.rebuild()
        end

        Supervisor.start_child(OneCore.Supervisor, Supervisor.child_spec({Task, bundled}, id: :bundled))
        started
      end
    else
      {:error, {:not_joined, key}}
    end
  end

  # A server that's stopping, as a system stops one with SIGTERM, first takes nothing new, so whoever asks it asks
  # another, and answers what it took already, before anything of it stops. That's within the time a system gives
  # a program to stop: launchd gives 20 s. It isn't up from now on (`OneCore.Server.up?/0`), so what it answers
  # meanwhile takes no lease it would leave behind.
  @impl true
  def prep_stop(state) do
    Application.put_env(:one_core, :stopping, true)
    Server.close(:timer.seconds(15))
    state
  end
end
