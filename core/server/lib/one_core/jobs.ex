defmodule OneCore.Jobs do
  @moduledoc """
  The clock that scheduled jobs are called by.

  A job is due at each whole multiple of its `every` seconds since 1970, so every server knows when without any
  telling another. At each of those times, and when this server starts or an app's version changes, the app is
  asked to have what's due called (`OneCore.Apps.Calls.schedule/3`), for each one it was installed for, and where it keeps
  each person's data apart, for each who keeps something with it (`OneCore.Apps.Installed.subjects/2`). Every server asks,
  and the app keeping the schedule is what makes one of them make each call, once, of those acting for whoever the
  app was installed for: a time that passed with no server up is made up for by the first that's back.
  """
  use GenServer
  require Logger
  alias OneCore.{Apps, Edge}
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

  # Asks for each app that has a job due since `at` and runs here, apart from the others, and waits for the next time
  # any app has one.
  defp schedule_next(%{at: at, timer: timer} = clock) do
    if timer, do: Process.cancel_timer(timer)
    now = System.os_time(:second)
    scheduled = Apps.scheduled()
    clock = asked(clock, scheduled, at, now)

    next =
      for {_owner, _name, jobs, _trusted, _own} <- scheduled, job <- jobs, do: (div(now, job.every) + 1) * job.every

    timer =
      if next != [],
        do: Process.send_after(self(), :due, max(Enum.min(next) * 1000 - System.os_time(:millisecond), 0) + :rand.uniform(@spread))

    %{clock | timer: timer}
  end

  # Which apps run here is read from the bucket, through the edge, which may not answer for a while
  # (`OneCore.Edge.Unreachable`): then nothing is asked this time, and what was due since `at` is asked for at the
  # next, once it does. Said once each time it stops answering.
  defp asked(clock, scheduled, at, now) do
    for {owner, name, jobs, trusted, own} <- scheduled,
        Enum.any?(jobs, &(div(now, &1.every) > div(at, &1.every))),
        subject <- Apps.runs_for(owner, name, own),
        whose = if(own == :subject, do: subject, else: owner),
        Apps.runs?(owner, trusted, whose) and Apps.first?(owner, trusted, whose) do
      Task.Supervisor.start_child(OneCore.Asks, fn -> Calls.schedule(owner, name, subject) end)
    end

    if clock.unreachable, do: Logger.info("Scheduled jobs are asked for again: the bucket answers")
    %{clock | at: now, unreachable: false}
  rescue
    unreachable in Edge.Unreachable ->
      if not clock.unreachable,
        do: Logger.warning("No scheduled job is asked for until the bucket answers: #{Exception.message(unreachable)}")

      %{clock | unreachable: true}
  end
end
