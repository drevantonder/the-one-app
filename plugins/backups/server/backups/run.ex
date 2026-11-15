defmodule Backups.Run do
  @moduledoc """
  Backing up to a place, checking what's there, and restoring from it into the running network.

  A backup reads every blob in the bucket and sends each one the place hasn't been sent before, by what's in it,
  as an object of its own: the blob encrypted to the place's backup key for the object's name, which is random.
  Then it sends the backup's list of blobs, encrypted the same way, named by when the backup began. What was sent
  is kept here, so the place's objects are never read to know what's there.

  A database is a snapshot and a log after it, where a snapshot is written whole and then the ones before the
  newest two are removed, and the log is only ever added to (`Database.Log`). So the log is listed last, once the
  rest is read: every snapshot a backup holds has the log that follows it, and the backup holds the database as it
  was at some moment, whatever was written meanwhile.

  Old backups go by the rule `Backups` says, and with them the objects no backup kept holds any more.
  """
  alias Backups.Place
  alias OneCore.Bucket
  import Backups, only: [transaction: 2, query: 3]

  # How many blobs are read from the bucket at once.
  @together 32

  @doc "Backs up to `place` now, and says what was sent and how long it took. A failure is kept as its problem."
  def back_up(call, place) do
    began = System.os_time(:millisecond)

    try do
      opened = Place.open(call, place)

      known =
        transaction(call, fn db ->
          Map.new(
            query(db, "select sha256, object from objects where place = ?", [place["id"]]),
            &{&1["sha256"], &1["object"]}
          )
        end)

      # The log is read last, once everything else is (`@moduledoc`).
      log? = &(&1 =~ ~r{/log/\d+$})
      {listed, sent} = copy(opened, Enum.reject(Bucket.list(""), log?), known, {[], []})

      {listed, sent} =
        copy(
          opened,
          Enum.filter(Bucket.list(""), log?),
          Map.merge(known, Map.new(sent, &{&1.sha256, &1.object})),
          {listed, sent}
        )

      listed = Enum.reverse(listed)

      name = String.pad_leading(Integer.to_string(began), 16, "0")
      json = JSON.encode!(%{at: began, blobs: listed})
      {:ok, sealed} = Cipher.seal(place["recipient"], "one/backups/1/backup\n" <> name, json)
      Place.put(opened, "backups/" <> name, sealed)
      took = System.os_time(:millisecond) - began
      bytes = Enum.sum_by(listed, & &1.size)
      sent_bytes = Enum.sum_by(sent, & &1.stored)

      transaction(call, fn db ->
        for object <- sent do
          query(db, "insert or ignore into objects (place, object, sha256, stored, digest) values (?, ?, ?, ?, ?)", [
            place["id"],
            object.object,
            object.sha256,
            object.stored,
            object.digest
          ])
        end

        query(
          db,
          "insert into backups (place, name, at, took, blobs, bytes, sent, sent_bytes, stored, digest, listed) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
          [
            place["id"],
            name,
            began,
            took,
            length(listed),
            bytes,
            length(sent),
            sent_bytes,
            byte_size(sealed),
            digest(sealed),
            {:blob, :zlib.compress(json)}
          ]
        )

        query(db, "delete from problems where place = ?", [place["id"]])
      end)

      gone = prune(call, opened, place)

      "#{place["name"]}: backed up #{length(listed)} blobs, #{size(bytes)}, in #{seconds(took)}. Sent #{length(sent)} new, #{size(sent_bytes)}." <>
        if(gone > 0, do: " #{gone} older backups went, by the rule of what's kept.", else: "")
    rescue
      failed ->
        text = Exception.message(failed)

        transaction(call, fn db ->
          query(db, "insert or replace into problems (place, at, kind, text) values (?, ?, 'backup', ?)", [
            place["id"],
            began,
            text
          ])
        end)

        reraise "Couldn't back up to #{place["name"]}: #{text}", __STACKTRACE__
    end
  end

  # Reads `keys` from the bucket, a few at once, and sends what the place doesn't have: the blobs listed, and the
  # objects sent. A blob removed since it was listed isn't there to back up.
  defp copy(opened, keys, known, {listed, sent}) do
    keys
    |> Enum.chunk_every(@together)
    |> Enum.reduce({listed, sent, known}, fn keys, acc ->
      Enum.zip(keys, Bucket.read_all(keys))
      |> Enum.reduce(acc, fn
        {key, {:ok, data, _tag}}, {listed, sent, known} ->
          sha256 = digest(data)

          {object, sent, known} =
            case known do
              %{^sha256 => object} ->
                {object, sent, known}

              _ ->
                object = Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)
                {:ok, sealed} = Cipher.seal(opened.place["recipient"], "one/backups/1/object\n" <> object, data)
                Place.put(opened, "objects/" <> object, sealed)
                new = %{object: object, sha256: sha256, stored: byte_size(sealed), digest: digest(sealed)}
                {object, [new | sent], Map.put(known, sha256, object)}
            end

          {[%{key: key, object: object, size: byte_size(data), sha256: sha256} | listed], sent, known}

        {_key, :absent}, acc ->
          acc
      end)
    end)
    |> then(fn {listed, sent, _known} -> {listed, sent} end)
  end

  # Removes the backups the rule doesn't keep, and then the objects no backup kept holds. Gives how many went.
  defp prune(call, opened, place) do
    backups =
      transaction(call, &query(&1, "select name, at from backups where place = ? order by at desc", [place["id"]]))

    gone = backups -- kept(backups, System.os_time(:millisecond))

    if gone == [] do
      0
    else
      for %{"name" => name} <- gone, do: Place.remove(opened, "backups/" <> name)

      unheld =
        transaction(call, fn db ->
          for %{"name" => name} <- gone,
              do: query(db, "delete from backups where place = ? and name = ?", [place["id"], name])

          held =
            for %{"listed" => listed} <- query(db, "select listed from backups where place = ?", [place["id"]]),
                %{"object" => object} <- listed(listed),
                into: MapSet.new(),
                do: object

          for %{"object" => object} <- query(db, "select object from objects where place = ?", [place["id"]]),
              not MapSet.member?(held, object),
              do: object
        end)

      for object <- unheld, do: Place.remove(opened, "objects/" <> object)

      transaction(call, fn db ->
        for object <- unheld,
            do: query(db, "delete from objects where place = ? and object = ?", [place["id"], object])
      end)

      length(gone)
    end
  end

  @doc """
  The backups kept, of `backups` newest first, at `now`: every one for a day, then the newest of each day for a
  week, of each week for a month, and of each month for a year. The newest is always kept.
  """
  def kept([], _now), do: []

  def kept([newest | _] = backups, now) do
    day = 86_400_000

    {kept, _seen} =
      Enum.reduce(backups, {[], MapSet.new()}, fn %{"at" => at} = backup, {kept, seen} ->
        date = DateTime.to_date(DateTime.from_unix!(at, :millisecond))
        {year, week} = :calendar.iso_week_number(Date.to_erl(date))

        group =
          cond do
            now - at < day -> {:each, at}
            now - at < 7 * day -> {:day, date}
            now - at < 31 * day -> {:week, year, week}
            now - at < 365 * day -> {:month, date.year, date.month}
            true -> nil
          end

        if group && group not in seen, do: {[backup | kept], MapSet.put(seen, group)}, else: {kept, seen}
      end)

    Enum.uniq([newest | Enum.reverse(kept)])
  end

  @doc """
  Reads the newest backup at `place` back whole, its list of blobs and every object it holds, and compares each
  with what was sent. Says what it read, how long it took, and which blobs aren't as they were sent.
  """
  def check(call, place) do
    began = System.os_time(:millisecond)
    opened = Place.open(call, place)

    {backup, objects} =
      transaction(call, fn db ->
        {List.first(
           query(db, "select name, digest, listed from backups where place = ? order by at desc limit 1", [place["id"]])
         ),
         Map.new(
           query(db, "select object, digest from objects where place = ?", [place["id"]]),
           &{&1["object"], &1["digest"]}
         )}
      end)

    if backup, do: check(call, place, opened, began, backup, objects), else: "#{place["name"]} has no backup yet."
  end

  defp check(call, place, opened, began, backup, objects) do
    %{"name" => name, "digest" => listed_digest, "listed" => listed} = backup
    blobs = listed(listed)
    held = blobs |> Enum.map(& &1["object"]) |> Enum.uniq()

    read =
      held
      |> Task.async_stream(&{&1, Place.get(opened, "objects/" <> &1)}, max_concurrency: 8, timeout: :infinity)
      |> Enum.map(fn {:ok, read} -> read end)

    wrong = for {object, data} <- read, data == nil or digest(data) != objects[object], into: MapSet.new(), do: object
    listed_ok = digest(Place.get(opened, "backups/" <> name) || "") == listed_digest
    bytes = Enum.sum_by(read, fn {_object, data} -> byte_size(data || "") end)
    took = seconds(System.os_time(:millisecond) - began)
    said = "#{place["name"]}: read back backup #{name}, #{length(held)} objects, #{size(bytes)}, in #{took}."

    case {listed_ok, for(%{"key" => key, "object" => object} <- blobs, object in wrong, do: key)} do
      {true, []} ->
        said <> " Every one is as it was sent."

      {listed_ok, keys} ->
        not_as_sent =
          if(listed_ok, do: "", else: " Its list of blobs isn't as it was sent.") <>
            if(keys == [],
              do: "",
              else: " Not as sent: #{Enum.join(Enum.take(keys, 10), ", ")}#{if length(keys) > 10, do: " and more"}."
            )

        # What isn't there as it was sent goes, and so does what's kept of it here, so the next backup sends it
        # again.
        for object <- wrong, do: Place.remove(opened, "objects/" <> object)

        transaction(call, fn db ->
          for object <- wrong,
              do: query(db, "delete from objects where place = ? and object = ?", [place["id"], object])

          query(db, "insert or replace into problems (place, at, kind, text) values (?, ?, 'check', ?)", [
            place["id"],
            began,
            String.trim(not_as_sent) <> " The next backup sends it again."
          ])
        end)

        raise said <> not_as_sent
    end
  end

  @doc """
  Brings back what backup `backup` at `place` holds under `blobs` that's gone from the bucket, with the backup key a
  device handed over in `given`. Changes nothing that's there.
  """
  def restore(call, place, backup, blobs, given) do
    began = System.os_time(:millisecond)
    opened = Place.open(call, place)

    sealed =
      with {:ok, %{} = sealed} <- JSON.decode(given),
           {:ok, sealed} <- Base.decode64(sealed[call.server] || "") do
        sealed
      else
        _ -> raise "The network's backup key wasn't handed to this server. Ask again from your device."
      end

    {key, public} =
      with {:ok, own} <- Cipher.key(OneCore.Server.key_file()),
           {:ok, key} <- Cipher.open_keys(own, "one/backups/1/restoring\n" <> call.server, sealed),
           {:ok, public} <- Cipher.public(key) do
        {key, public}
      else
        {:error, _why} -> raise "The network's backup key wasn't handed to this server. Ask again from your device."
      end

    if public != place["recipient"],
      do: raise("#{place["name"]}'s backups are encrypted to another key than the network's.")

    name =
      backup ||
        List.last(Place.list(opened, "backups/")) ||
        raise("#{place["name"]} has no backup yet.")

    name = String.replace_prefix(name, "backups/", "")
    sealed = Place.get(opened, "backups/" <> name) || raise "#{place["name"]} has no backup #{name}."
    {:ok, listed} = open(key, "one/backups/1/backup\n" <> name, sealed)

    {gone, there} =
      JSON.decode!(listed)["blobs"]
      |> Enum.filter(&String.starts_with?(&1["key"], blobs))
      |> Enum.split_with(&(Bucket.size(&1["key"]) == :absent))

    for blob <- gone do
      sealed = Place.get(opened, "objects/" <> blob["object"]) || raise "#{blob["key"]} is missing from the backup."
      {:ok, data} = open(key, "one/backups/1/object\n" <> blob["object"], sealed)
      if digest(data) != blob["sha256"], do: raise("#{blob["key"]} isn't what was backed up: the backup was changed.")
      Bucket.write(blob["key"], data)
    end

    took = seconds(System.os_time(:millisecond) - began)

    case gone do
      [] ->
        "Nothing under #{blobs} is gone since backup #{name}: #{counted(length(there))} there, unchanged."

      gone ->
        "Brought back #{counted(length(gone))} from backup #{name}, #{size(Enum.sum_by(gone, & &1["size"]))}, in #{took}: " <>
          Enum.join(Enum.take(Enum.map(gone, & &1["key"]), 10), ", ") <>
          if(length(gone) > 10, do: " and more", else: "") <>
          if(there == [], do: ".", else: ". #{counted(length(there))} there already, unchanged.")
    end
  end

  defp open(key, about, sealed) do
    case Cipher.open(key, about, sealed) do
      {:ok, data} -> {:ok, data}
      {:error, _why} -> raise "Your backup key doesn't open that backup: it was changed."
    end
  end

  # The blobs a backup holds, as they're kept here.
  defp listed(kept), do: JSON.decode!(:zlib.uncompress(kept))["blobs"]

  defp digest(data), do: Base.encode16(:crypto.hash(:sha256, data), case: :lower)

  @doc false
  def size(bytes) when bytes < 1_000, do: "#{bytes} bytes"
  def size(bytes) when bytes < 1_000_000, do: "#{Float.round(bytes / 1.0e3, 1)} KB"
  def size(bytes) when bytes < 1_000_000_000, do: "#{Float.round(bytes / 1.0e6, 1)} MB"
  def size(bytes), do: "#{Float.round(bytes / 1.0e9, 2)} GB"

  defp counted(1), do: "1 blob"
  defp counted(count), do: "#{count} blobs"

  defp seconds(ms) when ms < 1000, do: "#{ms} ms"
  defp seconds(ms), do: "#{Float.round(ms / 1000, 1)} s"
end
