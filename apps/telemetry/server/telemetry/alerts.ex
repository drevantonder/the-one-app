defmodule Telemetry.Alerts do
  @moduledoc """
  What needs a person, told to the network's first owner through Notifications, once a problem, and again once it
  clears. As SRE alerts are, each is a symptom someone would notice, not a cause, and it's quiet by default: a
  problem is one notification however often it recurs, and only what lasts or is new is told.

  | Kind | Fires when | Clears when |
  | --- | --- | --- |
  | `server` | One of the network's servers, as the relay says (`active`): its machine's `one serve run` said its server didn't start or stopped without being asked, and it hasn't started since, for 5 min, unless the relay says it's ready; or it said its server ended unasked 3 times in 10 min (`one.server.crash_loop`) | It says it started, or the relay says it's ready, and it hasn't looped for 10 min |
  | `update` | The network's update (`core/edge/updater/updater.ts`) is held: a server went back, or didn't finish in time | The update is under way again, or done |
  | `error` | A kind of error logged that wasn't in the last 14 days: its first line, numbers and ids aside. A fault, a crash or a client's exception, not a tool refusing (a `RuntimeError`) | Told once, and not told resolved: it stops being new an hour later |
  | `failing` | Half or more of an app's tool calls failed, of 10 or more in 10 min | 10 min with fewer |
  | `unanswered` | An app didn't answer, in 2 or more of the last 5 minutes: a hook it didn't answer in time (`one.hook.timeouts`), or its database's server kept a call waiting without running it (`one.database.lease.wait`, `not_answered`) | 5 min without |

  Each alert is a row of Telemetry's database, one per kind and key (the server, the build, the error's kind, the
  app), in this state machine, moved only by `evaluate/1`, the clock's job every minute:

  ```text
              fires                  told                        clears for its kind's while
    (none) ─────────► firing ──────────────► notified ─────────────────────────────────────► resolving
                        │                                                                      │ told
                        │ clears before it's told: nobody's told of it                        ▼
                        └─────────────────────────────────────────────────────────────────► resolved
    an `error`, which isn't told resolved: notified ─clears─► resolved
    the same kind and key firing again after it's resolved is a new alert, told again
  ```

  A kind its owner muted (`mute/3`) moves the same way, and nothing of it is told: as it's told, it's marked told.
  Telling is after the transaction that moved it, and marking told after the telling, so an alert is told at least
  once even where a server stops between them, as a job's call promises (`database`'s skill).

  What it reads is each batch kept since it last looked that it hasn't read, by its name (`Telemetry.Store.since/3`),
  so one kept late is read all the same, and it keeps of that only what its rules need: each server's last start and
  failure, each kind of error and when it was first and last seen, and per app per minute its calls, its failures and
  how often it didn't answer. Its first look reads the last 14 days for the kinds of error there were, and tells none
  of them. Whether a server is one of the network's and ready, and whether the update is held, it asks of the relay
  as it looks (`OneCore.Servers.presence/1`, `OneCore.Servers.network/0`): where the relay doesn't answer, what it
  would have said of stays as it was.
  """
  alias Telemetry.{Read, Store}
  alias OneCore.Call

  # How far back it looks for batches it hasn't read, in milliseconds: a minute's batches are put together in their
  # hour's blob 10 minutes after it ends (`Store.prune/1`), and the rules look back no further.
  @gap 5 * 60_000
  @minute 60_000

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
      text text, resolved text, began integer not null, last integer not null, ended integer
    )
    """,
    "create table muted (kind text primary key) without rowid",
    # Where a copy of what's kept is sent (`Telemetry.Forward`).
    "create table forward (id integer primary key check (id = 1), endpoint text not null, who text not null, item text not null, headers text not null)"
  ]

  @kinds %{
    "server" => "A server isn't running for 5 minutes, or keeps restarting",
    "update" => "The network's update is held",
    "error" => "A kind of error not seen in the last 14 days",
    "failing" => "Half or more of an app's tool calls fail, of 10 or more in 10 minutes",
    "unanswered" => "An app doesn't answer for more than a minute"
  }

  # How long each kind's alert stays after its symptom was last seen before it's resolved, in milliseconds.
  @cool %{"server" => 0, "update" => 0, "error" => 0, "failing" => 10 * 60_000, "unanswered" => 5 * 60_000}

  @doc "Each kind of alert, with what fires it."
  def kinds, do: @kinds

  @doc """
  The alerts not resolved, and those resolved in the last day, newest first, and each kind with whether it's
  muted.
  """
  def list(call) do
    day = System.os_time(:millisecond) - 86_400_000

    transaction(call, fn db ->
      alerts =
        query(
          db,
          """
          select id, kind, key, state, title, text, began, last, ended from alerts
          where state in ('firing', 'notified', 'resolving') or ended > ? order by began desc
          """,
          [day]
        )

      muted = for %{"kind" => kind} <- query(db, "select kind from muted", []), do: kind
      kinds = for {kind, about} <- Enum.sort(@kinds), do: %{kind: kind, about: about, muted: kind in muted}
      %{alerts: alerts, kinds: kinds}
    end)
  end

  @doc "Mutes a kind of alert, or has it told again: only the network's owners may."
  def mute(call, kind, muted) do
    if not Map.has_key?(@kinds, kind), do: raise("There's no kind of alert called #{kind}.\n\n`alerts` lists them.")
    if not Call.owns?(%{call | subject: "network"}), do: raise("Only the network's owners choose which alerts they're told.")

    transaction(call, fn db ->
      if muted,
        do: query(db, "insert or ignore into muted (kind) values (?)", [kind]),
        else: query(db, "delete from muted where kind = ?", [kind])
    end)

    Call.changed(call)
    %{kind: kind, muted: muted}
  end

  @doc """
  Reads what was recorded since it last looked, moves each alert on (the moduledoc's state machine), and tells the
  network's first owner what fired and what cleared. Sends a copy of what it read where its owner said
  (`Telemetry.Forward`). Gives how many it told.
  """
  def evaluate(call) do
    now = System.os_time(:millisecond)

    # Its first transaction, which marks the job as run: whether it has looked before, the batches it has read, and
    # the servers it may tell are down, whose presence is asked of the relay here rather than while the database is
    # held.
    {looked, read, failing} =
      transaction(call, fn db ->
        sql = "select server from servers where failing is not null or looping is not null"

        {query(db, "select at from looked", []) != [],
         MapSet.new(for %{"name" => name} <- query(db, "select name from batches", []), do: name),
         for(%{"server" => server} <- query(db, sql, []), do: server)}
      end)

    asked = %{presence: Map.new(failing, &{&1, presence(&1)}), update: update(), names: Map.new(Call.apps(call), &{&1.name, &1.title})}

    {names, batches} =
      if looked,
        do: Store.since(call, (now - @gap) * 1_000_000, read),
        else: {[], Store.read(call, (now - Store.days() * 86_400_000) * 1_000_000)}

    signals = signals(batches)

    {outbox, fresh} =
      transaction(call, fn db ->
        # Another evaluation read them already, as one that ran long overlapping the next does.
        if looked == (query(db, "select at from looked", []) != []) and not read?(db, names) do
          keep(db, signals, not looked, now)
          for name <- names, do: query(db, "insert into batches (name, at) values (?, ?)", [name, now])
          query(db, "delete from batches where at < ?", [now - 2 * @gap])
          query(db, "insert or replace into looked (id, at) values (1, ?)", [now])
          step(db, now, conditions(db, now, asked), unknown(asked))
          {outbox(db), names}
        else
          {[], []}
        end
      end)

    # What's newly kept is sent where its owner said, if they did.
    Telemetry.Forward.send(call, fresh)
    tell(call, outbox)
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

  defp span(_span), do: []

  defp log(%{"eventName" => "one.server.stop", "severity" => "ERROR", "attributes" => %{"one.server" => server} = a} = log),
    do: [{:server, server, a["host.name"], ms(log["time"]), {:failed, log["body"]}}]

  defp log(%{"eventName" => "one.server.crash_loop", "attributes" => %{"one.server" => server} = a} = log),
    do: [{:server, server, a["host.name"], ms(log["time"]), {:looping, log["body"]}}]

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

    # What no rule looks at any more.
    query(db, "delete from minutes where minute < ?", [now - 15 * @minute])
    query(db, "delete from seen where last < ?", [now - Store.days() * 86_400_000])
    query(db, "delete from servers where max(coalesce(failed, 0), coalesce(started, 0), coalesce(looping, 0)) < ?", [now - Store.days() * 86_400_000])
    query(db, "delete from alerts where ended < ?", [now - 7 * 86_400_000])
  end

  ## Rules

  # Each alert whose symptom is there now, by its kind and key: what it's told as, and as once it clears.
  defp conditions(db, now, asked) do
    called = fn "one" -> "The network's own tools"; app -> Map.get(asked.names, app, app) end

    servers =
      query(db, "select * from servers", [])
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
          %{{"update", build || ""} => %{title: "The update to #{version} is held", text: why, resolved: "The update to #{version} is going again"}}

        _ ->
          %{}
      end

    errors =
      for row <- query(db, "select * from seen where new = 1 and first > ?", [now - 60 * @minute]), into: %{} do
        {{"error", row["kind"]}, %{title: "New error: #{String.slice(row["message"], 0, 100)}", text: row["place"], resolved: nil}}
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
           text: "#{row["failed"]} of its #{row["calls"]} calls failed in the last 10 minutes.",
           resolved: "#{app} is working again"
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
           text: "It didn't answer in #{row["minutes"]} of the last 5 minutes, so what needed it failed.",
           resolved: "#{app} is answering again"
         }}
      end

    Enum.reduce([servers, update, errors, failing, unanswered], &Map.merge/2)
  end

  # A server's symptom, if it has one, while the relay says it's one of the network's (`active`), not one joining or
  # one that left: it keeps restarting, or it's been down 5 minutes and the relay doesn't say it's ready.
  defp server(_row, _name, _now, nil), do: nil
  defp server(_row, _name, _now, :unknown), do: nil

  defp server(row, name, now, presence) do
    cond do
      presence["server_membership"] != "active" ->
        nil

      row["looping"] && now - row["looping"] < 10 * @minute ->
        %{title: "#{name}'s server keeps restarting", text: row["why"], resolved: "#{name}'s server is running again"}

      row["failing"] && now - row["failing"] >= 5 * @minute && presence["server_ready"] != true ->
        %{title: "#{name}'s server isn't running", text: row["why"], resolved: "#{name}'s server is running again"}

      true ->
        nil
    end
  end

  ## The state machine

  # Moves every alert on by what fires now, `conditions`: a new one fires, one still firing is kept up to date,
  # and one whose symptom has been gone for its kind's while clears, unless it couldn't be asked this time (`unknown`).
  defp step(db, now, conditions, unknown) do
    open = Map.new(query(db, "select * from alerts where state in ('firing', 'notified')", []), &{{&1["kind"], &1["key"]}, &1})

    for {{kind, key} = id, condition} <- conditions do
      case open[id] do
        nil ->
          query(
            db,
            "insert into alerts (kind, key, state, title, text, resolved, began, last) values (?, ?, 'firing', ?, ?, ?, ?, ?)",
            [kind, key, condition.title, condition.text, condition.resolved, now, now]
          )

        row ->
          query(db, "update alerts set title = ?, text = ?, resolved = ?, last = ? where id = ?", [
            condition.title,
            condition.text,
            condition.resolved,
            now,
            row["id"]
          ])
      end
    end

    for {{kind, _key} = id, row} <- open, not Map.has_key?(conditions, id), not unknown.(id), now - row["last"] >= @cool[kind] do
      state = if row["state"] == "notified" and row["resolved"], do: "resolving", else: "resolved"
      query(db, "update alerts set state = ?, ended = ? where id = ?", [state, now, row["id"]])
    end
  end

  # What's to be told: each alert firing, and each resolving, with whether its kind is muted.
  defp outbox(db) do
    muted = for %{"kind" => kind} <- query(db, "select kind from muted", []), do: kind

    for row <- query(db, "select * from alerts where state in ('firing', 'resolving') order by began", []) do
      title = if row["state"] == "firing", do: row["title"], else: row["resolved"]
      text = if row["state"] == "firing", do: row["text"]
      %{id: row["id"], state: row["state"], title: title, text: text, muted: row["kind"] in muted}
    end
  end

  # Tells the network's first owner each one not muted, then marks each told: firing is notified, and resolving
  # resolved.
  defp tell(_call, []), do: %{told: 0}

  defp tell(call, outbox) do
    owner = OneCore.User.first_owner()
    notifications = Call.app(call, "notifications")

    told =
      for alert <- outbox, not alert.muted, owner do
        notification = if alert.text, do: %{"text" => String.slice(alert.text, 0, 1000)}, else: %{}
        notifications.send(call, Map.merge(notification, %{"title" => String.slice(alert.title, 0, 120), "to" => owner}))
      end

    transaction(call, fn db ->
      for alert <- outbox do
        query(db, "update alerts set state = ? where id = ? and state = ?", [
          if(alert.state == "firing", do: "notified", else: "resolved"),
          alert.id,
          alert.state
        ])
      end
    end)

    Call.changed(call)
    %{told: length(told)}
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
