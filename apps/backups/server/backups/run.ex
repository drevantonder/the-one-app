defmodule Backups.Run do
  @moduledoc """
  Backing up to a destination, checking what's there, and restoring from it into the running network.

  A backup lists the bucket with each blob's tag (`OneCore.Bucket.list_tagged/1`), and reads only the blobs whose tag
  isn't the one the last backup there had, as restic reads only files whose metadata changed: a blob with the same tag
  is the same blob. It sends each one the destination hasn't been sent before, by what's in it, as an object of its
  own: the blob as it is, under a random name. Then it sends the backup's list of blobs, named by when the backup
  began. What was sent is kept here, so the destination's objects are never read to know what's there.

  A database is a snapshot and a log after it, where a snapshot is written whole and then the ones before the
  newest two are removed, and the log is only ever added to (`Database.Log`). So the log is listed last, once the
  rest is read: every snapshot a backup holds has the log that follows it, and the backup holds the database as it
  was at some moment, whatever was written meanwhile.

  Old backups go by the rule `Backups` says, and with them the objects no backup kept holds any more.

  A backup under way is a row of `runs`, written before anything is sent, so every app and agent sees it, and a second
  one to the same destination, as the hourly job and Back Up Now at once, is refused. It goes when the backup is done
  or has failed, and a failure is kept as the destination's problem. Its server writes `beat` every 30 seconds while
  it runs: a row whose `beat` is older than `@stale` was cut short, as by its server restarting, and the next backup
  takes its place, sending only what the cut-short one didn't. Each blob read is kept in `listed` as it's sent, so
  the next one reads only what the cut-short one didn't either. A timer has `due` called every few minutes while a
  backup is under way (`Database.timer/5`), so one cut short carries on within minutes, not at the next hour.

  A backup is made where it began. Its database's lease may move to another server meanwhile (a server restarting,
  or another that couldn't reach it): a call's transaction then has the server holding it make the whole call again,
  which for a backup is a second one beside this one, refused while this one beats. So once it's claimed, its
  transactions are never passed on, as a scheduled job's aren't (`forwarded`): one the lease moved away from is cut
  short where it is, and carries on from there on the server holding it.
  """
  alias Backups.Destination
  alias OneCore.Bucket
  import Backups, only: [transaction: 2, query: 3]

  # How many blobs are read from the bucket at once.
  @together 32
  # How often a backup under way says it still is, and how long after it last did it's taken to have been cut short.
  @beat 30_000
  @stale 120_000

  @doc "How long after its last `beat` a backup under way is taken to have been cut short, in milliseconds."
  def stale, do: @stale

  @doc """
  Backs up to `destination` now, and says what was sent and how long it took, or `:running` when a backup there is
  under way already. A failure is kept as its problem.
  """
  def back_up(call, destination) do
    began = System.os_time(:millisecond)

    # Its keys are read, and its timer set, before it's claimed: either may have the server that can do it make the
    # whole call, and nothing's been done yet.
    OneCore.Call.stage(call, "Opening #{destination["name"]}")
    opened = failing(call, destination, began, fn -> Destination.open(call, destination) end)
    OneCore.Call.app(call, "database").timer(call, "due", %{}, div(began + @stale, 1000) + 60, div(@stale, 1000))

    started =
      transaction(call, fn db ->
        query(
          db,
          "insert into runs (destination, server, at, beat) values (?, ?, ?, ?) on conflict (destination) do update set server = excluded.server, at = excluded.at, beat = excluded.beat where runs.beat < ? returning destination",
          [destination["id"], call.server, began, began, began - @stale]
        )
      end)

    if started == [], do: :running, else: run(%{call | forwarded: true}, opened, began)
  end

  # Does `fun`, and keeps what it raised as the destination's problem, raising it again. A scheduled backup's call is
  # passed on to no other server, so one that needs another, as one whose keys are in a Vault only another opens, fails
  # here too.
  defp failing(call, destination, began, fun) do
    try do
      fun.()
    catch
      :throw, {OneCore.Call, :forwarding_loop} ->
        raise "#{OneCore.Servers.name(call.server) || "This server"}, which makes the scheduled backups, can't do it " <>
                "alone: the network's Vault, which holds its keys, opens only on another server. Choose the servers " <>
                "that open it in Vault › Security."
    end
  rescue
    failed ->
      text = Exception.message(failed)

      transaction(call, fn db ->
        query(db, "insert or replace into problems (destination, at, kind, text) values (?, ?, 'backup', ?)", [destination["id"], began, text])
      end)

      OneCore.Call.changed(call)
      reraise "Couldn't back up to #{destination["name"]}: #{text}", __STACKTRACE__
  end

  # `call` is passed on to no other server (`@moduledoc`).
  defp run(call, %{destination: destination} = opened, began) do
    id = destination["id"]
    OneCore.Call.changed(call)
    beat = spawn_link(fn -> beat(call, id, began) end)

    try do
      {known, last, kept} =
        transaction(call, fn db ->
          {Map.new(
             query(db, "select sha256, object from objects where destination = ?", [destination["id"]]),
             &{&1["sha256"], &1["object"]}
           ),
           query(db, "select listed from backups where destination = ? order by at desc limit 1", [destination["id"]]),
           query(db, "select key, tag, object, size, sha256 from listed where destination = ?", [destination["id"]])}
        end)

      # What the last backup there held of each blob by its key and tag, and what a backup cut short read since,
      # where what it was sent as is still there.
      unchanged =
        for %{"key" => key, "tag" => tag, "sha256" => sha256} = blob <- Enum.flat_map(last, &listed(&1["listed"])) ++ kept,
            known[sha256] == blob["object"],
            into: %{},
            do: {{key, tag}, %{key: key, tag: tag, object: blob["object"], size: blob["size"], sha256: sha256}}

      # The log is read last, once everything else is (`@moduledoc`).
      log? = &(elem(&1, 0) =~ ~r{/log/\d+$})
      {listed, sent} = copy(call, opened, Enum.reject(Bucket.list_tagged(""), log?), {known, unchanged}, {[], []})

      {listed, sent} =
        copy(
          call,
          opened,
          Enum.filter(Bucket.list_tagged(""), log?),
          {Map.merge(known, Map.new(sent, &{&1.sha256, &1.object})), unchanged},
          {listed, sent}
        )

      listed = Enum.reverse(listed)

      name = String.pad_leading(Integer.to_string(began), 16, "0")
      json = JSON.encode!(%{at: began, blobs: listed})
      Destination.put(opened, "backups/" <> name, json)
      took = System.os_time(:millisecond) - began
      bytes = Enum.sum_by(listed, & &1.size)
      sent_bytes = Enum.sum_by(sent, & &1.stored)

      transaction(call, fn db ->
        claimed!(db, id, began)

        query(
          db,
          "insert into backups (destination, name, at, took, blobs, bytes, sent, sent_bytes, stored, digest, listed) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
          [
            destination["id"],
            name,
            began,
            took,
            length(listed),
            bytes,
            length(sent),
            sent_bytes,
            byte_size(json),
            digest(json),
            {:blob, :zlib.compress(json)}
          ]
        )

        for table <- ~w(problems listed), do: query(db, "delete from #{table} where destination = ?", [id])
      end)

      # The run holds `runs` until it has pruned, as restic holds its lock: a backup started meanwhile would send
      # objects that no backup in `backups` holds yet, which pruning removes.
      gone = prune(call, opened, destination)
      transaction(call, &query(&1, "delete from runs where destination = ?", [id]))
      OneCore.Call.changed(call)

      %{
        destination: destination["name"],
        name: name,
        at: began,
        took: took,
        blobs: length(listed),
        bytes: bytes,
        sent: length(sent),
        sent_bytes: sent_bytes,
        gone: gone
      }
    rescue
      failed ->
        text = Exception.message(failed)

        # Kept where it can be: where the lease has moved meanwhile, the next backup takes this one's place as for
        # one cut short.
        moved(fn ->
          transaction(call, fn db ->
            query(db, "insert or replace into problems (destination, at, kind, text) values (?, ?, 'backup', ?)", [
              destination["id"],
              began,
              text
            ])

            query(db, "delete from runs where destination = ? and at = ?", [id, began])
          end)
        end)

        # A call that fails tells no app that anything changed, as one that succeeds does once it's over.
        OneCore.Call.changed(call)
        reraise "Couldn't back up to #{destination["name"]}: #{text}", __STACKTRACE__
    catch
      :throw, {OneCore.Call, :forwarding_loop} ->
        OneCore.Call.changed(call)

        raise "The backup to #{destination["name"]} was cut short: its database moved to another server. It carries " <>
                "on from where it stopped within #{div(2 * @stale, 60_000)} minutes."
    after
      Process.unlink(beat)
      Process.exit(beat, :kill)
    end
  end

  # Runs `fun`, a transaction of the run's, unless the database's lease has moved to another server.
  defp moved(fun) do
    fun.()
  catch
    :throw, {OneCore.Call, :forwarding_loop} -> :moved
  end

  # Raises if the backup that began `at` is no longer the one under way at destination `id`: it was taken to have
  # been cut short, and another took its place.
  defp claimed!(db, id, at) do
    if query(db, "select 1 from runs where destination = ? and at = ?", [id, at]) == [],
      do: raise("Another backup took its place, after it said nothing for #{div(@stale, 60_000)} minutes.")
  end

  # Says the backup to `id` that began `at` is still under way, every `@beat`, for as long as the backup's process
  # runs: it's linked to it, so it ends with it, however that ends. It stops saying so once the database has moved
  # to another server, and the backup finds that out at its next transaction.
  defp beat(call, id, at) do
    receive do
    after
      @beat ->
        said =
          moved(fn ->
            transaction(call, &query(&1, "update runs set beat = ? where destination = ? and at = ?", [System.os_time(:millisecond), id, at]))
          end)

        if said != :moved, do: beat(call, id, at)
    end
  end

  # Reads the blobs `tagged` from the bucket that aren't `unchanged` since the last backup, a few at once, and sends
  # what the destination doesn't have: the blobs listed, and the objects sent. A blob removed since it was listed
  # isn't there to back up.
  defp copy(call, opened, tagged, {known, unchanged}, {listed, sent}) do
    {kept, changed} = Enum.split_with(tagged, &is_map_key(unchanged, &1))
    listed = Enum.reduce(kept, listed, &[unchanged[&1] | &2])
    id = opened.destination["id"]

    changed
    |> Enum.map(&elem(&1, 0))
    |> Enum.chunk_every(@together)
    |> Enum.reduce({listed, sent, known}, fn keys, {listed, sent, known} ->
      OneCore.Call.stage(call, "Backing up to #{opened.destination["name"]}: #{length(listed)} blobs listed")

      {read, new, known} =
        for {key, {:ok, data, tag}} <- Enum.zip(keys, Bucket.read_all(keys)), reduce: {[], [], known} do
          {read, new, known} ->
            sha256 = digest(data)

            {object, new, known} =
              case known do
                %{^sha256 => object} ->
                  {object, new, known}

                _ ->
                  object = Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)
                  {object, [{object, data} | new], Map.put(known, sha256, object)}
              end

            {[%{key: key, tag: tag, object: object, size: byte_size(data), sha256: sha256} | read], new, known}
        end

      for {object, data} <- Enum.reverse(new), do: Destination.put(opened, "objects/" <> object, data)

      new = for {object, data} <- new, do: %{object: object, sha256: digest(data), stored: byte_size(data), digest: digest(data)}

      # What was sent and read is kept as it's sent, so a backup cut short, as by its server restarting, sends and
      # reads none of it again.
      transaction(call, fn db ->
        for object <- new do
          query(db, "insert or ignore into objects (destination, object, sha256, stored, digest) values (?, ?, ?, ?, ?)", [
            id,
            object.object,
            object.sha256,
            object.stored,
            object.digest
          ])
        end

        for blob <- read do
          query(db, "insert or replace into listed (destination, key, tag, object, size, sha256) values (?, ?, ?, ?, ?, ?)", [
            id,
            blob.key,
            blob.tag,
            blob.object,
            blob.size,
            blob.sha256
          ])
        end
      end)

      {read ++ listed, new ++ sent, known}
    end)
    |> then(fn {listed, sent, _known} -> {listed, sent} end)
  end

  # Removes the backups the rule doesn't keep, and then the objects no backup kept holds. Gives how many went.
  defp prune(call, opened, destination) do
    backups =
      transaction(call, &query(&1, "select name, at from backups where destination = ? order by at desc", [destination["id"]]))

    gone = backups -- retained(backups, System.os_time(:millisecond))

    if gone == [] do
      0
    else
      for %{"name" => name} <- gone, do: Destination.remove(opened, "backups/" <> name)

      unheld =
        transaction(call, fn db ->
          for %{"name" => name} <- gone,
              do: query(db, "delete from backups where destination = ? and name = ?", [destination["id"], name])

          held =
            for %{"listed" => listed} <- query(db, "select listed from backups where destination = ?", [destination["id"]]),
                %{"object" => object} <- listed(listed),
                into: MapSet.new(),
                do: object

          for %{"object" => object} <- query(db, "select object from objects where destination = ?", [destination["id"]]),
              not MapSet.member?(held, object),
              do: object
        end)

      for object <- unheld, do: Destination.remove(opened, "objects/" <> object)

      transaction(call, fn db ->
        for object <- unheld,
            do: query(db, "delete from objects where destination = ? and object = ?", [destination["id"], object])
      end)

      length(gone)
    end
  end

  @doc """
  The backups kept, of `backups` newest first, at `now`: every one for a day, then the newest of each day for a
  week, of each week for a month, and of each month for a year. The newest is always kept.
  """
  def retained([], _now), do: []

  def retained([latest | _] = backups, now) do
    day = 86_400_000

    {retained, _seen} =
      Enum.reduce(backups, {[], MapSet.new()}, fn %{"at" => at} = backup, {retained, seen} ->
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

        if group && group not in seen, do: {[backup | retained], MapSet.put(seen, group)}, else: {retained, seen}
      end)

    Enum.uniq([latest | Enum.reverse(retained)])
  end

  @doc """
  Reads the newest backup at `destination` back whole, its list of blobs and every object it holds, and compares each
  with what was sent. Says what it read, how long it took, and which blobs aren't as they were sent.
  """
  def check(call, destination) do
    began = System.os_time(:millisecond)
    opened = Destination.open(call, destination)

    {backup, objects} =
      transaction(call, fn db ->
        {List.first(
           query(db, "select name, digest, listed from backups where destination = ? order by at desc limit 1", [destination["id"]])
         ),
         Map.new(
           query(db, "select object, digest from objects where destination = ?", [destination["id"]]),
           &{&1["object"], &1["digest"]}
         )}
      end)

    if backup,
      do: check(call, destination, opened, began, backup, objects),
      else: %{destination: destination["name"], backup: nil}
  end

  defp check(call, destination, opened, began, backup, objects) do
    %{"name" => name, "digest" => listed_digest, "listed" => listed} = backup
    blobs = listed(listed)
    held = blobs |> Enum.map(& &1["object"]) |> Enum.uniq()

    # Each object is hashed as it's read and let go, so a check holds a few objects at a time, however large the
    # backup.
    read =
      held
      |> Task.async_stream(&read_back(opened, &1), max_concurrency: 8, timeout: :infinity)
      |> Enum.map(fn {:ok, read} -> read end)

    wrong = for {object, _bytes, digest} <- read, digest != objects[object], into: MapSet.new(), do: object
    listed_ok = digest(Destination.get(opened, "backups/" <> name) || "") == listed_digest
    bytes = Enum.sum_by(read, fn {_object, bytes, _digest} -> bytes end)
    took = System.os_time(:millisecond) - began
    message = "#{destination["name"]}: read back backup #{name}, #{length(held)} objects, #{size(bytes)}, in #{seconds(took)}."

    case {listed_ok, for(%{"key" => key, "object" => object} <- blobs, object in wrong, do: key)} do
      {true, []} ->
        %{destination: destination["name"], backup: name, objects: length(held), bytes: bytes, took: took}

      {listed_ok, keys} ->
        not_as_sent =
          if(listed_ok, do: "", else: " Its list of blobs isn't as it was sent.") <>
            if(keys == [],
              do: "",
              else: " Not as sent: #{Enum.join(Enum.take(keys, 10), ", ")}#{if length(keys) > 10, do: " and more"}."
            )

        # What isn't there as it was sent goes, and so does what's kept of it here, so the next backup sends it
        # again.
        for object <- wrong, do: Destination.remove(opened, "objects/" <> object)

        transaction(call, fn db ->
          for object <- wrong,
              do: query(db, "delete from objects where destination = ? and object = ?", [destination["id"], object])

          query(db, "insert or replace into problems (destination, at, kind, text) values (?, ?, 'check', ?)", [
            destination["id"],
            began,
            String.trim(not_as_sent) <> " The next backup sends it again."
          ])
        end)

        raise message <> not_as_sent
    end
  end

  # An object as it's read back: its name, its size, and its digest, or `nil` where it isn't there.
  defp read_back(opened, object) do
    case Destination.get(opened, "objects/" <> object) do
      nil -> {object, 0, nil}
      data -> {object, byte_size(data), digest(data)}
    end
  end

  @doc """
  Brings back what backup `backup` at `destination` holds under `blobs` that's gone from the bucket. Changes nothing
  that's there.
  """
  def restore(call, destination, backup, blobs) do
    began = System.os_time(:millisecond)
    opened = Destination.open(call, destination)

    name =
      backup ||
        List.last(Destination.list(opened, "backups/")) ||
        raise("#{destination["name"]} has no backup yet.")

    name = String.replace_prefix(name, "backups/", "")
    listed = Destination.get(opened, "backups/" <> name) || raise "#{destination["name"]} has no backup #{name}."

    {gone, there} =
      JSON.decode!(listed)["blobs"]
      |> Enum.filter(&String.starts_with?(&1["key"], blobs))
      |> Enum.split_with(&(Bucket.size(&1["key"]) == :absent))

    for blob <- gone do
      data = Destination.get(opened, "objects/" <> blob["object"]) || raise "#{blob["key"]} is missing from the backup."
      if digest(data) != blob["sha256"], do: raise("#{blob["key"]} isn't what was backed up: the backup was changed.")
      Bucket.write(blob["key"], data)
    end

    %{
      backup: name,
      restored: Enum.map(gone, & &1["key"]),
      bytes: Enum.sum_by(gone, & &1["size"]),
      there: length(there),
      took: System.os_time(:millisecond) - began
    }
  end

  # The blobs a backup holds, as they're kept here.
  defp listed(compressed), do: JSON.decode!(:zlib.uncompress(compressed))["blobs"]

  defp digest(data), do: Base.encode16(:crypto.hash(:sha256, data), case: :lower)

  @doc false
  def size(bytes) when bytes < 1_000, do: "#{bytes} bytes"
  def size(bytes) when bytes < 1_000_000, do: "#{Float.round(bytes / 1.0e3, 1)} KB"
  def size(bytes) when bytes < 1_000_000_000, do: "#{Float.round(bytes / 1.0e6, 1)} MB"
  def size(bytes), do: "#{Float.round(bytes / 1.0e9, 2)} GB"

  defp seconds(ms) when ms < 1000, do: "#{ms} ms"
  defp seconds(ms), do: "#{Float.round(ms / 1000, 1)} s"
end
