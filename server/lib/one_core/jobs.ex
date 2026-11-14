defmodule OneCore.Jobs do
  @moduledoc """
  The clock that scheduled jobs are called by.

  A job is due at each whole multiple of its `every` seconds since 1970, so every server knows when without any
  telling another. At each of those times, and when this server starts or a plugin's version changes, the plugin
  is asked to have what's due called (`OneCore.Plugins.schedule/2`), for each one it was added for. Every server asks, and the plugin keeping the
  schedule is what makes one of them make each call, once: a time that passed with no server up is made up for
  by the first that's back.
  """
  use GenServer
  alias OneCore.Plugins

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc "The plugins running changed: every plugin's jobs are looked at now. With no clock running, nothing is."
  def look, do: GenServer.cast(__MODULE__, :look)

  # `at` is the second up to which what was due has been asked for, and `timer` wakes this at the next.
  @impl true
  def init(nil), do: {:ok, %{at: 0, timer: nil}, {:continue, :look}}

  @impl true
  def handle_continue(:look, clock), do: {:noreply, asked(%{clock | at: 0})}

  @impl true
  def handle_cast(:look, clock), do: {:noreply, asked(%{clock | at: 0})}

  @impl true
  def handle_info(:due, clock), do: {:noreply, asked(clock)}

  # Asks for each plugin that has a job due since `at`, apart from the others, and waits for the next time one is.
  defp asked(%{at: at, timer: timer}) do
    if timer, do: Process.cancel_timer(timer)
    now = System.os_time(:second)
    scheduled = Plugins.scheduled()

    for {added, name, jobs} <- scheduled, Enum.any?(jobs, &(div(now, &1.every) > div(at, &1.every))) do
      Task.Supervisor.start_child(OneCore.Asks, fn -> Plugins.schedule(added, name) end)
    end

    next = for {_added, _name, jobs} <- scheduled, job <- jobs, do: (div(now, job.every) + 1) * job.every

    timer =
      if next != [],
        do: Process.send_after(self(), :due, max(Enum.min(next) * 1000 - System.os_time(:millisecond), 0))

    %{at: now, timer: timer}
  end
end
