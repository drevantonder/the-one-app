defmodule Backups do
  @moduledoc """
  Backups: copies of the whole bucket, every user's and group's blobs as they are, to a folder on one of the network's
  machines, as on a disk plugged into it, through the service `Backups.Disk`, or to S3-compatible storage, whose keys
  are an item in the network's Vault. A blob that's encrypted in the bucket is encrypted in a backup too. Whoever can
  reach the destination restores, with `one restore` and no network running (`apps/backups/client/native/src/lib.rs`,
  which says what a destination holds).

  What's kept here, in the app's database for the network, is which blobs each backup holds, and which of the
  destination's objects holds what, so a blob that's there already isn't sent again (`Backups.Run`).
  """
  @behaviour OneCore.App
  alias Backups.{Disk, Run}
  alias OneCore.{Call, Service}

  @schema [
    """
    create table destinations (id text primary key, name text not null, kind text not null, machine text, folder text,
      endpoint text, bucket text, region text, item text, recipient text not null, whose text not null,
      added integer not null)
    """,
    """
    create table backups (destination text not null, name text not null, at integer not null, took integer not null,
      blobs integer not null, bytes integer not null, sent integer not null, sent_bytes integer not null,
      stored integer not null, digest text not null, listed blob not null, primary key (destination, name))
    """,
    """
    create table objects (destination text not null, object text not null, sha256 text not null, stored integer not null,
      digest text not null, primary key (destination, object))
    """,
    "create index objects_by_content on objects (destination, sha256)",
    "create table problems (destination text primary key, at integer not null, kind text not null, text text not null)",
    "create table settings (name text primary key, value text not null)",
    "alter table destinations drop column whose",
    "alter table destinations drop column recipient",
    "create table runs (destination text primary key, server text not null, at integer not null, beat integer not null)",
    # The blobs read for the backup under way, kept as they're sent, so one cut short reads none of them again.
    """
    create table listed (destination text not null, key text not null, tag text not null, object text not null,
      size integer not null, sha256 text not null, primary key (destination, key))
    """,
    # When the network's owners were told a destination couldn't be backed up, until it is again.
    "create table told (destination text primary key, at integer not null)"
  ]

  # How often each schedule backs up, in seconds.
  @schedules %{"hour" => 3600, "day" => 86_400, "week" => 7 * 86_400, "never" => nil}

  # How long past its schedule a destination's last backup may be before it needs someone, in milliseconds: as Time
  # Machine says it couldn't complete the backup after a missed day.
  @overdue 86_400_000

  @retention "Every backup is kept for a day, then one a day for a week, one a week for a month and one a month for a year."

  @impl true
  def icon, do: "client/icon.svg"

  # Its data is the network's backups: one schedule and one set of places for everyone.
  @impl true
  def own, do: :installed

  @impl true
  def requires do
    %{
      "core" => ">= 0.32.107",
      "database" => "to keep where and when you back up",
      "vault" => "to keep your storage's keys",
      "notifications" => "to tell you when a backup couldn't be made",
      "telemetry" => "to raise an alert when a backup couldn't be made"
    }
  end

  @impl true
  def hooks, do: ["settings.sections"]

  # Backups is a section of Settings, for the network's owners, who alone see and change them.
  @impl true
  def hook("settings.sections", _about, call) do
    if Call.owns?(%{call | subject: "network"}),
      do: [%{in: "settings", name: "Backups", component: "backups", icon: "client/icon.svg", group: "system", order: 1, badge: "badge"}],
      else: []
  end

  @impl true
  def jobs, do: [%{tool: "due", every: 3600}]

  @impl true
  def tools do
    destination = %{type: "string", description: "The destination's name, as `destinations` lists it"}

    backup = %{
      type: "object",
      properties: %{
        name: %{type: "string", description: "What restore's backup takes"},
        at: %{type: "integer", description: "When it began, in milliseconds since 1970"},
        took: %{type: "integer", description: "In milliseconds"},
        blobs: %{type: "integer"},
        bytes: %{type: "integer"},
        sent: %{type: "integer", description: "How many objects it sent, new"},
        sent_bytes: %{type: "integer"}
      },
      required: ~w(name at took blobs bytes sent sent_bytes)
    }

    # A backup just made, at a destination, and how many older ones went by the rule of what's kept.
    made = %{
      backup
      | properties: Map.merge(backup.properties, %{destination: destination, gone: %{type: "integer"}}),
        required: ["destination", "gone" | backup.required]
    }

    [
      %{
        name: "destinations",
        read_only: true,
        description:
          "Where backups go, each with when it last finished, how big it is, how long it took, its backups and what's wrong if anything is; how often they're made; which are kept; and the machines a folder can be on",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            every: %{type: "string", enum: ["hour", "day", "week", "never"], description: "How often backups are made"},
            retention: %{type: "string", description: "Which backups are kept"},
            machines: %{
              type: "array",
              description: "The machines a folder can be on",
              items: %{
                type: "object",
                properties: %{key: %{type: "string"}, name: %{type: ["string", "null"]}},
                required: ["key", "name"]
              }
            },
            destinations: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: %{type: "string"},
                  kind: %{type: "string", enum: ["folder", "s3"]},
                  where: %{type: "string", description: "Its folder, or its storage and bucket"},
                  machine: %{type: ["string", "null"], description: "The machine its folder is on"},
                  item: %{type: ["string", "null"], description: "The Vault item it's reached with"},
                  size: %{type: "integer", description: "How much it holds, in bytes"},
                  last: %{anyOf: [backup, %{type: "null"}], description: "The newest backup"},
                  backups: %{type: "array", items: backup, description: "Newest first"},
                  overdue: %{
                    type: "boolean",
                    description: "No backup has finished there within its schedule and a day"
                  },
                  state: %{
                    type: "string",
                    enum: ~w(never backing_up backed_up failed changed interrupted),
                    description:
                      "never backed up; backing up now; backed up; the last backup failed; what's there was changed since it was sent; or the last backup was cut short, as by its server restarting"
                  },
                  running: %{
                    type: ["object", "null"],
                    description: "The backup under way, or the one cut short: when it began, and the machine running it",
                    properties: %{at: %{type: "integer"}, machine: %{type: ["string", "null"]}}
                  },
                  problem: %{
                    type: ["object", "null"],
                    description: "What's wrong, if anything",
                    properties: %{
                      at: %{type: "integer"},
                      kind: %{type: "string", enum: ["backup", "check"]},
                      text: %{type: "string"}
                    }
                  }
                },
                required: ~w(name kind where machine item size last backups overdue state running problem)
              }
            }
          },
          required: ["every", "retention", "machines", "destinations"]
        }
      },
      %{
        name: "badge",
        read_only: true,
        description:
          "How many destinations need you, the red count on Backups in Settings: their last backup failed, or none finished within their schedule and a day",
        inputs: %{type: "object", properties: %{}},
        outputs: %{type: "object", properties: %{count: %{type: "integer"}}, required: ["count"]}
      },
      %{
        name: "add",
        description:
          "Adds a destination to back up to: a folder on a machine of the network's, as on a disk plugged into it, or S3-compatible storage. It's tried first",
        inputs: %{
          type: "object",
          properties: %{
            name: %{type: "string", description: "What to call it. The folder's or the bucket's name if left out"},
            folder: %{type: "string", description: "A folder, by its whole path on the machine"},
            machine: %{
              type: "string",
              description:
                "The key of the machine the folder is on, which runs Backups.Disk. The first that does, if left out"
            },
            endpoint: %{
              type: "string",
              description: "S3-compatible storage's address, as https://<account>.r2.cloudflarestorage.com"
            },
            bucket: %{type: "string", description: "The bucket's name there, and a folder in it after a /, if any"},
            region: %{type: "string", description: "Its region, if it has them. auto if left out"},
            item: %{
              type: "string",
              description:
                "The network's Vault item with its access_key_id and secret_access_key, switched on for backups"
            }
          }
        },
        outputs: %{
          type: "object",
          properties: %{
            name: destination,
            kind: %{type: "string", enum: ["folder", "s3"]},
            where: %{type: "string"},
            every: %{type: "string", description: "How often backups go there: hour, day, week or never"}
          },
          required: ["name", "kind", "where", "every"]
        }
      },
      %{
        name: "remove",
        description: "Stops backing up to a destination. What's there stays",
        inputs: %{type: "object", properties: %{destination: destination}, required: ["destination"]},
        outputs: %{type: "object", properties: %{destination: destination}, required: ["destination"]}
      },
      %{
        name: "now",
        task_support: :optional,
        description: "Backs up now, to one destination or to each, and answers when it's done with what was sent",
        inputs: %{type: "object", properties: %{destination: destination}},
        outputs: %{type: "object", properties: %{backups: %{type: "array", items: made}}, required: ["backups"]}
      },
      %{
        name: "schedule",
        description: "Sets how often backups are made: every hour, day or week, or never but when asked",
        inputs: %{
          type: "object",
          properties: %{every: %{type: "string", enum: ["hour", "day", "week", "never"]}},
          required: ["every"]
        },
        outputs: %{type: "object", properties: %{every: %{type: "string"}}, required: ["every"]}
      },
      %{
        name: "check",
        description:
          "Reads the newest backup at a destination, or at each, back whole, and compares every object with what was sent",
        inputs: %{type: "object", properties: %{destination: destination}},
        outputs: %{
          type: "object",
          properties: %{
            checks: %{
              type: "array",
              description: "Each destination's newest backup, read back whole: one that isn't as it was sent fails the call",
              items: %{
                type: "object",
                properties: %{
                  destination: destination,
                  backup: %{type: ["string", "null"], description: "The backup read back, or null where there's none yet"},
                  objects: %{type: "integer"},
                  bytes: %{type: "integer"},
                  took: %{type: "integer", description: "In milliseconds"}
                },
                required: ["destination", "backup"]
              }
            }
          },
          required: ["checks"]
        }
      },
      %{
        name: "restore",
        description:
          "Brings back what a backup holds under `blobs` that's gone from the network, such as one person's or one app's data, or a file. It changes nothing that's there",
        inputs: %{
          type: "object",
          properties: %{
            destination: destination,
            backup: %{type: "string", description: "Which backup, as `destinations` lists them. The newest if left out"},
            blobs: %{
              type: "string",
              description:
                "What to bring back, by how its blobs' names start: users/alice/ for a person, users/alice/own/notes/ for what one app keeps for them, or one blob's whole name"
            }
          },
          required: ["destination", "blobs"]
        },
        outputs: %{
          type: "object",
          properties: %{
            backup: %{type: "string", description: "The backup it came from"},
            restored: %{type: "array", items: %{type: "string"}, description: "The blobs brought back"},
            bytes: %{type: "integer", description: "How much was brought back"},
            there: %{type: "integer", description: "How many were there already, unchanged"},
            took: %{type: "integer", description: "In milliseconds"}
          },
          required: ["backup", "restored", "bytes", "there", "took"]
        }
      },
      %{
        name: "due",
        description:
          "Backs up to each destination whose time has come, and carries on each backup cut short. The clock calls it every hour, and every few minutes while a backup is under way",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            backups: %{type: "array", items: made},
            failed: %{type: "array", items: destination, description: "Where it couldn't back up, which destinations says why"},
            running: %{type: "array", items: destination, description: "Where a backup is under way already"}
          },
          required: ["backups", "failed", "running"]
        }
      }
    ]
  end

  @impl true
  def call("due", _inputs, call) do
    if call.actor, do: owner!(call)

    {destinations, every, last, runs} =
      transaction(call, fn db ->
        {destinations(db), setting(db, "every"),
         Map.new(query(db, "select destination, max(at) as at from backups group by destination"), &{&1["destination"], &1["at"]}),
         MapSet.new(query(db, "select destination from runs"), & &1["destination"])}
      end)

    now = System.os_time(:millisecond)
    seconds = @schedules[every]

    # One with a backup under way or cut short is asked too, whatever the schedule: one under way says so, and one
    # cut short carries on. The clock calls every hour on the hour, and a backup that took a while finished after it.
    made =
      for destination <- destinations,
          MapSet.member?(runs, destination["id"]) or (seconds && now - (last[destination["id"]] || 0) >= seconds * 1000 - 600_000) do
        try do
          with :running <- Run.back_up(call, destination), do: {:running, destination["name"]}
        rescue
          _failed -> {:failed, destination["name"]}
        end
      end

    # Its timer goes once no backup is under way (`Backups.Run`).
    if transaction(call, &query(&1, "select 1 from runs")) == [],
      do: Call.app(call, "database").timer(call, "due", %{}, nil)

    stand(call)

    %{
      backups: for(%{} = backup <- made, do: backup),
      failed: for({:failed, name} <- made, do: name),
      running: for({:running, name} <- made, do: name)
    }
  end

  # Settings asks for whoever opens it: the count is for the network's owners, who alone see Backups.
  def call("badge", _inputs, call) do
    count =
      if Call.owns?(%{call | subject: "network"}),
        do: transaction(call, &standing/1) |> Enum.count(& &1.needs),
        else: 0

    %{count: count}
  end

  def call(tool, inputs, call) do
    owner!(call)
    owned(tool, inputs, call)
  end

  # How each destination stands: when it was last backed up (when it was added, if never), how often it's to be, its
  # problem, whether it's overdue, and whether it needs someone: its last backup failed or what's there was changed,
  # or it's overdue.
  defp standing(db) do
    every = @schedules[setting(db, "every")]
    now = System.os_time(:millisecond)
    finished = Map.new(query(db, "select destination, max(at) as at from backups group by destination"), &{&1["destination"], &1["at"]})
    problems = Map.new(query(db, "select * from problems"), &{&1["destination"], &1})
    told = Map.new(query(db, "select * from told"), &{&1["destination"], &1["at"]})

    for destination <- destinations(db) do
      since = finished[destination["id"]] || destination["added"]
      overdue = every != nil and now - since > every * 1000 + @overdue
      problem = problems[destination["id"]]

      %{
        id: destination["id"],
        name: destination["name"],
        last: finished[destination["id"]],
        since: since,
        every: every || 0,
        overdue: overdue,
        problem: problem && problem["text"],
        needs: overdue or problem != nil,
        told: told[destination["id"]]
      }
    end
  end

  # Says how each destination stands where a person hears of it: to Telemetry, whose `backup` alert fires on it, and
  # to the network's owner, told once when it starts to need them, and again only after it's backed up.
  defp stand(call) do
    standing = transaction(call, &standing/1)

    records =
      for destination <- standing do
        %{
          "name" => "one.backup",
          "attributes" =>
            Map.reject(
              %{
                "one.destination" => destination.name,
                "one.backup.last" => destination.since,
                "one.backup.every" => destination.every,
                "one.outcome" => if(destination.needs, do: "failed", else: "ok"),
                "one.problem" => destination.problem
              },
              fn {_name, value} -> value in [nil, false] end
            )
        }
      end

    if records != [], do: Call.tool(call, "telemetry", "record", %{"records" => records})

    for %{needs: true, told: nil} = destination <- standing do
      tell(call, destination)
      transaction(call, &query(&1, "insert or replace into told (destination, at) values (?, ?)", [destination.id, System.os_time(:millisecond)]))
    end

    for %{needs: false, told: told} = destination <- standing, told != nil,
        do: transaction(call, &query(&1, "delete from told where destination = ?", [destination.id]))

    :ok
  end

  # Tells the network's owner that `destination` couldn't be backed up, as Time Machine does: its first owner, whom
  # the network keeps, since a call the clock makes as nobody can't list who else owns it.
  defp tell(call, destination) do
    when_last =
      case destination.last do
        nil -> "#{destination.name} hasn't been backed up yet."
        at -> "The last backup to #{destination.name} was #{ago(System.os_time(:millisecond) - at)}."
      end

    if owner = OneCore.User.first_owner() do
      Call.app(call, "notifications").send(call, %{
        "title" => "Backups couldn't complete the backup",
        "text" => when_last,
        "to" => owner,
        "section" => "backups"
      })
    end
  end

  defp ago(ms) when ms < 2 * 3_600_000, do: "#{div(ms, 60_000)} minutes ago"
  defp ago(ms) when ms < 2 * 86_400_000, do: "#{div(ms, 3_600_000)} hours ago"
  defp ago(ms), do: "#{div(ms, 86_400_000)} days ago"

  # A destination's state, from the backup under way there (`runs`), what's wrong with it and its backups.
  defp state(%{"beat" => beat}, _problem, _listed, now), do: if(now - beat < Run.stale(), do: "backing_up", else: "interrupted")
  defp state(nil, %{"kind" => "check"}, _listed, _now), do: "changed"
  defp state(nil, %{}, _listed, _now), do: "failed"
  defp state(nil, nil, [], _now), do: "never"
  defp state(nil, nil, _listed, _now), do: "backed_up"

  defp owner!(call) do
    cond do
      call.subject != "network" -> raise "Backups are the network's: ask for the network (ONE_SUBJECT=network)."
      not Call.owns?(call) -> raise "Only the network's owners see and change its backups."
      true -> :ok
    end
  end

  defp owned("destinations", _inputs, call) do
    {destinations, every, problems, backups, runs, standing} =
      transaction(call, fn db ->
        {destinations(db), setting(db, "every"), Map.new(query(db, "select * from problems"), &{&1["destination"], &1}),
         query(
           db,
           "select destination, name, at, took, blobs, bytes, sent, sent_bytes from backups order by destination, at desc"
         ), Map.new(query(db, "select * from runs"), &{&1["destination"], &1}), Map.new(standing(db), &{&1.id, &1})}
      end)

    now = System.os_time(:millisecond)

    sizes = sizes(call)
    backups = Enum.group_by(backups, & &1["destination"])

    %{
      every: every,
      retention: @retention,
      machines: for(machine <- Service.servers(call, Disk), do: %{key: machine, name: Service.name(machine)}),
      destinations:
        for destination <- destinations do
          listed = backups[destination["id"]] || []

          %{
            name: destination["name"],
            kind: destination["kind"],
            where: where(destination),
            machine: destination["machine"] && Service.name(destination["machine"]),
            item: destination["item"],
            size: sizes[destination["id"]] || 0,
            last: List.first(listed),
            backups: for(backup <- listed, do: Map.drop(backup, ["destination"])),
            overdue: standing[destination["id"]].overdue,
            state: state(runs[destination["id"]], problems[destination["id"]], listed, now),
            running: with(%{} = run <- runs[destination["id"]], do: %{at: run["at"], machine: Service.name(run["server"])}),
            problem: problems[destination["id"]] && Map.take(problems[destination["id"]], ["at", "kind", "text"])
          }
        end
    }
  end

  defp owned("add", inputs, call) do
    destination =
      case inputs do
        %{"folder" => folder} ->
          if Path.type(folder) != :absolute, do: raise("A folder is given by its whole path, as /mnt/disk/backups.")

          machine =
            inputs["machine"] || List.first(Service.servers(call, Disk)) ||
              raise "No machine runs Backups.Disk, which a folder is reached through: install it on the one the disk is on."

          %{
            "kind" => "folder",
            "folder" => folder,
            "machine" => machine,
            "name" => inputs["name"] || Path.basename(folder)
          }

        %{"endpoint" => endpoint, "bucket" => bucket, "item" => item} ->
          %{
            "kind" => "s3",
            "endpoint" => String.trim_trailing(endpoint, "/"),
            "bucket" => String.trim(bucket, "/"),
            "region" => inputs["region"] || "auto",
            "item" => item,
            "name" => inputs["name"] || bucket |> String.split("/") |> hd()
          }

        _ ->
          raise "Choose a folder, or S3-compatible storage.\n\nSay a `folder` (on a `machine`), or S3-compatible storage's `endpoint`, `bucket` and Vault `item`."
      end

    destination =
      Map.put(destination, "id", Base.encode16(:crypto.strong_rand_bytes(8), case: :lower))

    Backups.Destination.try(call, destination)

    transaction(call, fn db ->
      if query(db, "select 1 from destinations where name = ?", [destination["name"]]) != [],
        do: raise("There's a destination called #{destination["name"]} already.")

      columns = ~w(id name kind machine folder endpoint bucket region item)

      query(
        db,
        "insert into destinations (#{Enum.join(columns, ", ")}, added) values (#{Enum.map_join(columns, ", ", fn _ -> "?" end)}, ?)",
        Enum.map(columns, &destination[&1]) ++ [System.os_time(:millisecond)]
      )
    end)

    %{
      name: destination["name"],
      kind: destination["kind"],
      where: where(destination),
      every: transaction(call, &setting(&1, "every"))
    }
  end

  defp owned("remove", %{"destination" => name}, call) do
    transaction(call, fn db ->
      [destination] = named(db, name)
      for table <- ~w(destinations), do: query(db, "delete from #{table} where id = ?", [destination["id"]])

      for table <- ~w(backups objects problems runs listed told),
          do: query(db, "delete from #{table} where destination = ?", [destination["id"]])
    end)

    Call.tool(call, "telemetry", "record", %{"records" => [%{"name" => "one.backup", "attributes" => %{"one.destination" => name, "one.outcome" => "removed"}}]})

    %{destination: name}
  end

  defp owned("now", inputs, call) do
    destinations = transaction(call, fn db -> if inputs["destination"], do: named(db, inputs["destination"]), else: destinations(db) end)
    if destinations == [], do: raise("There's nowhere to back up to yet: add a destination first.")
    try do
      %{
        backups:
          each(destinations, fn destination ->
            with :running <- Run.back_up(call, destination), do: raise("It's backing up to #{destination["name"]} already.")
          end)
      }
    after
      stand(call)
    end
  end

  defp owned("schedule", %{"every" => every}, call) do
    transaction(call, fn db ->
      query(db, "insert or replace into settings (name, value) values ('every', ?)", [every])
    end)

    %{every: every}
  end

  defp owned("check", inputs, call) do
    destinations = transaction(call, fn db -> if inputs["destination"], do: named(db, inputs["destination"]), else: destinations(db) end)
    %{checks: each(destinations, &Run.check(call, &1))}
  end

  defp owned("restore", %{"destination" => name, "blobs" => blobs} = inputs, call) do
    [destination] = transaction(call, &named(&1, name))
    Run.restore(call, destination, inputs["backup"], blobs)
  end

  # Does `fun` to each destination, and gives what each gave. One that failed doesn't keep the rest from being done,
  # and what failed is raised after.
  defp each(destinations, fun) do
    results =
      for destination <- destinations do
        try do
          {:ok, fun.(destination)}
        rescue
          failed -> {:failed, Exception.message(failed)}
        end
      end

    case for({:failed, why} <- results, do: why) do
      [] -> for {:ok, result} <- results, do: result
      failed -> raise Enum.join(failed, "\n")
    end
  end

  @doc false
  def where(%{"kind" => "folder", "folder" => folder}), do: folder
  def where(%{"kind" => "s3", "endpoint" => endpoint, "bucket" => bucket}), do: "#{URI.parse(endpoint).host}/#{bucket}"

  # How many bytes each destination holds: its objects, and its lists of blobs.
  defp sizes(call) do
    transaction(call, fn db ->
      rows =
        query(db, "select destination, sum(stored) as size from objects group by destination") ++
          query(db, "select destination, sum(stored) as size from backups group by destination")

      Enum.reduce(rows, %{}, fn %{"destination" => destination, "size" => size}, sizes ->
        Map.update(sizes, destination, size, &(&1 + size))
      end)
    end)
  end

  @doc """
  Runs `fun` in a transaction on the app's database, with what `query/3` takes as the database.
  """
  def transaction(call, fun) do
    database = Call.app(call, "database")
    database.transaction(call, @schema, &fun.({database, &1}))
  end

  @doc false
  def query({database, db}, sql, values \\ []), do: database.query(db, sql, values)

  defp destinations(db), do: query(db, "select * from destinations order by added")

  defp named(db, name) do
    case query(db, "select * from destinations where name = ?", [name]) do
      [] -> raise "There's no destination called #{name}.\n\n`destinations` lists them."
      found -> found
    end
  end

  defp setting(db, name) do
    case query(db, "select value from settings where name = ?", [name]) do
      [%{"value" => value}] -> value
      [] -> "day"
    end
  end
end
