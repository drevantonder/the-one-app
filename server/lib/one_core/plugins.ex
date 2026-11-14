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
          GenServer.call(__MODULE__, {:run, name, version})
          {:ok, number, warnings}

        {:error, why} ->
          Version.unload(version)
          {:error, why}
      end
    end
  end

  @doc "Removes a plugin. What it kept stays."
  def remove(name) do
    with {:ok, _files} <- kept(name),
         {:ok, _number} <- keep(name, %{}) do
      GenServer.call(__MODULE__, {:stop, name})
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

  @doc "The plugins running, by name: each one's `Version`, which holds its tools and its guide."
  def running, do: GenServer.call(__MODULE__, :running)

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

    code =
      if files == %{},
        do: "",
        else: Base.encode16(:crypto.hash(:sha256, :erlang.term_to_binary(files, [:deterministic])), case: :lower)

    for {path, text} <- files, do: Artifact.write(plugin, "code/#{code}/#{path}", text)
    number = length(Artifact.list(plugin, "versions/")) + 1

    case Artifact.write(plugin, "versions/" <> String.pad_leading("#{number}", 8, "0"), code) do
      :ok -> {:ok, number}
      :exists -> {:error, "#{name} was deployed or removed by someone else just now. Look at it, and do this again."}
    end
  end

  # The files of a plugin's newest version.
  defp kept(name) do
    plugin = artifact(name)

    with version when version != nil <- List.last(Artifact.list(plugin, "versions/")),
         {:ok, code, _} when code != "" <- Artifact.read(plugin, version) do
      {:ok,
       for path <- Artifact.list(plugin, "code/#{code}/"), into: %{} do
         {:ok, text, _} = Artifact.read(plugin, path)
         {String.replace_prefix(path, "code/#{code}/", ""), text}
       end}
    else
      _ -> {:error, "There's no plugin named #{name}"}
    end
  end

  # A server starts with the newest version of every plugin in the bucket.
  @impl true
  def init(nil) do
    names =
      Enum.uniq(for key <- Artifact.list(%Artifact{prefix: @anyone <> "plugins/"}), do: hd(String.split(key, "/")))

    running =
      for name <- names, {:ok, files} <- [kept(name)], reduce: %{} do
        running ->
          case Version.load(files) do
            {:ok, version, _warnings} ->
              Map.put(running, name, version)

            {:error, why} ->
              Logger.error("#{name} isn't running, because it doesn't compile:\n#{why}")
              running
          end
      end

    {:ok, %{running: running, calls: %{}}}
  end

  @impl true
  def handle_call({:run, name, version}, _from, state) do
    {:reply, :ok, replace(state, name, &Map.put(&1, name, version))}
  end

  def handle_call({:stop, name}, _from, state) do
    {:reply, :ok, replace(state, name, &Map.delete(&1, name))}
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

  defp replace(state, name, change) do
    was = state.running[name]
    unload_unused(%{state | running: change.(state.running)}, was)
  end

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
