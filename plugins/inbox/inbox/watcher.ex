defmodule Inbox.Watcher do
  @moduledoc """
  Watches a folder on the machine it runs on, for each user or group its server acts for, and tells the plugin
  each file that arrives.

  It holds, between calls, the names it has seen in each folder, which is how it knows a new one.
  """
  use GenServer
  @behaviour OneCore.Service

  # Elixir isn't told when a folder changes, so it reads the folder this often, in milliseconds.
  @every 1_000

  @impl OneCore.Service
  def needs, do: "A folder on the machine that files arrive in. The watch tool says which one."

  @doc "Watches `folder` for `who` from now on, and gives its whole path. Raises if this machine has no such folder."
  def watch(service, who, folder) do
    folder = Path.expand(folder)
    if not File.dir?(folder), do: raise("There's no folder #{folder} on this machine")
    GenServer.call(service, {:watch, who, folder})
  end

  @doc "What is watched for `who`: the folder, since when, and how many files arrived since. `nil` if nothing is."
  def watching(service, who), do: GenServer.call(service, {:watching, who})

  # It starts watching what the plugin last kept for this server.
  @impl GenServer
  def init(_) do
    server = OneCore.Service.server()

    watches =
      for who <- OneCore.Service.acts_for(),
          folder = OneCore.Service.plugin(self(), who, :watched, [server]),
          into: %{},
          do: {who, watch(folder)}

    Process.send_after(self(), :look, @every)
    {:ok, %{server: server, watches: watches}}
  end

  @impl GenServer
  def handle_call({:watch, who, folder}, _from, state) do
    {:reply, folder, put_in(state.watches[who], watch(folder))}
  end

  def handle_call({:watching, who}, _from, state) do
    watching =
      with %{} = watch <- state.watches[who] do
        %{"folder" => watch.folder, "since" => watch.since, "arrived" => watch.arrived}
      end

    {:reply, watching, state}
  end

  @impl GenServer
  def handle_info(:look, state) do
    watches =
      Map.new(state.watches, fn {who, watch} ->
        names = names(watch.folder)
        arrived = Enum.sort(MapSet.difference(names, watch.seen))
        for name <- arrived, do: OneCore.Service.plugin(self(), who, :arrived, [state.server, watch.folder, name])
        {who, %{watch | seen: names, arrived: watch.arrived + length(arrived)}}
      end)

    Process.send_after(self(), :look, @every)
    {:noreply, %{state | watches: watches}}
  end

  defp watch(folder) do
    %{folder: folder, seen: names(folder), since: DateTime.to_iso8601(DateTime.utc_now(:second)), arrived: 0}
  end

  # A folder that has gone has nothing in it, until it's back.
  defp names(folder) do
    case File.ls(folder) do
      {:ok, names} -> MapSet.new(names)
      {:error, _gone} -> MapSet.new()
    end
  end
end
