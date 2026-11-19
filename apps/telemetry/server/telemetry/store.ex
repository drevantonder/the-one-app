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
  - `otlp/<day>/<hour>/hour-<at>-<bytes>.rollup` is that hour rolled up (`Telemetry.Rollup`) as it was at `at`, naming
    the exports it holds: what a look over hours reads of it, but a trace. Alerts roll up what they read each minute
    (`roll_up/2`), and an hour that's over is rolled up whole before it's put together (`prune/1`).

  Its size is in its name, so listing them says how much is kept. What's older than `days/0`, and the oldest hours
  past `most_bytes/0`, are let go. What was kept before it was OTLP, under `batches/`, isn't read, and is let go as
  it ages.
  """
  require Logger
  require OpenTelemetry.Tracer, as: Tracer
  alias Telemetry.{Cache, Otlp, Rollup}
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

  @doc """
  What's kept of each hour that started after `from`, in nanoseconds since 1970, laid flat: an hour that's rolled up
  as its rollup (`Telemetry.Rollup`), and any other as its exports, all that `Telemetry.Read` reads but a trace whole.
  """
  def read(call, from) do
    hours = call |> listed(from) |> Enum.group_by(&Path.dirname/1) |> Map.values()
    rollups = for names <- hours, rollup = latest(names), do: rollup
    read = fn _name, data -> Rollup.decode(data) end
    rolled = Tracer.with_span("telemetry rollups", %{attributes: %{"one.blobs" => length(rollups)}}, do: Cache.read(call, rollups, read))
    exports = Enum.flat_map(hours, &unrolled(&1, rolled))
    read = &Rollup.of(List.wrap(export(&1, &2)))
    rolled ++ Tracer.with_span("telemetry exports", %{attributes: %{"one.blobs" => length(exports)}}, do: Cache.read(call, exports, read))
  end

  @doc "Every export kept of an hour that started after `from`, in nanoseconds since 1970, each laid flat and whole."
  def whole(call, from), do: decode(call, Enum.reject(listed(call, from), &rollup?/1))

  defp listed(call, from) do
    since = hour(from - 3_600_000_000_000)
    listed = Tracer.with_span("telemetry list", do: Artifact.list(call.own, "otlp/"))
    for name <- listed, Path.dirname(name) > since, do: name
  end

  @doc """
  Every export that ended after `from`, in nanoseconds since 1970, that isn't among `read`, by its name: the names,
  and each export laid flat with its name, read from the hours they're in before those are put together (`prune/1`).
  One put together meanwhile is named, and not given.
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

    {names, decoded(call.own, names)}
  end

  @doc """
  Rolls the exports `read`, each laid flat with its name, into their hours' rollups, those it doesn't hold yet: an
  hour's rollup is written again holding them too, and the one before it is let go. One whose rollup was let go as
  it was read is left as it is, read as an export until the hour is rolled up whole (`prune/1`).
  """
  def roll_up(_own, []), do: :ok

  def roll_up(own, read) do
    for {hour, read} <- Enum.group_by(read, fn {name, _batch} -> Path.dirname(name) end) do
      before = latest(Artifact.list(own, hour <> "/"))

      with {:ok, held} <- held(own, before),
           names = Map.get(held || %{}, "exports", []),
           [_ | _] = added <- Enum.reject(read, fn {name, _batch} -> name in names end) do
        rollup = Map.put(Rollup.of(List.wrap(held) ++ Enum.map(added, &elem(&1, 1))), "exports", names ++ Enum.map(added, &elem(&1, 0)))
        data = Rollup.encode(rollup)
        :ok = Artifact.write(own, "#{hour}/hour-#{System.os_time(:nanosecond)}-#{byte_size(data)}.rollup", data)
        if before, do: Artifact.remove(own, before)
      end
    end

    :ok
  end

  # The rollup kept as `name`, or `nil` with none.
  defp held(_own, nil), do: {:ok, nil}

  defp held(own, name) do
    with {:ok, data, _tag} <- Artifact.read(own, name), do: {:ok, Rollup.decode(data)}
  end

  @doc "The exports kept as `names`, each by its name, as they're kept: OTLP's protobuf."
  def raw(call, names), do: for({name, {:ok, data, _tag}} <- Enum.zip(names, Artifact.read_all(call.own, names)), do: {name, data})

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

    # An hour is rolled up whole and put together 10 minutes after it ends, once alerts have read its last exports
    # one by one (`since/3`, `Telemetry.Alerts`).
    ended =
      for {hour, names} <- Enum.group_by(Artifact.list(own, "otlp/"), &Path.dirname/1),
          hour >= oldest,
          hour < hour(now - 600_000_000_000) do
        exports = Enum.reject(names, &(rollup?(&1) or hour?(&1)))
        # An hour with nothing but its rollup and its blobs put together holds nothing unrolled.
        unrolled = if latest(names) && exports == [], do: [], else: unrolled(names, held_by(own, names))
        roll_up(own, decoded(own, unrolled))
        for {signal, names} <- Enum.group_by(Enum.reject(names, &rollup?/1), &Path.extname/1), exports != [], do: together(own, hour, signal, names)
        {unrolled != [], exports != []}
      end

    hours = Enum.group_by(list(own), &folder/1)
    {old, retained} = Enum.split_with(Enum.sort(hours), fn {hour, _names} -> hour < oldest end)
    {over, _retained} = past_most(Enum.reverse(retained), 0)
    let_go = old ++ over
    for {_hour, names} <- let_go, name <- names, do: Artifact.remove(own, name)

    %{put_together: Enum.count(ended, &elem(&1, 1)), rolled_up: Enum.count(ended, &elem(&1, 0)), let_go: length(let_go)}
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
    Tracer.with_span "telemetry fetch", %{attributes: %{"one.blobs" => length(names)}} do
      for {_name, batch} <- decoded(call.own, names), do: batch
    end
  end

  # Each of the exports `names` laid flat, with its name.
  defp decoded(own, names),
    do: for({name, {:ok, data, _tag}} <- Enum.zip(names, Artifact.read_all(own, names)), batch = export(name, data), do: {name, batch})

  @doc "The export kept as `name`, laid flat, or `nil` where it isn't OTLP."
  def export(name, data) do
    Otlp.decode(signal(name), data)
  rescue
    ErlangError ->
      Logger.warning("Telemetry couldn't read #{name}, #{byte_size(data)} bytes starting #{inspect(binary_part(data, 0, min(8, byte_size(data))))}: it isn't OTLP")
      nil
  end

  # An hour's latest rollup, by the names kept in it, or `nil`.
  defp latest(names), do: names |> Enum.filter(&rollup?/1) |> Enum.max_by(&to_of/1, fn -> nil end)

  # The rollups of an hour, by the names kept in it, read.
  defp held_by(own, names), do: for(rollup <- List.wrap(latest(names)), {:ok, held} <- [held(own, rollup)], do: held)

  # What of an hour, by the names kept in it, isn't in its rollup, one of `rolled`: each export it doesn't name, or
  # where it has none, all the hour holds. A rolled-up hour's exports put together (`prune/1`) are in its rollup.
  defp unrolled(names, rolled) do
    held = MapSet.new(Enum.flat_map(rolled, &Map.get(&1, "exports", [])))
    rolled_up = latest(names) != nil
    for name <- names, not rollup?(name), not (rolled_up and hour?(name)), not MapSet.member?(held, name), do: name
  end

  defp rollup?(name), do: Path.extname(name) == ".rollup"
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
