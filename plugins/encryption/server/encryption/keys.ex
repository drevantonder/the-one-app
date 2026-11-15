defmodule Encryption.Keys do
  @moduledoc """
  One encrypted artifact's keys on this server, for as long as it's chosen to open it. They're what the artifact's
  blob `keys/<this server>` hands it, opened once with the server's own key and held in the NIF (`Cipher`),
  where nothing reads them. The server that records a choice tells the others to look at the blob at once
  (`Encryption.choose/3`), so one that was taken off lets go of its keys, and one that's handed a new epoch's
  takes them. One that wasn't reached then looks every few seconds while the artifact is in use. A server that
  holds none looks each time it's asked: it's about to ask another server anyway.

  Keys handed over anew are a new reference, and the one before opens nothing more. So whatever held the
  artifact open with the old one finds out, as a database does (`Database.Lease`): what a server had open before
  it was taken off isn't what it carries on with when it's chosen again.
  """
  use GenServer

  # How long this server goes without looking at what it was handed, in milliseconds: as long as it keeps that it
  # may reach something (`OneCore.Hooks`).
  @looks_after 10_000

  @impl true
  def init({artifact, server}), do: {:ok, %{artifact: artifact, server: server, keys: nil, slot: nil, looked: nil}}

  @impl true
  def handle_call(:keys, _from, state) do
    now = System.monotonic_time(:millisecond)
    state = if state.slot == nil or now - state.looked > @looks_after, do: look(state, now), else: state
    {:reply, if(state.slot, do: {:ok, state.keys}, else: :none), state}
  end

  def handle_call(:look, _from, state), do: {:reply, :ok, look(state, System.monotonic_time(:millisecond))}
  def handle_call(:forget, _from, state), do: {:reply, :ok, forgotten(%{state | looked: nil})}

  defp look(state, now) do
    name = OneCore.Artifact.name(state.artifact)
    state = %{state | looked: now}

    case OneCore.Artifact.read(state.artifact, "keys/" <> state.server) do
      {:ok, slot, _tag} when slot == state.slot ->
        state

      {:ok, slot, _tag} ->
        state = forgotten(state)

        case Cipher.keys(OneCore.Server.key_file(), name, slot) do
          {:ok, keys} -> %{state | keys: keys, slot: slot}
          {:error, _why} -> state
        end

      :absent ->
        forgotten(state)
    end
  end

  defp forgotten(state) do
    if state.keys, do: Cipher.forget(state.keys)
    %{state | keys: nil, slot: nil}
  end
end
