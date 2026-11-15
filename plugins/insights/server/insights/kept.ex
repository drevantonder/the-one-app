defmodule Insights.Kept do
  @moduledoc """
  Where what's recorded is kept: blobs of the plugin's own artifact for the network, by the hour they're of.

  - `batches/<day>/<hour>/<to>-<server>-<bytes>` is one server's batch, as it handed it on: what it recorded until
    `to`, in nanoseconds since 1970. A server that's busy writes about one a minute, and an idle one none.
  - `batches/<day>/<hour>/hour-<bytes>` is all of an hour that's over, put together once it is (`tidy/1`), with the
    batches it holds named in it, so it's put together once even if tidying stopped partway.

  Each is Erlang's term format, compressed, and its size is in its name, so listing them says how much is kept.
  What's older than `days/0`, and the oldest hours past `most_bytes/0`, are let go.
  """
  alias Insights.Read
  alias OneCore.Artifact

  @days 14
  @most_bytes 256 * 1024 * 1024

  @doc "How many days are kept."
  def days, do: @days

  @doc "How many bytes are kept at most."
  def most_bytes, do: @most_bytes

  @doc """
  Keeps a batch a server handed on. Each of its spans and logs says which server's it was, since an hour's are put
  together.
  """
  def keep(own, %{"server" => server, "to" => to} = batch) do
    mark = &Map.put(&1, "server", batch["machine"])
    batch = %{batch | "spans" => Enum.map(batch["spans"], mark), "logs" => Enum.map(batch["logs"], mark)}
    data = :erlang.term_to_binary(batch, [:compressed])
    Artifact.write(own, "#{hour(to)}/#{to}-#{String.slice(server, 0, 8)}-#{byte_size(data)}", data)
    :ok
  end

  @doc "Every batch kept of an hour that started after `from`, in nanoseconds since 1970, put together by hours."
  def read(own, from) do
    since = hour(from - 3_600_000_000_000)
    names = for name <- Artifact.list(own, "batches/"), Path.dirname(name) > since, do: name

    for {:ok, data, _tag} <- Artifact.read_all(own, names), do: :erlang.binary_to_term(data, [:safe])
  end

  @doc "How much is kept: since when, in milliseconds since 1970, how many bytes, for how many days, and the most."
  def kept(own) do
    names = Artifact.list(own, "batches/")

    %{
      since: Enum.find_value(names, &since/1),
      bytes: Enum.sum_by(names, &bytes/1),
      days: @days,
      most_bytes: @most_bytes
    }
  end

  @doc """
  Puts each hour that's over into one blob, and lets go of hours older than `days/0`, and then of the oldest while
  more than `most_bytes/0` is kept. Gives what it did.
  """
  def tidy(own) do
    now = System.os_time(:nanosecond)
    hours = Enum.group_by(Artifact.list(own, "batches/"), &Path.dirname/1)
    oldest = hour(now - @days * 86_400_000_000_000)

    put_together =
      for {hour, names} <- hours, hour >= oldest, hour < hour(now), Enum.any?(names, &(not hour?(&1))) do
        together(own, hour, names)
        hour
      end

    hours = Enum.group_by(Artifact.list(own, "batches/"), &Path.dirname/1)
    {old, kept} = Enum.split_with(Enum.sort(hours), fn {hour, _names} -> hour < oldest end)
    {over, _kept} = past_most(Enum.reverse(kept), 0)
    let_go = old ++ over
    for {_hour, names} <- let_go, name <- names, do: Artifact.remove(own, name)

    %{put_together: length(put_together), let_go: length(let_go)}
  end

  # An hour's batches as one blob, which says which batches it holds: a batch it holds already isn't added again.
  defp together(own, hour, names) do
    batches = for {:ok, data, _tag} <- Artifact.read_all(own, names), do: :erlang.binary_to_term(data, [:safe])
    {hours, batches} = Enum.split_with(batches, &Map.has_key?(&1, "holds"))
    held = MapSet.new(Enum.flat_map(hours, & &1["holds"]))
    added = Enum.reject(batches, &(id(&1) in held))

    merged = Read.merge(hours ++ added)
    merged = Map.put(merged, "holds", MapSet.to_list(held) ++ Enum.map(added, &id/1))
    data = :erlang.term_to_binary(merged, [:compressed])
    :ok = Artifact.write(own, "#{hour}/hour-#{byte_size(data)}", data)
    for name <- names, Path.basename(name) != "hour-#{byte_size(data)}", do: Artifact.remove(own, name)
  end

  # The newest hours that fit in `most_bytes/0`, and the rest.
  defp past_most([], _bytes), do: {[], []}

  defp past_most([{_hour, names} = newest | older], bytes) do
    bytes = bytes + Enum.sum_by(names, &bytes/1)

    if bytes > @most_bytes do
      {[newest | older], []}
    else
      {over, kept} = past_most(older, bytes)
      {over, [newest | kept]}
    end
  end

  defp id(batch), do: "#{batch["server"]}/#{batch["to"]}"
  defp hour?(name), do: String.starts_with?(Path.basename(name), "hour-")
  defp bytes(name), do: name |> String.split("-") |> List.last() |> String.to_integer()

  # When the oldest kept is from, in milliseconds since 1970: the start of its hour.
  defp since(name) do
    [_, day, hour | _] = String.split(name, "/")
    {:ok, at, 0} = DateTime.from_iso8601("#{day}T#{hour}:00:00Z")
    DateTime.to_unix(at, :millisecond)
  end

  # The folder of the hour a time in nanoseconds since 1970 is in: `batches/2026-11-15/07`.
  defp hour(at) do
    time = DateTime.from_unix!(div(at, 1_000_000_000))
    "batches/#{Date.to_iso8601(time)}/#{String.pad_leading(to_string(time.hour), 2, "0")}"
  end
end
