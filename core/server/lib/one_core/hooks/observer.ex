defmodule OneCore.Hooks.Observer do
  @moduledoc """
  One plugin's observing on this server, for whoever it was installed for: what it observes waits here and is told
  it in turn, where it happened (`OneCore.Plugins.Calls.observe/3`). So what happened never waits on a plugin that
  observes it, and what's observed meanwhile is told under one hold of the plugin's version: a busy server holds it
  once for many calls, not once each.
  """
  use GenServer, restart: :temporary
  alias OneCore.Plugins.Calls

  @doc "What runs every plugin's observing: where each is registered, and what starts it."
  def children, do: [{Registry, keys: :unique, name: __MODULE__}, {DynamicSupervisor, name: OneCore.Hooks}]

  @doc "Tells the plugin `name`, installed for `at`, about `hook`, for `asked`. It's told once it was told what came before."
  def tell(at, name, hook, about, asked), do: send(of(at, name), {:observed, {hook, about, asked}})

  defp of(at, name) do
    with :undefined <- Registry.whereis_name({__MODULE__, {at, name}}) do
      case DynamicSupervisor.start_child(OneCore.Hooks, {__MODULE__, {at, name}}) do
        {:ok, observer} -> observer
        {:error, {:already_started, observer}} -> observer
      end
    end
  end

  def start_link(plugin), do: GenServer.start_link(__MODULE__, plugin, name: {:via, Registry, {__MODULE__, plugin}})

  @impl true
  def init(plugin), do: {:ok, plugin}

  @impl true
  def handle_info({:observed, observed}, {at, name} = plugin) do
    Calls.observe(at, name, [observed | more()])
    {:noreply, plugin}
  end

  # What else was observed by now.
  defp more do
    receive do
      {:observed, observed} -> [observed | more()]
    after
      0 -> []
    end
  end
end
