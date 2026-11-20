defmodule OneCore.Apps.Icons do
  @moduledoc """
  An app's icon as a client loads it over and over, as a Home Screen does: by an address named for what's in it,
  `ui://icons/<sha256>/icon.<ext>`, which a client reads as it reads an app's files (`OneCore.Mcp`) and keeps for
  good, rather than in every answer that shows it. Its bytes are kept in the bucket by that hash the first time this
  server gives its address, so any server reads it: written behind its answer, which this server meanwhile reads
  from what it holds. An icon is what the version said it is (`OneCore.Apps.Version`),
  so its address gives whoever has it nothing they weren't given already.
  """
  alias OneCore.Bucket
  alias OneCore.Apps.Version

  @doc "The address of an icon given as a `data:` address (`OneCore.Apps.Version`), or `nil` for none."
  def address(nil), do: nil

  def address("data:" <> _ = icon) do
    with nil <- :persistent_term.get({__MODULE__, icon}, nil) do
      [kind, base64] = icon |> String.replace_prefix("data:", "") |> String.split(";base64,", parts: 2)
      data = Base.decode64!(base64)
      hash = Base.encode16(:crypto.hash(:sha256, data), case: :lower)
      address = "ui://icons/#{hash}/icon.#{hd(MIME.extensions(kind))}"
      :persistent_term.put({__MODULE__, :data, hash}, data)
      # Its address is kept only once it's in the bucket, so one that couldn't be written is written the next time
      # it's given.
      Task.Supervisor.start_child(OneCore.Asks, fn ->
        if Bucket.write("icons/#{hash}", data) in [:ok, :exists], do: :persistent_term.put({__MODULE__, icon}, address)
      end)

      address
    end
  end

  @doc "The icon at `address`, as `address/1` gave it: `{:ok, kind, data}`, or `:absent`."
  def read("ui://icons/" <> named) do
    with [hash, file] <- String.split(named, "/", parts: 2),
         true <- hash =~ ~r/^[0-9a-f]{64}\z/,
         {:ok, data, _tag} <- held(hash) || Bucket.read("icons/#{hash}") do
      {:ok, Version.kind(file), data}
    else
      _ -> :absent
    end
  end

  # An icon's bytes as this server holds them, once it's given its address, as `Bucket.read/1` gives a blob, or `nil`.
  defp held(hash) do
    with data when is_binary(data) <- :persistent_term.get({__MODULE__, :data, hash}, nil), do: {:ok, data, nil}
  end
end
