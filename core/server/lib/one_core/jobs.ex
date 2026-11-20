defmodule OneCore.Jobs do
  @moduledoc """
  The clock that scheduled jobs are called by.

  A job is due at each whole multiple of its `every` seconds since 1970, so every server knows when without any
  telling another. At each of those times, and when this server starts or an app's version changes, the app is
  asked to have what's due called (`OneCore.Apps.Calls.schedule/3`), for each one it was installed for. Every server
  that would run it asks, and the app keeping the schedule is what makes one of them make each call, once: a time that
  passed with no server up is made up for by the first that's back. What's done for a person is their timer's.

  An app also sets a timer for someone as they do something (`Database.timer/5` in the `database` app), kept with
  their data and said by an alarm, a key `<whose>/alarms/<installed for>/<app>/<due>`, which the database holding
  it keeps at its next one. Each minute a server acting for the network looks at everyone's alarms, as it acts for
  everyone, and any other server at those of whoever it's for, and one that's due has its app make what's due, on the
  server the one rule picks (`OneCore.Apps.Calls.schedule/4`), once.
  """
  use GenServer
  require Logger
  alias OneCore.{Apps, Bucket, Edge, Servers}
  alias OneCore.Apps.Calls

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc "The apps running changed: every app's jobs are looked at now. With no clock running, nothing is."
  def reload, do: GenServer.cast(__MODULE__, :reload)

  # `at` is the second up to which what was due has been asked for, `timer` wakes this at the next, and `unreachable`
  # is whether the last time couldn't say which apps run here.
  @impl true
  def init(nil), do: {:ok, %{at: 0, timer: nil, unreachable: false}, {:continue, :reload}}

  @impl true
  def handle_continue(:reload, clock), do: {:noreply, schedule_next(%{clock | at: 0})}

  @impl true
  def handle_cast(:reload, clock), do: {:noreply, schedule_next(%{clock | at: 0})}

  @impl true
  def handle_info(:due, clock), do: {:noreply, schedule_next(clock)}

  # Every server's clock would wake at the same whole minute, and each asks the server holding an app's schedule for
  # what's due at once. So each wakes a random part of `@spread` milliseconds after it, as systemd's timers do with
  # `RandomizedDelaySec`, so that machines don't all fire together. It's well short of the shortest job's `every`.
  @spread 10_000

  # How often alarms are looked for, in seconds: a timer fires within a minute of when it was set for.
  @alarms 60

  # Asks for each app that has a job due since `at` and runs here, apart from the others, and waits for the next time
  # any app has one.
  defp schedule_next(%{at: at, timer: timer} = clock) do
    if timer, do: Process.cancel_timer(timer)
    now = System.os_time(:second)
    scheduled = Apps.scheduled()
    clock = asked(clock, scheduled, at, now)

    next =
      for {_owner, _name, jobs, _trusted, _own} <- scheduled, job <- jobs, do: (div(now, job.every) + 1) * job.every

    next = Enum.min([(div(now, @alarms) + 1) * @alarms | next])
    timer = Process.send_after(self(), :due, max(next * 1000 - System.os_time(:millisecond), 0) + :rand.uniform(@spread))

    %{clock | timer: timer}
  end

  # Which apps run here is read from the bucket, through the edge, which may not answer for a while
  # (`OneCore.Edge.Unreachable`): then nothing is asked this time, and what was due since `at` is asked for at the
  # next, once it does. Said once each time it stops answering.
  defp asked(clock, scheduled, at, now) do
    for {owner, name, jobs, trusted, _own} <- scheduled,
        Enum.any?(jobs, &(div(now, &1.every) > div(at, &1.every))),
        Apps.runs?(owner, trusted) and Apps.first?(owner, trusted, owner) do
      Task.Supervisor.start_child(OneCore.Asks, fn -> Calls.schedule(owner, name, owner) end)
    end

    if div(now, @alarms) > div(at, @alarms), do: alarmed(now)

    if clock.unreachable, do: Logger.info("Scheduled jobs are asked for again: the bucket answers")
    %{clock | at: now, unreachable: false}
  rescue
    unreachable in Edge.Unreachable ->
      if not clock.unreachable,
        do: Logger.warning("No scheduled job is asked for until the bucket answers: #{Exception.message(unreachable)}")

      %{clock | unreachable: true}
  end

  # Has each alarm that's due, of whoever this server looks at, make what's due. Where the bucket won't list them yet,
  # as for a server whose start the presigner hasn't heard of, they're looked for again the next minute.
  defp alarmed(now) do
    for whose <- looked_at(),
        key <- Bucket.list(whose <> "/alarms/"),
        {at, name, due} = alarm(whose, key),
        due <= now do
      Task.Supervisor.start_child(OneCore.Asks, fn -> Calls.schedule(at, name, whose) end)
    end
  rescue
    refused in Bucket.Refused -> Logger.info("Alarms are looked for again in a minute: #{Exception.message(refused)}")
  end

  # Whose alarms this server looks at: everyone's, and the network's, for one acting for the network, which it lists
  # by their folders, and whoever else it's for.
  defp looked_at do
    everyone =
      if Servers.here?("network"),
        do: ["network" | for(folder <- Bucket.folders("users/") ++ Bucket.folders("groups/"), do: String.trim_trailing(folder, "/"))],
        else: []

    Enum.uniq(everyone ++ Servers.here())
  end

  # An alarm's key: whoever the app was installed for, the app's name, and when it's due, in seconds since 1970.
  defp alarm(whose, key) do
    {at, [name, due]} = Enum.split(String.split(String.replace_prefix(key, whose <> "/alarms/", ""), "/"), -2)
    {Enum.join(at, "/"), name, String.to_integer(due)}
  end
end
