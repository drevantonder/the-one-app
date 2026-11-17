defmodule OneCore.Plugins.Precompiled do
  @moduledoc """
  A plugin's builds of its NIFs and programs (`server/nif/<crate>/<kind>.so`, `server/bin/<crate>/<kind>`) published
  as files at URLs, as Elixir's rustler_precompiled publishes a library's, since its git repo keeps only their source.
  Its `server/checksums.json` names each build by its path in the plugin, with its `url` and `sha256`:

      {"server/nif/sqlite/linux-x86_64.so": {"url": "https://github.com/…/sqlite-<input hash>-linux-x86_64.so",
                                             "sha256": "…"}}

  So the commit a plugin is installed or updated from pins exactly which builds are trusted. A deploy that lacks a build
  it names, as one from git does, downloads it and checks it before anything loads it:

      named in checksums.json, not in the files ─► downloaded ─► its SHA-256 is the one named ─► deployed with it
                                                       └─ missing, or another SHA-256 ─► the deploy fails, saying which

  A build a deploy brings itself, as one from a checkout built with `tooling/nifs` does, is deployed as it is.
  """

  alias OneCore.Call.State

  @checksums "server/checksums.json"

  @doc """
  `files` with each build its `server/checksums.json` names that they lack, downloaded and checked: `{:ok, files}`,
  or `{:error, why}`.
  """
  def fill(files) do
    with {:ok, named} <- named(files[@checksums]) do
      missing = Map.drop(named, Map.keys(files))
      if missing != %{}, do: State.stage(State.id(), "Downloading its builds")

      missing
      |> Task.async_stream(&download/1, max_concurrency: 4, timeout: :infinity, ordered: false)
      |> Enum.reduce_while({:ok, files}, fn
        {:ok, {:ok, path, build}}, {:ok, files} -> {:cont, {:ok, Map.put(files, path, build)}}
        {:ok, {:error, why}}, _ -> {:halt, {:error, why}}
      end)
    end
  end

  defp named(nil), do: {:ok, %{}}

  defp named(json) do
    with {:ok, named} when is_map(named) <- JSON.decode(json),
         true <- Enum.all?(named, &build?/1) do
      {:ok, named}
    else
      _ ->
        {:error,
         "#{@checksums} names each build of a NIF or program by its path, server/nif/<crate>/<kind>.so or " <>
           "server/bin/<crate>/<kind>, with its https `url` and `sha256`"}
    end
  end

  defp build?({path, %{"url" => "https://" <> _, "sha256" => sha256}}),
    do:
      path =~ ~r{^server/(nif/[^/.]+/[a-z0-9_]+-[a-z0-9_]+\.so|bin/[^/.]+/[a-z0-9_]+-[a-z0-9_]+)$} and
        sha256 =~ ~r/^[0-9a-f]{64}$/

  defp build?(_), do: false

  defp download({path, %{"url" => url, "sha256" => sha256}}) do
    case Req.get(url, raw: true, retry: :transient) do
      {:ok, %{status: 200, body: build}} ->
        case Base.encode16(:crypto.hash(:sha256, build), case: :lower) do
          ^sha256 ->
            {:ok, path, build}

          other ->
            {:error, "#{path} from #{url} isn't the build #{@checksums} names: its SHA-256 is #{other}, not #{sha256}"}
        end

      {:ok, %{status: status}} ->
        {:error, "#{path} couldn't be downloaded from #{url}, which answered #{status}"}

      {:error, error} ->
        {:error, "#{path} couldn't be downloaded from #{url}: #{Exception.message(error)}"}
    end
  end
end
