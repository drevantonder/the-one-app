defmodule Telemetry.Store do
  @moduledoc """
  Where what's recorded is kept: blobs of the app's own artifact for the network, by the hour they're of, each an OTLP
  export request in its protobuf (`Telemetry.Otlp`), which any OpenTelemetry tool takes as it is:

      curl --data-binary @<blob> -H 'content-type: application/x-protobuf' http://localhost:4318/v1/traces

  - `otlp/<day>/<hour>/<to>-<source>-<bytes>.<signal>` is the exports of `traces`, `metrics` or `logs` that the server
    `source` held until `to`, in nanoseconds since 1970, its own SDK's and its clients' (`Telemetry.Buffer`). A busy
    server writes about two a minute of each, and an idle one none.
  - `otlp/<day>/<hour>/hour-<to>-<bytes>.<signal>` is all of an hour's exports of a signal, once the hour is over
    (`prune/1`): protobuf's messages put one after the other are one message holding all of theirs, so it's still one
    export request. It holds every export of its hour up to `to`, so one left over by a tidying that stopped partway
    is let go, not put in again.

  Its size is in its name, so listing them says how much is kept. What's older than `days/0`, and the oldest hours
  past `most_bytes/0`, are let go. What was kept before it was OTLP, under `batches/`, isn't read, and is let go as
  it ages.
  """
  require Logger
  require OpenTelemetry.Tracer, as: Tracer
  alias Telemetry.Otlp
  alias OneCore.Artifact

  @days 14
  @most_bytes 256 * 1024 * 1024

  @doc "How many days are kept."
  def days, do: @days

  @doc "How many bytes are kept at most."
  def most_bytes, do: @most_bytes

  @doc "Keeps an OTLP export request of `signal` that the server `source` held."
  def keep(own, source, signal, otlp) when is_binary(otlp) do
    to = System.os_time(:nanosecond)
    :ok = Artifact.write(own, "#{hour(to)}/#{to}-#{source}-#{byte_size(otlp)}.#{signal}", otlp)
  end

  @doc "Every export kept of an hour that started after `from`, in nanoseconds since 1970, each laid flat."
  def read(call, from) do
    since = hour(from - 3_600_000_000_000)
    listed = Tracer.with_span("telemetry list", do: Artifact.list(call.own, "otlp/"))
    names = for name <- listed, Path.dirname(name) > since, do: name
    decode(call, names)
  end

  @doc """
  Every export that ended after `from`, in nanoseconds since 1970, that isn't among `read`, by its name: the names,
  and the exports, as each was kept, read from the hours they're in before those are put together (`prune/1`). One
  put together meanwhile is named, and not given.
  """
  def since(call, from, read) do
    to = System.os_time(:nanosecond)
    hours = Enum.uniq(for(at <- from..to//3_600_000_000_000, do: hour(at)) ++ [hour(to)])

    names =
      for hour <- hours,
          name <- Artifact.list(call.own, hour <> "/"),
          not hour?(name),
          to_of(name) > from,
          name not in read,
          do: name

    {names, decode(call, names)}
  end

  @doc "The exports kept as `names`, each by its name, as they're kept: OTLP's protobuf."
  def raw(call, names), do: for({name, {:ok, data, _tag}} <- Enum.zip(names, read_all(call, names)), do: {name, data})

  @doc "The signal an export kept as `name` is of, `traces`, `metrics` or `logs`, or `nil` for one kept before OTLP."
  def signal("otlp/" <> _ = name), do: String.trim_leading(Path.extname(name), ".")
  def signal(_name), do: nil

  @doc "How much is kept: since when, in milliseconds since 1970, how many bytes, for how many days, and the most."
  def retention(own) do
    names = list(own)

    %{
      since: names |> Enum.map(&since/1) |> Enum.min(fn -> nil end),
      bytes: Enum.sum_by(names, &bytes/1),
      days: @days,
      most_bytes: @most_bytes
    }
  end

  @doc """
  Puts each hour that's over into one blob of each signal, and lets go of hours older than `days/0`, and then of the
  oldest while more than `most_bytes/0` is kept. Gives what it did.
  """
  def prune(own) do
    now = System.os_time(:nanosecond)
    oldest = hour(now - @days * 86_400_000_000_000)

    # An hour is put together 10 minutes after it ends, once alerts have read its last exports one by one
    # (`since/3`, `Telemetry.Alerts`).
    put_together =
      for {hour, names} <- Enum.group_by(Artifact.list(own, "otlp/"), &Path.dirname/1),
          hour >= oldest,
          hour < hour(now - 600_000_000_000),
          Enum.any?(names, &(not hour?(&1))) do
        for {signal, names} <- Enum.group_by(names, &Path.extname/1), do: together(own, hour, signal, names)
        hour
      end

    hours = Enum.group_by(list(own), &folder/1)
    {old, retained} = Enum.split_with(Enum.sort(hours), fn {hour, _names} -> hour < oldest end)
    {over, _retained} = past_most(Enum.reverse(retained), 0)
    let_go = old ++ over
    for {_hour, names} <- let_go, name <- names, do: Artifact.remove(own, name)

    %{put_together: length(put_together), let_go: length(let_go)}
  end

  # An hour's exports of a signal as one blob, after what it held already: an export it holds isn't added again.
  defp together(own, hour, signal, names) do
    {hours, exports} = Enum.split_with(names, &hour?/1)
    held = Enum.max(Enum.map(hours, &to_of/1), fn -> 0 end)
    {added, gone} = Enum.split_with(exports, &(to_of(&1) > held))

    if added != [] do
      data = for({:ok, data, _tag} <- Artifact.read_all(own, hours ++ added), do: data) |> IO.iodata_to_binary()
      to = Enum.max(Enum.map(hours ++ added, &to_of/1))
      :ok = Artifact.write(own, "#{hour}/hour-#{to}-#{byte_size(data)}#{signal}", data)
    end

    for name <- if(added == [], do: gone, else: names), do: Artifact.remove(own, name)
  end

  # The newest hours that fit in `most_bytes/0`, and the rest.
  defp past_most([], _bytes), do: {[], []}

  defp past_most([{_hour, names} = latest | older], bytes) do
    bytes = bytes + Enum.sum_by(names, &bytes/1)

    if bytes > @most_bytes do
      {[latest | older], []}
    else
      {over, retained} = past_most(older, bytes)
      {over, [latest | retained]}
    end
  end

  defp list(own), do: Artifact.list(own, "otlp/") ++ Artifact.list(own, "batches/")

  # Each kept export laid flat, by its signal. One that isn't OTLP, as a device may send anything, is passed over.
  defp decode(call, names) do
    read = Tracer.with_span("telemetry fetch", %{attributes: %{"one.blobs" => length(names)}}, do: read_all(call, names))

    Tracer.with_span "telemetry decode", %{attributes: %{"one.bytes" => Enum.sum(for {:ok, data, _tag} <- read, do: byte_size(data))}} do
      for {name, {:ok, data, _tag}} <- Enum.zip(names, read), batch = export(name, data), do: batch
    end
  end

  # The blobs `names`, in order, as `OneCore.Artifact.read_all/2` gives them. The exports of hours not yet put
  # together, many small ones that every look reads again, are kept between calls (`Telemetry.Exports`) for this hour
  # and the last.
  defp read_all(call, names) do
    {hours, exports} = Enum.split_with(names, &hour?/1)
    since = hour(System.os_time(:nanosecond) - 3_600_000_000_000)
    read = Map.new(Enum.zip(hours, Artifact.read_all(call.own, hours)) ++ Enum.zip(exports, Telemetry.Exports.read(call, exports, since)))
    Enum.map(names, &read[&1])
  end

  defp export(name, data) do
    Otlp.decode(signal(name), data)
  rescue
    ErlangError ->
      Logger.warning("Telemetry couldn't read #{name}, #{byte_size(data)} bytes starting #{inspect(binary_part(data, 0, min(8, byte_size(data))))}: it isn't OTLP")
      nil
  end

  defp folder(name), do: name |> Path.dirname() |> String.replace_prefix("batches/", "otlp/")
  defp hour?(name), do: String.starts_with?(Path.basename(name), "hour-")
  defp to_of(name), do: name |> Path.basename() |> String.trim_leading("hour-") |> String.split("-") |> hd() |> String.to_integer()
  defp bytes(name), do: name |> Path.rootname() |> String.split("-") |> List.last() |> String.to_integer()

  # When the oldest kept is from, in milliseconds since 1970: the start of its hour.
  defp since(name) do
    [_, day, hour | _] = String.split(name, "/")
    {:ok, at, 0} = DateTime.from_iso8601("#{day}T#{hour}:00:00Z")
    DateTime.to_unix(at, :millisecond)
  end

  # The folder of the hour a time in nanoseconds since 1970 is in: `otlp/2026-11-15/07`.
  defp hour(at) do
    time = DateTime.from_unix!(div(at, 1_000_000_000))
    "otlp/#{Date.to_iso8601(time)}/#{String.pad_leading(to_string(time.hour), 2, "0")}"
  end
end
