defmodule OneCore.Plugins do
  @moduledoc """
  Runs a network's plugins on this server, and adds, updates and removes them while it keeps running.

  A plugin is an artifact whose blobs are code. Each set of files deployed is kept at `code/<its hash>/`, and
  `versions/<number>` names the set that version runs: the newest is the one in use, and an empty one means the
  plugin was removed. All of it is written only if absent, so a plugin keeps its history, and two deploys at once
  can't both be the next version.

  A call runs on the version that was newest when it began. That version stays loaded until its last call ends,
  however many versions follow it.

  With no plugin that signs users in, everyone who reaches a network is the same one user. Plugins are theirs, and
  run for them.
  """
  use GenServer
  require Logger
  alias OneCore.{Artifact, Call}
  alias OneCore.Plugins.Version

  @anyone "users/anyone/"

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc "Adds a plugin, or updates it, from its files by their paths. Gives the compiler's warnings."
  def deploy(name, files) do
    with :ok <- check(name, files),
         {:ok, version, warnings} <- Version.load(files) do
      case keep(name, files) do
        {:ok, number} ->
          GenServer.call(__MODULE__, {:newest, name, number, version})
          {:ok, number, warnings}

        {:error, why} ->
          Version.unload(version)
          {:error, why}
      end
    end
  end

  @doc "Removes a plugin. What it kept stays."
  def remove(name) do
    with {:ok, _number, _files} <- kept(name),
         {:ok, number} <- keep(name, %{}) do
      GenServer.call(__MODULE__, {:newest, name, number, nil})
    end
  end

  @doc "Calls a plugin's tool. Gives `{:ok, answer}`, or `{:error, why}` with what the caller is told."
  def call(name, tool, inputs) do
    with {:ok, call, version} <- GenServer.call(__MODULE__, {:begin, name}) do
      try do
        if Enum.any?(version.tools, &(&1.name == tool)) do
          {:ok, version.plugin.call(tool, inputs, %Call{own: %Artifact{prefix: @anyone <> "own/#{name}/"}})}
        else
          {:error, "#{name} has no tool named #{tool}. It has: #{Enum.map_join(version.tools, ", ", & &1.name)}"}
        end
      catch
        kind, reason ->
          Logger.error(Exception.format(kind, reason, __STACKTRACE__))
          {blamed, _} = Exception.blame(kind, reason, __STACKTRACE__)
          why = if kind == :error, do: Exception.message(blamed), else: Exception.format_banner(kind, blamed)
          {:error, Version.own_names(why, version)}
      after
        GenServer.call(__MODULE__, {:end, call})
      end
    end
  end

  @doc "The plugins running, by name: each one's `Version`, which holds its tools, its guide and its apps."
  def running, do: GenServer.call(__MODULE__, :running)

  @doc "One file of the set of a plugin's files named `code`, which needn't be the set running now."
  def file(name, code, path) do
    with {:ok, text, _tag} <- Artifact.read(artifact(name), "code/#{code}/#{path}"), do: {:ok, text}
  end

  defp check(name, files) do
    cond do
      not (is_binary(name) and name =~ ~r/^[a-z][a-z0-9-]*$/) ->
        {:error, "A plugin's name is lowercase letters, digits and hyphens, starting with a letter"}

      not (is_map(files) and map_size(files) > 0 and
               Enum.all?(files, fn {path, text} -> path?(path) and is_binary(text) end)) ->
        {:error, "A plugin's files are text by their paths, with no path part starting with a dot"}

      true ->
        :ok
    end
  end

  defp path?(path), do: is_binary(path) and Enum.all?(String.split(path, "/"), &(&1 =~ ~r/^[^.]/))

  defp artifact(name), do: %Artifact{prefix: @anyone <> "plugins/#{name}/"}

  # Writes a set of files and the next version naming it. No files is the version that removes the plugin.
  defp keep(name, files) do
    plugin = artifact(name)

    code = if files == %{}, do: "", else: Version.code(files)

    for {path, text} <- files, do: Artifact.write(plugin, "code/#{code}/#{path}", text)
    number = length(Artifact.list(plugin, "versions/")) + 1

    case Artifact.write(plugin, "versions/" <> String.pad_leading("#{number}", 8, "0"), code) do
      :ok -> {:ok, number}
      :exists -> {:error, "#{name} was deployed or removed by someone else just now. Look at it, and do this again."}
    end
  end

  # A plugin's newest version: its number and its files.
  defp kept(name) do
    plugin = artifact(name)

    with version when version != nil <- List.last(Artifact.list(plugin, "versions/")),
         {:ok, code, _} when code != "" <- Artifact.read(plugin, version) do
      files =
        for path <- Artifact.list(plugin, "code/#{code}/"), into: %{} do
          {:ok, text, _} = Artifact.read(plugin, path)
          {String.replace_prefix(path, "code/#{code}/", ""), text}
        end

      {:ok, String.to_integer(Path.basename(version)), files}
    else
      _ -> {:error, "There's no plugin named #{name}"}
    end
  end

  # A server starts with the newest version of every plugin in the bucket.
  @impl true
  def init(nil) do
    names =
      Enum.uniq(for key <- Artifact.list(%Artifact{prefix: @anyone <> "plugins/"}), do: hd(String.split(key, "/")))

    state =
      for name <- names, {:ok, number, files} <- [kept(name)], reduce: %{running: %{}, newest: %{}, calls: %{}} do
        state ->
          case Version.load(files) do
            {:ok, version, _warnings} ->
              %{state | running: Map.put(state.running, name, version), newest: Map.put(state.newest, name, number)}

            {:error, why} ->
              Logger.error("#{name} isn't running, because it doesn't compile:\n#{why}")
              state
          end
      end

    {:ok, state}
  end

  # A plugin's newest version takes over: one to run, or none once it's removed. Deploys that finish out of order
  # still leave the one with the highest number, as the bucket has it.
  @impl true
  def handle_call({:newest, name, number, version}, _from, state) do
    if number > Map.get(state.newest, name, 0) do
      was = state.running[name]
      running = if version, do: Map.put(state.running, name, version), else: Map.delete(state.running, name)
      {:reply, :ok, unload_unused(%{state | running: running, newest: Map.put(state.newest, name, number)}, was)}
    else
      {:reply, :ok, unload_unused(state, version)}
    end
  end

  # The caller is watched, so a call that dies has ended too.
  def handle_call({:begin, name}, {caller, _}, state) do
    case state.running do
      %{^name => version} ->
        call = Process.monitor(caller)
        {:reply, {:ok, call, version}, put_in(state.calls[call], version)}

      _ ->
        {:reply, {:error, "There's no plugin named #{name}"}, state}
    end
  end

  def handle_call({:end, call}, _from, state) do
    Process.demonitor(call, [:flush])
    {:reply, :ok, ended(state, call)}
  end

  def handle_call(:running, _from, state), do: {:reply, state.running, state}

  @impl true
  def handle_info({:DOWN, call, :process, _, _}, state), do: {:noreply, ended(state, call)}

  defp ended(state, call) do
    {version, calls} = Map.pop(state.calls, call)
    unload_unused(%{state | calls: calls}, version)
  end

  defp unload_unused(state, version) do
    if version != nil and version not in Map.values(state.running) and version not in Map.values(state.calls) do
      Version.unload(version)
    end

    state
  end
end
