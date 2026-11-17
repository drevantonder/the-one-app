defmodule OneCore.Jobs do
  @moduledoc """
  The clock that scheduled jobs are called by.

  A job is due at each whole multiple of its `every` seconds since 1970, so every server knows when without any
  telling another. At each of those times, and when this server starts or an app's version changes, the app is
  asked to have what's due called (`OneCore.Apps.Calls.schedule/2`), for each one it was installed for. Every server asks,
  and the app keeping the schedule is what makes one of them make each call, once, of those acting for whoever the
  app was installed for: a time that passed with no server up is made up for by the first that's back.
  """
  use GenServer
  alias OneCore.{Apps, Servers}
  alias OneCore.Apps.Calls

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc "The apps running changed: every app's jobs are looked at now. With no clock running, nothing is."
  def reload, do: GenServer.cast(__MODULE__, :reload)

  # `at` is the second up to which what was due has been asked for, and `timer` wakes this at the next.
  @impl true
  def init(nil), do: {:ok, %{at: 0, timer: nil}, {:continue, :reload}}

  @impl true
  def handle_continue(:reload, clock), do: {:noreply, schedule_next(%{clock | at: 0})}

  @impl true
  def handle_cast(:reload, clock), do: {:noreply, schedule_next(%{clock | at: 0})}

  @impl true
  def handle_info(:due, clock), do: {:noreply, schedule_next(clock)}

  # Asks for each app that has a job due since `at`, apart from the others, and waits for the next time one is.
  defp schedule_next(%{at: at, timer: timer}) do
    if timer, do: Process.cancel_timer(timer)
    now = System.os_time(:second)

    # An untrusted app runs on any server acting for whoever it's installed for.
    scheduled =
      for {installed, name, jobs, trusted} <- Apps.scheduled(),
          Servers.runs?(installed) or (not trusted and Servers.here?(installed)),
          do: {installed, name, jobs}

    for {installed, name, jobs} <- scheduled, Enum.any?(jobs, &(div(now, &1.every) > div(at, &1.every))) do
      Task.Supervisor.start_child(OneCore.Asks, fn -> Calls.schedule(installed, name) end)
    end

    next = for {_added, _name, jobs} <- scheduled, job <- jobs, do: (div(now, job.every) + 1) * job.every

    timer =
      if next != [],
        do: Process.send_after(self(), :due, max(Enum.min(next) * 1000 - System.os_time(:millisecond), 0))

    %{at: now, timer: timer}
  end
end
