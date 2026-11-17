defmodule OneCore.Application do
  @moduledoc false
  use Application
  alias OneCore.{Iroh, Jobs, Plugins, Server, Servers, Services}
  alias OneCore.Plugins.Deploy

  # A server is started for its owner, its `:owner` setting: a user, a group or the network. One that isn't on
  # their list doesn't start: `{:not_joined, key}` says so, with its key. Once it's up (`OneCore.Server.up?/0`), one
  # server builds again the apps built against another UI library than the one it ships with, for everyone
  # (`OneCore.Plugins.Deploy.rebuild/0`).

  # How long a server that's starting may take to be up, in seconds, after which the edge takes it for on once it's
  # connected: a build being tried has three minutes to answer by its key, after its plugins have started.
  @up_within 300

  @impl true
  def start(_type, _args) do
    # What a crash reports holds what it was working with, which may be a person's tokens: none of it reaches the log.
    OneCore.Redact.install()
    OneCore.Bucket.Fuse.install()
    # Plugins tell the time in any zone: `DateTime.new(date, time, "Australia/Sydney")`.
    Calendar.put_time_zone_database(Tz.TimeZoneDatabase)
    {:ok, key} = Iroh.key(Server.key_file())

    if Servers.joined?(key, Application.get_env(:one_core, :owner, "network")) do
      # What state each call is in, which this process, the application's, keeps.
      OneCore.Call.State.table()
      # What everyone has deployed (`OneCore.Plugins.Deploy.deployed/0`), which this process keeps too.
      Deploy.table()

      # `OneCore.Waiting` is who waits for something to happen, by what: `:seen`, another server coming or going
      # (`OneCore.Servers.back/1`), and `:update`, this one's update moving on (`OneCore.Builds.await_update/1`).
      children = [
        {Registry, keys: :duplicate, name: OneCore.Waiting},
        {Task.Supervisor, name: OneCore.Asks},
        Plugins,
        Server,
        Services,
        Jobs,
        OneCore.Plugins.Lifecycle
      ]

      # It's restarting at the network's edge from before it's reached there until it's up, its plugins started, so
      # nobody asks it what it would answer only once they have: another server's lease it held before it stopped
      # among them.
      Servers.restarting(@up_within)

      with {:ok, _supervisor} = started <-
             Supervisor.start_link(children, strategy: :one_for_one, name: OneCore.Supervisor) do
        rebuild = fn ->
          Server.await_up()
          Deploy.rebuild()
        end

        up = fn ->
          Plugins.started()
          Server.await_up()
          Servers.up()
        end

        Supervisor.start_child(OneCore.Supervisor, Supervisor.child_spec({Task, rebuild}, id: :rebuild))
        Supervisor.start_child(OneCore.Supervisor, Supervisor.child_spec({Task, up}, id: :up))
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
