defmodule Telemetry.Alerts do
  @moduledoc """
  What needs a person, kept as alerts that are read in the Telemetry app and by `alerts`, never sent to anyone: whoever
  wants them elsewhere has a copy of everything kept forwarded (`Telemetry.Forward`). As SRE alerts are, each is a
  symptom someone would notice, not a cause: a problem is one alert however often it recurs.

  | Kind | Fires when | Clears when |
  | --- | --- | --- |
  | `server` | One of the network's servers, as the relay says (`active`): its machine's `one serve run` said its server didn't start or stopped without being asked, and it hasn't started since, for 5 min, unless the relay says it's ready; or it said its server ended unasked 3 times in 10 min (`one.server.crash_loop`) | It says it started, or the relay says it's ready, or a server on the same machine started after it failed (its machine's server has a new key), and it hasn't looped for 10 min |
  | `update` | The network's update (`core/edge/updater/updater.ts`) is held: a server went back, or didn't finish in time | The update is under way again, or done |
  | `error` | A kind of error logged that wasn't in the last 14 days: its first line, numbers and ids aside. A fault, a crash or a client's exception, not a tool refusing (a `RuntimeError`) | It isn't logged for 10 min, or it stops being new an hour after it was first |
  | `failing` | Half or more of an app's tool calls failed, of 10 or more in 10 min | 10 min with fewer |
  | `unanswered` | An app didn't answer, in 2 or more of the last 5 minutes: a hook it didn't answer in time (`one.hook.timeouts`), or its database's server kept a call waiting without running it (`one.database.lease.wait`, `not_answered`) | 5 min without |
  | `deploy` | An app's deploy took over its 500 ms budget to show in a window, from when it began (`one.client.deploy.window`), in 2 or more of the last 10 minutes: a server release building every app again is one | 10 min without |
  | `open` | An app's window took over its 300 ms budget, from being asked to open to its page's first frame, the last time it opened on a device (`one.client.native.open`, which a client says of each open, with each step's time) | It opens within its budget there, or hasn't opened there for a day |
  | `backup` | A destination's last backup is older than its schedule and a day, or its last backup failed, as Backups last said (`one.backup`, which it says each hour and as each backup ends) | Backups says it's backed up, or that the destination is gone |

  Each alert is a row of Telemetry's database, one per kind and key (the server, the build, the error's kind, the
  app), moved only by `evaluate/1`, the clock's job every minute: it's `firing` while its symptom is seen, and
  `resolved` once the symptom has been gone for its kind's while. The same kind and key firing again after it's
  resolved is a new alert.

  What it reads is each batch kept since it last looked that it hasn't read, by its name (`Telemetry.Store.since/3`),
  so one kept late is read all the same, and it keeps of that only what its rules need: each server's last start and
  failure, each kind of error and when it was first and last seen, and per app per minute its calls, its failures and
  how often it didn't answer, and what Backups last said of each destination. Its first look reads the last 14 days for the kinds of error there were, and has no alert for
  any of them. Whether a server is one of the network's and ready, and whether the update is held, it asks of the relay
  as it looks (`OneCore.Servers.presence/1`, `OneCore.Servers.network/0`): where the relay doesn't answer, what it
  would have said of stays as it was.
  """
  alias Telemetry.{Read, Store}
  alias OneCore.Call

  # How far back it looks for batches it hasn't read, in milliseconds: a minute's batches are put together in their
  # hour's blob 10 minutes after it ends (`Store.prune/1`), and the rules look back no further.
  @gap 5 * 60_000
  @minute 60_000
  @day 86_400_000
  # How long an error kind isn't logged before its alert resolves: its recovery window, as Grafana's and Datadog's.
  @recovery 10 * @minute

  @schema [
    "create table looked (id integer primary key check (id = 1), at integer not null)",
    "create table batches (name text primary key, at integer not null) without rowid",
    """
    create table servers (
      server text primary key, name text, failing integer, failed integer, started integer, looping integer,
      why text
    )
    """,
    "create table seen (kind text primary key, place text, message text, first integer, last integer, new integer)",
    """
    create table minutes (
      app text not null, minute integer not null, calls integer not null default 0, failed integer not null default 0,
      unanswered integer not null default 0, primary key (app, minute)
    ) without rowid
    """,
    """
    create table alerts (
      id integer primary key, kind text not null, key text not null, state text not null, title text not null,
      text text, began integer not null, last integer not null, ended integer
    )
    """,
    # Kept in its place, since statements already run are counted by position: a kind's mute, dropped below.
    "create table muted (kind text primary key) without rowid",
    # Where a copy of what's kept is sent (`Telemetry.Forward`).
    "create table forward (id integer primary key check (id = 1), endpoint text not null, who text not null, item text not null, headers text not null)",
    # Alerts that were told, and a kind's mute, from when it notified people.
    "update alerts set state = 'firing' where state = 'notified'",
    "update alerts set state = 'resolved' where state = 'resolving'",
    "drop table muted",
    # Each app's slowest deploy to show in a window, per minute.
    "create table deploys (app text not null, minute integer not null, slowest integer not null, primary key (app, minute)) without rowid",
    # What Backups last said of each destination (`one.backup`).
    "create table backups (destination text primary key, at integer not null, last integer not null, every integer not null, outcome text not null, problem text)",
    # Each app's window's last open on each device, and its steps (`one.client.native.open`).
    "create table opens (device text not null, view text not null, name text, at integer not null, took integer not null, steps text, primary key (device, view)) without rowid"
  ]

  @kinds %{
    "server" => "A server isn't running for 5 minutes, or keeps restarting",
    "update" => "The network's update is held",
    "error" => "A kind of error not seen in the last 14 days",
    "failing" => "Half or more of an app's tool calls fail, of 10 or more in 10 minutes",
    "unanswered" => "An app doesn't answer for more than a minute",
    "deploy" => "An app's deploys take over 500 ms to show in its window, in 2 or more of the last 10 minutes",
    "backup" => "A backup isn't made within its schedule and a day, or the last one failed",
    "open" => "An app's window took over 300 ms to show its page the last time it opened on a device"
  }

  # How long a deploy has to show in a window, from when it began, in milliseconds.
  @deploy_budget 500
  # How long an app's window has to show its page, from being asked to open, in milliseconds: a cached app's should
  # feel instant.
  @open_budget 300

  # How long each kind's alert stays after its symptom was last seen before it's resolved, in milliseconds.
  @cool %{"server" => 0, "update" => 0, "error" => 0, "failing" => 10 * 60_000, "unanswered" => 5 * 60_000, "deploy" => 10 * 60_000, "backup" => 0, "open" => 0}

  @doc "Each kind of alert, with what fires it."
  def kinds, do: @kinds

  @doc """
  The alerts not resolved, and those resolved in the last `hours`, newest first, each kind of alert, and where a copy
  is sent (`Telemetry.Forward`).
  """
  def list(call, hours) do
    since = System.os_time(:millisecond) - hours * 3_600_000

    transaction(call, fn db ->
      alerts =
        query(
          db,
          """
          select id, kind, key, state, title, text, began, last, ended from alerts
          where state = 'firing' or ended > ? order by began desc
          """,
          [since]
        )

      kinds = for {kind, about} <- Enum.sort(@kinds), do: %{kind: kind, about: about}
      forwarding = for %{"endpoint" => endpoint} <- query(db, "select endpoint from forward", []), do: endpoint
      %{alerts: alerts, kinds: kinds, forwarding: List.first(forwarding)}
    end)
  end

  @doc """
  Reads what was recorded since it last looked, moves each alert on (the moduledoc's state machine) and sends a copy of what it
  read where its owner said (`Telemetry.Forward`).
  """
  def evaluate(call) do
    now = System.os_time(:millisecond)

    # Its first transaction, which marks the job as run: whether it has looked before, the batches it has read, and
    # the servers that may be down, whose presence is asked of the relay here rather than while the database is
    # held.
    {looked, read, failing} =
      transaction(call, fn db ->
        sql = "select server from servers where failing is not null or looping is not null"

        {query(db, "select at from looked", []) != [],
         MapSet.new(for %{"name" => name} <- query(db, "select name from batches", []), do: name),
         for(%{"server" => server} <- query(db, sql, []), do: server)}
      end)

    asked = %{presence: Map.new(failing, &{&1, presence(&1)}), update: update(), names: Map.new(Call.apps(call), &{&1.name, &1.title})}

    {names, exports} = if looked, do: Store.since(call, (now - @gap) * 1_000_000, read), else: {[], []}
    batches = if looked, do: Enum.map(exports, &elem(&1, 1)), else: Store.read(call, (now - Store.days() * 86_400_000) * 1_000_000)

    signals = signals(batches)

    fresh =
      transaction(call, fn db ->
        # Another evaluation read them already, as one that ran long overlapping the next does.
        if looked == (query(db, "select at from looked", []) != []) and not read?(db, names) do
          keep(db, signals, not looked, now)
          for name <- names, do: query(db, "insert into batches (name, at) values (?, ?)", [name, now])
          query(db, "delete from batches where at < ?", [now - 2 * @gap])
          # Said once: a minute that read nothing writes nothing, so it adds nothing to the database's log.
          if not looked, do: query(db, "insert into looked (id, at) values (1, ?)", [now])
          step(db, now, conditions(db, now, asked), unknown(asked))
          names
        else
          []
        end
      end)

    # What's newly kept is rolled up, for looks over hours to read, and sent where its owner said, if they did.
    Store.roll_up(call.own, for({name, _batch} = export <- exports, name in fresh, do: export))
    Telemetry.Forward.send(call, fresh)
  end

  defp read?(_db, []), do: false

  defp read?(db, names),
    do: query(db, "select 1 from batches where name in (select value from json_each(?))", [JSON.encode!(names)]) != []

  # Whether a server is one of the network's and ready, as the relay says, or `:unknown` where it can't be asked.
  defp presence(server) do
    OneCore.Servers.presence(server) || %{}
  rescue
    # The edge didn't answer: what's known of the server stays as it was.
    OneCore.Edge.Unreachable -> :unknown
  end

  # The network's update, as its updater says, or `:unknown` where it can't be asked.
  defp update do
    case OneCore.Servers.network() do
      {:ok, update} -> update || %{}
      {:error, _why} -> :unknown
    end
  end

  # The alerts whose symptom couldn't be asked of the relay this time, which stay as they are, by kind and key.
  defp unknown(asked) do
    fn
      {"server", server} -> asked.presence[server] == :unknown
      {"update", _build} -> asked.update == :unknown
      _ -> false
    end
  end

  ## Signals

  # What the rules need of what was recorded, as one list of tuples, from the batches' spans, logs and metrics.
  defp signals(batches) do
    Enum.flat_map(batches, fn batch ->
      Enum.flat_map(batch["spans"], &span/1) ++
        Enum.flat_map(batch["logs"], &log/1) ++ Enum.flat_map(batch["metrics"], &metric/1)
    end)
  end

  defp span(%{"name" => "one.server.start", "attributes" => %{"one.server" => server} = a} = span) do
    case span["status"] do
      %{"code" => "error", "message" => why} -> [{:server, server, a["host.name"], ms(span["end"]), {:failed, why}}]
      _ -> [{:server, server, a["host.name"], ms(span["end"]), :started}]
    end
  end

  # A client's window that opened (`one.steps`, as it said each step), not one that didn't (`error`).
  defp span(%{"name" => "one.client.native.open", "attributes" => %{"one.view" => view} = a} = span) do
    case span["status"] do
      %{"code" => "error"} -> []
      _ -> [{:open, a["one.device"] || span["server"] || "", view, Read.from(span), ms(span["end"]), ms(span["end"]) - ms(span["start"]), a["one.steps"]}]
    end
  end

  defp span(_span), do: []

  defp log(%{"eventName" => "one.server.stop", "severity" => "ERROR", "attributes" => %{"one.server" => server} = a} = log),
    do: [{:server, server, a["host.name"], ms(log["time"]), {:failed, log["body"]}}]

  defp log(%{"eventName" => "one.server.crash_loop", "attributes" => %{"one.server" => server} = a} = log),
    do: [{:server, server, a["host.name"], ms(log["time"]), {:looping, log["body"]}}]

  defp log(%{"eventName" => "one.backup", "attributes" => %{"one.destination" => destination} = a} = log),
    do: [{:backup, destination, ms(log["time"]), a["one.backup.last"] || 0, a["one.backup.every"] || 0, a["one.outcome"], a["one.problem"]}]

  # What a server's own program says of it is its alert's, not a new kind of error.
  defp log(%{"eventName" => "one.server." <> _}), do: []

  # A tool refuses by raising its message, a `RuntimeError` (`OneCore.App`), which is the caller's to read, not a
  # fault: anything else raised is.
  defp log(%{"body" => "** (RuntimeError) " <> _}), do: []

  defp log(%{"severity" => "ERROR", "body" => body} = log) when is_binary(body),
    do: [{:error, ms(log["time"]), kind(body), place(log), first_line(body)}]

  defp log(_log), do: []

  defp metric(%{"name" => "one.tool.duration", "attributes" => a} = point) do
    failed = if a["one.outcome"] == "error", do: point["count"], else: 0
    [{:calls, a["one.app"], minute(point["time"]), point["count"], failed}]
  end

  defp metric(%{"name" => "one.hook.timeouts", "attributes" => %{"one.app" => app}} = point),
    do: [{:unanswered, app, minute(point["time"]), point["count"]}]

  defp metric(%{"name" => "one.database.lease.wait", "attributes" => %{"one.app" => app, "one.outcome" => "not_answered"}} = point),
    do: [{:unanswered, app, minute(point["time"]), point["count"]}]

  defp metric(%{"name" => "one.client.deploy.window", "attributes" => %{"one.app" => app}} = point),
    do: [{:deploy, app, minute(point["time"]), round(point["max"] || point["sum"] || 0)}]

  defp metric(_point), do: []

  # Where an error was, as a person reads it: what logged it, if it said, on the server or client it was on.
  defp place(log) do
    on = Read.from(log) || log["server"]
    if log["eventName"], do: "#{log["eventName"]}, on #{on}", else: "On #{on}"
  end

  # A kind of error: its first line, with what changes from one to the next, numbers and ids, taken out.
  defp kind(message) do
    message
    |> first_line()
    |> String.replace(~r/[0-9a-f]{8,}/i, "…")
    |> String.replace(~r/\d+/, "#")
    |> String.slice(0, 300)
  end

  # Keeps what the rules need of `signals`. Its first look keeps only the kinds of error there were, as seen, and
  # none is new.
  defp keep(db, signals, true, now) do
    signals = for {:error, _at, _kind, _place, _message} = error <- signals, do: error
    keep(db, signals, false, now)
    query(db, "update seen set new = 0", [])
  end

  defp keep(db, signals, false, now) do
    servers = Enum.sort_by(for({:server, _, _, _, _} = said <- signals, do: said), &elem(&1, 3))

    for {:server, server, name, at, said} <- servers do
      row =
        case query(db, "select * from servers where server = ?", [server]) do
          [row] -> row
          [] -> %{"failing" => nil, "failed" => nil, "started" => nil, "looping" => nil, "why" => nil}
        end

      row =
        case said do
          :started -> %{row | "started" => max(row["started"] || 0, at), "failing" => if(row["failing"] && row["failing"] < at, do: nil, else: row["failing"])}
          {:failed, why} -> %{row | "failing" => row["failing"] || at, "failed" => at, "why" => why}
          {:looping, why} -> %{row | "looping" => at, "why" => why}
        end

      query(
        db,
        "insert or replace into servers (server, name, failing, failed, started, looping, why) values (?, ?, ?, ?, ?, ?, ?)",
        [server, name, row["failing"], row["failed"], row["started"], row["looping"], row["why"]]
      )
    end

    for {:error, at, kind, place, message} <- signals do
      query(
        db,
        """
        insert into seen (kind, place, message, first, last, new) values (?, ?, ?, ?, ?, ?)
        on conflict (kind) do update set last = max(last, excluded.last)
        """,
        [kind, place, message, at, at, 1]
      )
    end

    for {:calls, app, minute, calls, failed} <- signals do
      query(
        db,
        """
        insert into minutes (app, minute, calls, failed) values (?, ?, ?, ?)
        on conflict (app, minute) do update set calls = calls + excluded.calls, failed = failed + excluded.failed
        """,
        [app, minute, calls, failed]
      )
    end

    for {:unanswered, app, minute, count} <- signals do
      query(
        db,
        """
        insert into minutes (app, minute, unanswered) values (?, ?, ?)
        on conflict (app, minute) do update set unanswered = unanswered + excluded.unanswered
        """,
        [app, minute, count]
      )
    end

    for {:deploy, app, minute, took} <- signals do
      query(
        db,
        """
        insert into deploys (app, minute, slowest) values (?, ?, ?)
        on conflict (app, minute) do update set slowest = max(slowest, excluded.slowest)
        """,
        [app, minute, took]
      )
    end

    for {:backup, destination, at, last, every, outcome, problem} <- Enum.sort_by(for({:backup, _, _, _, _, _, _} = said <- signals, do: said), &elem(&1, 2)) do
      if outcome == "removed",
        do: query(db, "delete from backups where destination = ? and at <= ?", [destination, at]),
        else:
          query(
            db,
            """
            insert into backups (destination, at, last, every, outcome, problem) values (?, ?, ?, ?, ?, ?)
            on conflict (destination) do update set at = excluded.at, last = excluded.last, every = excluded.every,
              outcome = excluded.outcome, problem = excluded.problem where excluded.at >= backups.at
            """,
            [destination, at, last, every, outcome, problem]
          )
    end

    for {:open, device, view, from, at, took, steps} <- signals do
      query(
        db,
        """
        insert into opens (device, view, name, at, took, steps) values (?, ?, ?, ?, ?, ?)
        on conflict (device, view) do update set name = excluded.name, at = excluded.at, took = excluded.took,
          steps = excluded.steps where excluded.at >= opens.at
        """,
        [device, view, from, at, took, steps]
      )
    end

    # What no rule looks at any more.
    query(db, "delete from minutes where minute < ?", [now - 15 * @minute])
    query(db, "delete from deploys where minute < ?", [now - 15 * @minute])
    query(db, "delete from opens where at < ?", [now - @day])
    query(db, "delete from seen where last < ?", [now - Store.days() * 86_400_000])
    query(db, "delete from servers where max(coalesce(failed, 0), coalesce(started, 0), coalesce(looping, 0)) < ?", [now - Store.days() * 86_400_000])
    query(db, "delete from alerts where ended < ?", [now - 7 * 86_400_000])
  end

  ## Rules

  # Each alert whose symptom is there now, by its kind and key: what it's called.
  defp conditions(db, now, asked) do
    called = fn "one" -> "The network's own tools"; app -> Map.get(asked.names, app, app) end

    rows = query(db, "select * from servers", [])
    # Each machine's latest start, by whichever key its server had: a machine's server given a new key leaves its
    # old one down for good, which a start under the new one answers.
    started = rows |> Enum.group_by(& &1["name"]) |> Map.new(fn {name, rows} -> {name, rows |> Enum.map(&(&1["started"] || 0)) |> Enum.max()} end)

    servers =
      rows
      |> Enum.reject(&(&1["name"] && &1["failing"] && started[&1["name"]] > &1["failing"]))
      |> Enum.flat_map(fn row ->
        case server(row, row["name"] || "A server", now, asked.presence[row["server"]]) do
          nil -> []
          condition -> [{{"server", row["server"]}, condition}]
        end
      end)
      |> Map.new()

    update =
      case asked.update do
        %{"network_update_status" => "held", "build" => build, "held" => held} ->
          version = String.split(build || "", "+") |> hd()
          why = Enum.join(Enum.reject([held["name"] || held["server"], held["why"]], &is_nil/1), ": ")
          %{{"update", build || ""} => %{title: "The update to #{version} is held", text: why}}

        _ ->
          %{}
      end

    errors =
      for row <- query(db, "select * from seen where new = 1 and first > ? and last > ?", [now - 60 * @minute, now - @recovery]), into: %{} do
        {{"error", row["kind"]}, %{title: "New error: #{String.slice(row["message"], 0, 100)}", text: row["place"]}}
      end

    failing =
      for row <-
            query(
              db,
              """
              select app, sum(calls) as calls, sum(failed) as failed from minutes where minute > ?
              group by app having sum(calls) >= 10 and sum(failed) * 2 >= sum(calls)
              """,
              [now - 10 * @minute]
            ),
          into: %{} do
        app = called.(row["app"])

        {{"failing", row["app"]},
         %{
           title: "#{app} is failing",
           text: "#{row["failed"]} of its #{row["calls"]} calls failed in the last 10 minutes."
         }}
      end

    unanswered =
      for row <-
            query(
              db,
              "select app, count(*) as minutes from minutes where unanswered > 0 and minute > ? group by app having count(*) >= 2",
              [now - 5 * @minute]
            ),
          into: %{} do
        app = called.(row["app"])

        {{"unanswered", row["app"]},
         %{
           title: "#{app} isn't answering",
           text: "It didn't answer in #{row["minutes"]} of the last 5 minutes, so what needed it failed."
         }}
      end

    deploys =
      for row <-
            query(
              db,
              """
              select app, count(*) as minutes, max(slowest) as slowest from deploys where slowest > ? and minute > ?
              group by app having count(*) >= 2
              """,
              [@deploy_budget, now - 10 * @minute]
            ),
          into: %{} do
        {{"deploy", row["app"]},
         %{
           title: "#{called.(row["app"])} is slow to show its deploys",
           text:
             "In #{row["minutes"]} of the last 10 minutes a deploy took up to #{row["slowest"]} ms to show in its " <>
               "window, over its #{@deploy_budget} ms budget."
         }}
      end

    opens =
      for row <- query(db, "select * from opens where took > ?", [@open_budget]), into: %{} do
        {{"open", "#{row["device"]}|#{row["view"]}"},
         %{
           title: "#{called.(row["view"])} is slow to open#{String.replace_prefix(row["name"] || "", "App", "")}",
           text: "It took #{row["took"]} ms to show its page, over its #{@open_budget} ms budget: #{row["steps"]}."
         }}
      end

    # A destination's last backup older than its schedule and a day, or one that failed: as Time Machine says it
    # couldn't complete the backup after a missed day.
    backups =
      for row <- query(db, "select * from backups", []),
          row["outcome"] == "failed" or (row["every"] > 0 and now - row["last"] > row["every"] * 1000 + @day),
          into: %{} do
        {{"backup", row["destination"]},
         %{
           title: "Backups couldn't complete the backup to #{row["destination"]}",
           text: Enum.join(Enum.reject([last_backup(row["last"], now), row["problem"]], &is_nil/1), " ")
         }}
      end

    Enum.reduce([servers, update, errors, failing, unanswered, deploys, opens, backups], &Map.merge/2)
  end

  # When the last backup was, as a person says it.
  defp last_backup(0, _now), do: "It hasn't been backed up yet."
  defp last_backup(at, now) when now - at < 2 * 3_600_000, do: "The last backup was #{div(now - at, @minute)} minutes ago."
  defp last_backup(at, now) when now - at < 2 * @day, do: "The last backup was #{div(now - at, 3_600_000)} hours ago."
  defp last_backup(at, now), do: "The last backup was #{div(now - at, @day)} days ago."

  # A server's symptom, if it has one, while the relay says it's one of the network's (`active`), not one joining or
  # one that left: it keeps restarting, or it's been down 5 minutes and the relay doesn't say it's ready.
  defp server(_row, _name, _now, nil), do: nil
  defp server(_row, _name, _now, :unknown), do: nil

  defp server(row, name, now, presence) do
    cond do
      presence["server_membership"] != "active" ->
        nil

      row["looping"] && now - row["looping"] < 10 * @minute ->
        %{title: "#{name}'s server keeps restarting", text: row["why"]}

      row["failing"] && now - row["failing"] >= 5 * @minute && presence["server_ready"] != true ->
        %{title: "#{name}'s server isn't running", text: row["why"]}

      true ->
        nil
    end
  end

  ## The state machine

  # Moves every alert on by what fires now, `conditions`: a new one fires, one still firing is kept up to date,
  # and one whose symptom has been gone for its kind's while clears, unless it couldn't be asked this time (`unknown`).
  defp step(db, now, conditions, unknown) do
    firing = query(db, "select * from alerts where state = 'firing' order by id", [])
    # One alert per kind and key, the newest: any other left firing beside it, as one from before a server's
    # restart, is resolved with the rest below.
    open = Map.new(firing, &{{&1["kind"], &1["key"]}, &1})

    for {{kind, key} = id, condition} <- conditions do
      case open[id] do
        nil ->
          query(
            db,
            "insert into alerts (kind, key, state, title, text, began, last) values (?, ?, 'firing', ?, ?, ?, ?)",
            [kind, key, condition.title, condition.text, now, now]
          )

        row ->
          query(db, "update alerts set title = ?, text = ?, last = ? where id = ?", [
            condition.title,
            condition.text,
            now,
            row["id"]
          ])
      end
    end

    for %{"kind" => kind, "key" => key} = row <- firing,
        id = {kind, key},
        open[id]["id"] != row["id"] or (not Map.has_key?(conditions, id) and not unknown.(id) and now - row["last"] >= @cool[kind]) do
      query(db, "update alerts set state = 'resolved', ended = ? where id = ?", [now, row["id"]])
    end
  end

  ## The database

  @doc "Runs `fun` in a transaction on Telemetry's database, whose tables are this module's `@schema`."
  def transaction(call, fun) do
    database = Call.app(call, "database")
    database.transaction(call, @schema, &fun.({database, &1}))
  end

  @doc "Runs one statement in a `transaction/2`."
  def query({database, db}, sql, values), do: database.query(db, sql, values)

  defp ms(ns), do: div(ns, 1_000_000)
  defp minute(ns), do: div(ms(ns), @minute) * @minute
  defp first_line(text), do: text |> String.split("\n", parts: 2) |> hd()
end
