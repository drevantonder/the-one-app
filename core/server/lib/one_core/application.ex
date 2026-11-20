defmodule OneCore.Application do
  @moduledoc false
  use Application
  require Logger
  alias OneCore.{Iroh, Jobs, Apps, Release, Server, Servers, Services}
  alias OneCore.Apps.Deploy

  # A server is started for its owner, its `:owner` setting: a user, a group or the network. One that isn't on
  # their list doesn't start: `{:not_joined, key}` says so, with its key. Once it's up (`OneCore.Server.up?/0`), one
  # server builds again the apps built against another UI library than the one it ships with, for everyone
  # (`OneCore.Apps.Deploy.rebuild/0`).

  @impl true
  def start(_type, _args) do
    # What a crash reports holds what it was working with, which may be a person's tokens: none of it reaches the log.
    OneCore.Redact.install()
    # What it logs that's a warning or worse is exported with its spans and metrics (`OneCore.Otlp`).
    OneCore.Otlp.start()
    # What every server caches (`OneCore.Cache`), which this process, the application's, keeps.
    OneCore.Cache.table()

    # A server that couldn't reach the bucket may have missed being told something meanwhile: once it can, it looks again.
    OneCore.Bucket.Fuse.install(&Apps.reconnected/0)
    # Windows keeps a few root certificates and fetches the rest only as its own programs ask it to, which Erlang
    # doesn't, so github.com's was missing on a new PC: a server there trusts Mozilla's, as Mint does where a system
    # has none (`castore`). Every `:public_key.cacerts_get/0`, and so every HTTPS request, has them from here.
    if match?({:win32, _}, :os.type()), do: :ok = :public_key.cacerts_load(CAStore.file_path())
    # Apps tell the time in any zone: `DateTime.new(date, time, "Australia/Sydney")`.
    Calendar.put_time_zone_database(Tz.TimeZoneDatabase)
    {:ok, key} = Iroh.key(Server.key_file())

    if Servers.joined?(key, Application.get_env(:one_core, :owner, "network")) do
      # What state each call is in, which this process, the application's, keeps.
      OneCore.Call.State.table()
      # What everyone has deployed (`OneCore.Apps.Deploy.deployed/0`), which this process keeps too.
      Deploy.table()

      # `OneCore.Waiting` is who waits for something to happen, by what: `:seen`, another server coming or going
      # (`OneCore.Servers.back/1`), `:status`, what this one is changing (`OneCore.Release.await/1`), and
      # `{:watching, device}`, a device's app watching again once it was woken (`OneCore.Server.ask_device/3`).
      children = [
        {Registry, keys: :duplicate, name: OneCore.Waiting},
        {Task.Supervisor, name: OneCore.Asks},
        # It's starting at the network's edge from before it's reached there until it's ready, its apps started
        # and answering by its key, so nobody asks it what it would answer only once they have: another server's
        # lease it held before it stopped among them. `Servers.Report` says so as it starts, before them.
        Servers.Report,
        Apps,
        # A message is answered once the server is up, its apps there and running, since they say whose a device is
        # and hold its leases: so whoever asks while it starts waits for it, and isn't told the server can't be
        # reached, that their device isn't signed in, or that it's starting.
        {Server, handle: &answer/3, wake: &OneCore.Hooks.wake/1, started: &Apps.changed/0},
        Services,
        Jobs,
        OneCore.Apps.Lifecycle
      ]

      with {:ok, _supervisor} = started <-
             Supervisor.start_link(children, strategy: :one_for_one, name: OneCore.Supervisor) do
        # An installed server of a network takes its core apps to the build it started on (`update_core_apps/0`),
        # before it builds again what was built against another UI library, which those then aren't: the two would
        # otherwise deploy the same app at once, and one of them be refused.
        rebuild = fn ->
          Server.await_up()

          if Application.get_env(:one_core, :installed, false) and !Application.get_env(:one_core, :first_owner) do
            with {:error, why} <- OneCore.Apps.Lifecycle.update_core_apps(), do: Logger.error(why)
          end

          Deploy.rebuild()
        end

        # A server nothing keeps running, as a network being built runs one, has nothing to answer by its key to.
        up = fn ->
          Apps.started()
          OneCore.Hooks.observe("started", %{"server" => key}, {nil, OneCore.Servers.owner()})
          if not Application.get_env(:one_core, :installed, false), do: Release.told(%{"server_daemon_status" => "running"})
          # A new network's first server makes it, once it's running (`OneCore.Apps.Lifecycle.found/1`).
          if owner = Application.get_env(:one_core, :first_owner) do
            with {:error, why} <- OneCore.Apps.Lifecycle.found("users/" <> owner),
                 do: Logger.error("users/#{owner} isn't the network's first owner: #{why}")
          end
        end

        Supervisor.start_child(OneCore.Supervisor, Supervisor.child_spec({Task, rebuild}, id: :rebuild))
        Supervisor.start_child(OneCore.Supervisor, Supervisor.child_spec({Task, up}, id: :up))
        started
      end
    else
      {:error, {:not_joined, key}}
    end
  end

  defp answer(message, from, ask) do
    Apps.started()
    # One stopping answers what it took already, and takes nothing new (`prep_stop/1`).
    Release.await(&(&1["server_daemon_status"] != "starting"))
    OneCore.Mcp.handle(message, from, ask)
  end

  # A server that's stopping, as a system stops one with SIGTERM, drains: it says it's stopping, so it isn't ready
  # and the network sends it nothing new, then takes nothing new, so whoever asks it asks another, and the leases it
  # holds are taken by whoever asks for them next, and it answers what it took already, before anything of it stops.
  # That's within the time the program keeping it running gives it to stop (`STOP`).
  @impl true
  def prep_stop(state) do
    Release.told(%{"server_daemon_status" => "stopping"})
    Server.close(:timer.seconds(15))
    state
  end
end
