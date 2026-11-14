defmodule OneCore.Plugins do
  @moduledoc """
  Runs a network's plugins on this server, and adds, updates and removes them while it keeps running.

  A plugin is an artifact whose blobs are code. Each set of files deployed is kept at `code/<its hash>/`, and
  `versions/<number>` names the set that version runs: the newest is the one in use, and an empty one means the
  plugin was removed. All of it is written only if absent, so a plugin keeps its history, and two deploys at once
  can't both be the next version.

  A call runs on the version that was newest when it began. That version stays loaded until its last call ends,
  however many versions follow it.

  A plugin says which other plugins it uses, and a call holds the versions of those too. Adding a plugin brings
  along the ones it uses that the server ships with, from the folder that is its `:shipped` setting.

  A server learns of a version another server was given by being told to look, and by looking when it's asked for a
  plugin it doesn't run.

  With no plugin that signs users in, everyone who reaches a network is the same one user. Plugins are theirs, and
  run for them.
  """
  use GenServer
  require Logger
  alias OneCore.{Artifact, Call, Server, Servers}
  alias OneCore.Plugins.Version

  @anyone "users/anyone/"

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc "Adds a plugin, or updates it, from its files by their paths. Gives the compiler's warnings."
  def deploy(name, files) do
    with :ok <- check(name, files),
         {:ok, version, warnings} <- Version.load(files) do
      with :ok <- bring(name, version.uses),
           {:ok, number} <- keep(name, files) do
        GenServer.call(__MODULE__, {:newest, name, number, version})
        tell(name)
        {:ok, number, warnings}
      else
        {:error, why} ->
          Version.unload(version)
          {:error, why}
      end
    end
  end

  @doc "Removes a plugin. What it kept stays."
  def remove(name) do
    used_by = for {other, version} <- running(), name in version.uses, do: other

    with [] <- used_by,
         {:ok, _number, _files} <- kept(name),
         {:ok, number} <- keep(name, %{}) do
      GenServer.call(__MODULE__, {:newest, name, number, nil})
      tell(name)
    else
      [_ | _] -> {:error, "#{name} is used by #{Enum.join(used_by, ", ")}. Remove what uses it first."}
      {:error, why} -> {:error, why}
      _ -> {:error, "There's no plugin named #{name}"}
    end
  end

  @doc """
  Calls a plugin's tool. Gives `{:ok, answer}`, or `{:error, why}` with what the caller is told. A call another
  server ran instead gives `{:passed_on, result}`, that server's result. `passed_on` says another server passed
  this call on to this one, and `:passed_back` that this one would pass it on in turn.
  """
  def call(name, tool, inputs, passed_on \\ false) do
    with {:ok, call, versions} <- begin(name) do
      version = versions[name]

      try do
        if Enum.any?(version.tools, &(&1.name == tool)) do
          uses = Map.new(versions, fn {used, version} -> {used, version.plugin} end)

          {:ok,
           version.plugin.call(tool, inputs, %Call{
             own: %Artifact{prefix: @anyone <> "own/#{name}/"},
             server: Server.key(),
             plugin: name,
             tool: tool,
             inputs: inputs,
             uses: Map.delete(uses, name),
             passed_on: passed_on
           })}
        else
          {:error, "#{name} has no tool named #{tool}. It has: #{Enum.map_join(version.tools, ", ", & &1.name)}"}
        end
      catch
        :throw, {Call, outcome} ->
          outcome

        kind, reason ->
          Logger.error(Exception.format(kind, reason, __STACKTRACE__))
          {blamed, _} = Exception.blame(kind, reason, __STACKTRACE__)
          why = if kind == :error, do: Exception.message(blamed), else: Exception.format_banner(kind, blamed)
          {:error, Enum.reduce(Map.values(versions), why, &Version.own_names(&2, &1))}
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

  @doc "Looks in the bucket for a plugin's newest version, and runs it. Gives whether the plugin is running now."
  def look(name) do
    with true <- name?(name),
         {number, files} <- newest(name),
         true <- number > GenServer.call(__MODULE__, {:number, name}),
         {:ok, version} <- loaded(name, files) do
      GenServer.call(__MODULE__, {:newest, name, number, version})
    end

    is_map_key(running(), name)
  end

  # A call holds the newest version of its plugin and of each plugin that one uses, however far that goes.
  defp begin(name) do
    case GenServer.call(__MODULE__, {:begin, name}) do
      {:absent, missing} ->
        cond do
          look(missing) -> begin(name)
          missing == name -> {:error, "There's no plugin named #{name}"}
          true -> {:error, "#{name} needs #{missing}, which isn't in this network"}
        end

      began ->
        began
    end
  end

  # The plugins a plugin uses are in the network before it is: the ones missing come from what the server ships
  # with.
  defp bring(name, uses) do
    case Enum.reject(uses, &(look(&1) or match?({:ok, _, _}, deploy(&1, shipped(&1))))) do
      [] -> :ok
      missing -> {:error, "#{name} uses #{Enum.join(missing, ", ")}, which isn't in this network. Deploy it first."}
    end
  end

  defp shipped(name) do
    with {:ok, folder} <- Application.fetch_env(:one_core, :shipped) do
      folder = Path.join(folder, name)
      files = Enum.filter(Path.wildcard(Path.join(folder, "**")), &File.regular?/1)
      Map.new(files, &{Path.relative_to(&1, folder), File.read!(&1)})
    end
  end

  # The other servers acting for whoever a plugin is for are told to look at it, and have looked, or couldn't be
  # reached, when this returns.
  defp tell(name) do
    message = JSON.encode!(%{jsonrpc: "2.0", method: "notifications/plugin", params: %{name: name}})
    others = Servers.acting() -- [Server.key()]
    Stream.run(Task.async_stream(others, &Server.ask(&1, message), timeout: :infinity))
  end

  defp check(name, files) do
    cond do
      not name?(name) ->
        {:error, "A plugin's name is lowercase letters, digits and hyphens, starting with a letter"}

      not (is_map(files) and map_size(files) > 0 and
               Enum.all?(files, fn {path, text} -> path?(path) and is_binary(text) end)) ->
        {:error, "A plugin's files are text by their paths, with no path part starting with a dot"}

      true ->
        :ok
    end
  end

  defp name?(name), do: is_binary(name) and name =~ ~r/^[a-z][a-z0-9-]*$/

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

  # A plugin's newest version: its number and its files, which are none if that version removed it.
  defp newest(name) do
    plugin = artifact(name)

    with version when version != nil <- List.last(Artifact.list(plugin, "versions/")) do
      {:ok, code, _} = Artifact.read(plugin, version)

      files =
        for path <- Artifact.list(plugin, "code/#{code}/"), code != "", into: %{} do
          {:ok, text, _} = Artifact.read(plugin, path)
          {String.replace_prefix(path, "code/#{code}/", ""), text}
        end

      {String.to_integer(Path.basename(version)), files}
    end
  end

  defp kept(name) do
    with {number, files} when files != %{} <- newest(name), do: {:ok, number, files}
  end

  # The version a plugin's files are, loaded. No files is no version: the plugin was removed.
  defp loaded(_name, files) when files == %{}, do: {:ok, nil}

  defp loaded(name, files) do
    case Version.load(files) do
      {:ok, version, _warnings} ->
        {:ok, version}

      {:error, why} ->
        Logger.error("#{name} isn't running, because it doesn't compile:\n#{why}")
        :error
    end
  end

  # A server starts with the newest version of every plugin in the bucket.
  @impl true
  def init(nil) do
    names =
      Enum.uniq(for key <- Artifact.list(%Artifact{prefix: @anyone <> "plugins/"}), do: hd(String.split(key, "/")))

    state =
      for name <- names,
          {number, files} <- [newest(name)],
          {:ok, version} when version != nil <- [loaded(name, files)],
          reduce: %{running: %{}, newest: %{}, calls: %{}} do
        state ->
          %{state | running: Map.put(state.running, name, version), newest: Map.put(state.newest, name, number)}
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
      {:reply, :ok, unload_unused(%{state | running: running, newest: Map.put(state.newest, name, number)}, [was])}
    else
      {:reply, :ok, unload_unused(state, [version])}
    end
  end

  def handle_call({:number, name}, _from, state), do: {:reply, Map.get(state.newest, name, 0), state}

  # The caller is watched, so a call that dies has ended too.
  def handle_call({:begin, name}, {caller, _}, state) do
    case used(state.running, [name], %{}) do
      {:ok, versions} ->
        call = Process.monitor(caller)
        {:reply, {:ok, call, versions}, put_in(state.calls[call], Map.values(versions))}

      absent ->
        {:reply, absent, state}
    end
  end

  def handle_call({:end, call}, _from, state) do
    Process.demonitor(call, [:flush])
    {:reply, :ok, ended(state, call)}
  end

  def handle_call(:running, _from, state), do: {:reply, state.running, state}

  @impl true
  def handle_info({:DOWN, call, :process, _, _}, state), do: {:noreply, ended(state, call)}

  # The versions a call to `names` holds, by their plugins' names, or the first plugin that isn't running.
  defp used(_running, [], found), do: {:ok, found}
  defp used(running, [name | names], found) when is_map_key(found, name), do: used(running, names, found)

  defp used(running, [name | names], found) do
    case running do
      %{^name => version} -> used(running, version.uses ++ names, Map.put(found, name, version))
      _ -> {:absent, name}
    end
  end

  defp ended(state, call) do
    {versions, calls} = Map.pop(state.calls, call, [])
    unload_unused(%{state | calls: calls}, versions)
  end

  defp unload_unused(state, versions) do
    held = Map.values(state.running) ++ Enum.concat(Map.values(state.calls))
    for version <- versions, version != nil and version not in held, do: Version.unload(version)
    state
  end
end
