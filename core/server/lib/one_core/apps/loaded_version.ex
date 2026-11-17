defmodule OneCore.Apps.LoadedVersion do
  @moduledoc """
  Each version of an app loaded on this server, one state machine each, `loaded_app_version`. Loaded is
  Erlang's word (`code:is_loaded`) for code in a server's memory, as opposed to deployed, in the bucket for the
  whole network (`OneCore.Apps.Deploy`).

      loading ──compiled, or loaded as it was compiled before──► loaded (in use by n)
      loading ──compile fails or raises──► (unloaded)
      loaded ──n reaches 0──► unloading ──old calls finished (Erlang's purge)──► (unloaded)

  The VM keeps only two versions of a module, so a version's modules sit under a name of its own, named for its app
  and its code (`OneCore.Apps.V<hash>.Notes`), as a NIF's build is (`OneCore.Nif`): the same version has the same
  name each time it's loaded, so what it was compiled to is loaded again rather than compiled
  (`OneCore.Apps.Version.Compile`). Unloading purges its modules, so it can be loaded again under that name.

  `n` counts each owner the version is installed for (`installed/1`), and each hold of a process on it: a call, a
  app's process, or whatever loaded it (`hold/2`), until it lets go (`release/1`) or ends. Releasing is the only
  way a version is unloaded: by then every call on it has finished, and Erlang's purge ends whatever else still runs
  its code. A version loaded already, by its app and its code, isn't loaded again: whoever loads it holds it too.
  The NIF builds a version loaded (`OneCore.Nif`) are let go of as it's unloaded.

  A version only described here (`OneCore.Apps.Version.Description`) is in the same machine by its app and
  code, with no name or modules: it's loaded as it's described, and unloading it only forgets it.

  Each is a row of a table this owns, which is started before `OneCore.Apps` and outlives it, so a restarted
  `OneCore.Apps` adopts what's loaded rather than loading it again under new names. What a version's calls read of
  it, its files and skills, is kept in `:persistent_term` (`kept/1`), where reading copies nothing.
  """
  use GenServer
  alias OneCore.Nif

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc """
  Loads the version of the app `artifact` whose files are named `code`, held by the caller: `{:ok, version, warnings}`,
  or `{:error, why}`. One loaded already is the one held. Otherwise `compile` is called with the name it's
  loaded under, in the caller, and gives `{:ok, version, warnings, kept}`, with what `kept/1` gives, or `{:error,
  why}`.
  """
  def load(artifact, code, compile) do
    case call({:load, artifact, code}) do
      {:loading, under} ->
        outcome =
          try do
            compile.(under)
          catch
            kind, reason ->
              call({:failed, under, Exception.format_banner(kind, reason, __STACKTRACE__)})
              :erlang.raise(kind, reason, __STACKTRACE__)
          end

        case outcome do
          {:ok, version, warnings, kept} -> call({:loaded, under, version, warnings, kept})
          {:error, why} -> call({:failed, under, why})
        end

      loaded ->
        loaded
    end
  end

  @doc """
  A version only described here, `version`, held by the caller. One described already, by its app and code, keeps
  what it keeps (`kept/1`), and `kept` is what it keeps otherwise.
  """
  def described(%{under: {:described, _artifact, _code}} = version, kept), do: call({:described, version, kept})

  @doc "Holds `versions` for the process `pid` until it lets go of them (`release/1`) or ends. Each is a hold of its own."
  def hold(versions, pid), do: call({:hold, Enum.map(versions, & &1.under), pid})

  @doc "Lets go of one of the caller's holds on each of `versions`. One nobody holds or has installed any more is unloaded."
  def release(versions), do: call({:release, Enum.map(versions, & &1.under)})

  @doc """
  Says which versions are installed for each owner in `installed`, `%{owner => versions}`: each counts once for each
  owner it's installed for, and one installed for an owner before that isn't any more is let go of. Owners left out
  keep theirs.
  """
  def installed(installed), do: call({:installed, Map.new(installed, fn {owner, versions} -> {owner, MapSet.new(versions, & &1.under)} end)})

  @doc "Everyone something loaded here is installed for: whose apps a restarted `OneCore.Apps` looks at again."
  def owners, do: Enum.uniq(for {_under, row} <- :ets.tab2list(__MODULE__), owner <- row.owners, do: owner)

  @doc "Each version loaded here, by the name it's under: its `state`, `artifact`, `code`, `owners` and `holds` by process."
  def all, do: Map.new(:ets.tab2list(__MODULE__), fn {under, row} -> {under, Map.take(row, [:state, :artifact, :code, :owners, :holds])} end)

  @doc "What a loaded version's calls read of it, by the name it's under: its `files`, `hashes`, `skills` and `own` files."
  def kept(under), do: :persistent_term.get({__MODULE__, under})

  defp call(message), do: GenServer.call(__MODULE__, message, :infinity)

  # Each row is a version by the name it's under, with its state and who uses it: the owners it's installed for, and
  # how many holds each process has. `loaders` are the processes loading a version, by their monitors, which are
  # watched too, so a loading that's killed is unloaded, and `waiting` who waits for a version loading, by its name.
  @impl true
  def init(nil) do
    # What an owner of this table before this one left loaded nobody can say is used any more, so it's unloaded:
    # `OneCore.Apps`, started after this, loads what runs again.
    left = for {{__MODULE__, under}, _kept} <- :persistent_term.get(), do: under
    named = for {module, _} <- :code.all_loaded(), under <- [slot(module)], under != nil, do: under
    Enum.each(Enum.uniq(left ++ named), &free/1)

    :ets.new(__MODULE__, [:named_table, :protected, read_concurrency: true])
    {:ok, %{loaders: %{}, holders: %{}, waiting: %{}}}
  end

  @impl true
  def handle_call({:load, artifact, code}, {pid, _} = from, state) do
    under = name(artifact, code)

    case :ets.lookup(__MODULE__, under) do
      [{^under, %{state: :loaded} = row}] ->
        {:reply, {:ok, row.version, row.warnings}, held(state, [under], pid)}

      [{^under, %{state: :loading}}] ->
        {:noreply, update_in(state.waiting[under], &[from | &1])}

      [] ->
        :ets.insert(__MODULE__, {under, %{state: :loading, artifact: artifact, code: code, version: nil, warnings: nil, owners: MapSet.new(), holds: %{}}})
        state = put_in(state.loaders[Process.monitor(pid)], {under, pid})
        {:reply, {:loading, under}, put_in(state.waiting[under], [])}
    end
  end

  def handle_call({:loaded, under, version, warnings, kept}, _from, state) do
    {monitor, pid} = loader(state, under)
    Process.demonitor(monitor, [:flush])
    version = %{version | artifact: row(under).artifact}
    :persistent_term.put({__MODULE__, under}, kept)
    :ets.insert(__MODULE__, {under, %{row(under) | state: :loaded, version: version, warnings: warnings}})
    {waiting, state} = pop_in(state.waiting[under])
    state = held(%{state | loaders: Map.delete(state.loaders, monitor)}, [under], pid)

    state =
      for {pid, _} = from <- waiting, reduce: state do
        state ->
          GenServer.reply(from, {:ok, version, warnings})
          held(state, [under], pid)
      end

    {:reply, {:ok, version, warnings}, state}
  end

  def handle_call({:failed, under, why}, _from, state) do
    {monitor, _pid} = loader(state, under)
    Process.demonitor(monitor, [:flush])
    {:reply, {:error, why}, failed(%{state | loaders: Map.delete(state.loaders, monitor)}, under, why)}
  end

  def handle_call({:described, %{under: under} = version, kept}, {pid, _}, state) do
    if not :ets.member(__MODULE__, under) do
      {:described, artifact, code} = under
      :persistent_term.put({__MODULE__, under}, kept)
      :ets.insert(__MODULE__, {under, %{state: :loaded, artifact: artifact, code: code, version: version, warnings: "", owners: MapSet.new(), holds: %{}}})
    end

    {:reply, :ok, held(state, [under], pid)}
  end

  def handle_call({:hold, unders, pid}, _from, state) do
    case Enum.reject(unders, &match?([{_, %{state: :loaded}}], :ets.lookup(__MODULE__, &1))) do
      [] -> {:reply, :ok, held(state, unders, pid)}
      gone -> {:reply, {:error, "#{inspect(gone)} isn't loaded here"}, state}
    end
  end

  def handle_call({:release, unders}, {pid, _}, state) do
    state =
      Enum.reduce(unders, state, fn under, state ->
        case :ets.lookup(__MODULE__, under) do
          [{^under, %{holds: %{^pid => holds}} = row}] ->
            holds = if holds == 1, do: Map.delete(row.holds, pid), else: %{row.holds | pid => holds - 1}
            used(state, under, %{row | holds: holds})

          _ ->
            state
        end
      end)

    {:reply, :ok, forget(state, pid)}
  end

  def handle_call({:installed, installed}, _from, state) do
    state =
      for {under, row} <- :ets.tab2list(__MODULE__), row.state == :loaded, reduce: state do
        state ->
          owners =
            Enum.reduce(installed, row.owners, fn {owner, unders}, owners ->
              if under in unders, do: MapSet.put(owners, owner), else: MapSet.delete(owners, owner)
            end)

          used(state, under, %{row | owners: owners})
      end

    {:reply, :ok, state}
  end

  # A process that loaded or held a version ended: a loading is unloaded, and each hold is let go of.
  @impl true
  def handle_info({:DOWN, monitor, :process, pid, why}, state) do
    case Map.pop(state.loaders, monitor) do
      {{under, _pid}, loaders} ->
        {:noreply, failed(%{state | loaders: loaders}, under, "its loading ended: #{Exception.format_exit(why)}")}

      {nil, _loaders} ->
        state =
          for {under, %{holds: %{^pid => _}} = row} <- :ets.tab2list(__MODULE__), reduce: state do
            state -> used(state, under, %{row | holds: Map.delete(row.holds, pid)})
          end

        {:noreply, %{state | holders: Map.delete(state.holders, pid)}}
    end
  end

  defp held(state, unders, pid) do
    for under <- unders, do: update(under, &%{&1 | holds: Map.update(&1.holds, pid, 1, fn holds -> holds + 1 end)})
    if is_map_key(state.holders, pid), do: state, else: put_in(state.holders[pid], Process.monitor(pid))
  end

  # A process that holds nothing any more isn't watched.
  defp forget(state, pid) do
    if Enum.any?(:ets.tab2list(__MODULE__), fn {_under, row} -> is_map_key(row.holds, pid) end) or not is_map_key(state.holders, pid) do
      state
    else
      Process.demonitor(state.holders[pid], [:flush])
      %{state | holders: Map.delete(state.holders, pid)}
    end
  end

  # A version whose `n` reaches 0 is unloading: its modules are purged.
  defp used(state, under, row) do
    if MapSet.size(row.owners) == 0 and row.holds == %{} do
      :ets.insert(__MODULE__, {under, %{row | state: :unloading}})
      free(under)
      :ets.delete(__MODULE__, under)
    else
      :ets.insert(__MODULE__, {under, row})
    end

    state
  end

  defp failed(state, under, why) do
    free(under)
    :ets.delete(__MODULE__, under)
    {waiting, state} = pop_in(state.waiting[under])
    for from <- waiting || [], do: GenServer.reply(from, {:error, why})
    state
  end

  # A version's modules are deleted, which makes them old code, and purged; then the NIF builds it loaded are let go
  # of, and what it kept forgotten.
  defp free({:described, _artifact, _code} = under), do: :persistent_term.erase({__MODULE__, under})

  defp free(under) do
    prefix = Atom.to_string(under) <> "."

    for {module, _} <- :code.all_loaded(), String.starts_with?(Atom.to_string(module), prefix) do
      :code.delete(module)
      :code.purge(module)
    end

    Nif.release(under)
    :persistent_term.erase({__MODULE__, under})
  end

  defp name(artifact, code), do: Module.concat(OneCore.Apps, "V" <> OneCore.Apps.Version.code({artifact, code}))

  # The name a module of a version sits under, or `nil` for any other module.
  defp slot(module) do
    with [under] <- Regex.run(~r/^(Elixir\.OneCore\.Apps\.V[0-9a-f]{64})\./, Atom.to_string(module), capture: :all_but_first),
         do: String.to_atom(under)
  end

  defp loader(state, under), do: Enum.find_value(state.loaders, fn {monitor, {^under, pid}} -> {monitor, pid}; _ -> nil end)
  defp row(under), do: :ets.lookup_element(__MODULE__, under, 2)
  defp update(under, fun), do: :ets.insert(__MODULE__, {under, fun.(row(under))})
end
