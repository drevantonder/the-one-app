defmodule Backups do
  @moduledoc """
  Backups: copies of the whole bucket, every user's and group's blobs as they are, to a folder on one of the network's
  machines, as on a disk plugged into it, through the service `Backups.Disk`, or to S3-compatible storage, whose keys
  are an item in the network's Vault. A blob that's encrypted in the bucket is encrypted in a backup too. Whoever can
  reach the destination restores, with `one restore` and no network running (`plugins/backups/client/native/src/lib.rs`,
  which says what a destination holds).

  What's kept here, in the plugin's database for the network, is which blobs each backup holds, and which of the
  destination's objects holds what, so a blob that's there already isn't sent again (`Backups.Run`).
  """
  @behaviour OneCore.Plugin
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
    "alter table destinations drop column recipient"
  ]

  # How often each schedule backs up, in seconds.
  @schedules %{"hour" => 3600, "day" => 86_400, "week" => 7 * 86_400, "never" => nil}

  @kept "Every backup is kept for a day, then one a day for a week, one a week for a month and one a month for a year."

  @impl true
  def uses, do: ["database", "vault"]

  @impl true
  def apps, do: [%{name: "Backups", page: "client/index.html", icon: "client/icon.svg", category: "settings"}]

  @impl true
  def jobs, do: [%{tool: "due", every: 3600}]

  @impl true
  def tools do
    destination = %{type: "string", description: "The destination's name, as `destinations` lists it"}

    [
      %{
        name: "destinations",
        read_only: true,
        description:
          "Where backups go, each with when it last finished, how big it is, how long it took, its backups and what's wrong if anything is; how often they're made; which are kept; and the machines a folder can be on",
        inputs: %{type: "object", properties: %{}}
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
        }
      },
      %{
        name: "remove",
        description: "Stops backing up to a destination. What's there stays",
        inputs: %{type: "object", properties: %{destination: destination}, required: ["destination"]}
      },
      %{
        name: "now",
        description: "Backs up now, to one destination or to each, and answers when it's done with what was sent",
        inputs: %{type: "object", properties: %{destination: destination}}
      },
      %{
        name: "schedule",
        description: "Sets how often backups are made: every hour, day or week, or never but when asked",
        inputs: %{
          type: "object",
          properties: %{every: %{type: "string", enum: ["hour", "day", "week", "never"]}},
          required: ["every"]
        }
      },
      %{
        name: "check",
        description:
          "Reads the newest backup at a destination, or at each, back whole, and compares every object with what was sent",
        inputs: %{type: "object", properties: %{destination: destination}}
      },
      %{
        name: "restore",
        description:
          "Brings back what a backup holds under `blobs` that's gone from the network, such as one person's or one plugin's data, or a file. It changes nothing that's there",
        inputs: %{
          type: "object",
          properties: %{
            destination: destination,
            backup: %{type: "string", description: "Which backup, as `destinations` lists them. The newest if left out"},
            blobs: %{
              type: "string",
              description:
                "What to bring back, by how its blobs' names start: users/alice/ for a person, users/alice/own/notes/ for what one plugin keeps for them, or one blob's whole name"
            }
          },
          required: ["destination", "blobs"]
        }
      },
      %{
        name: "due",
        description: "Backs up to each destination whose time has come. The clock calls it every hour",
        inputs: %{type: "object", properties: %{}}
      }
    ]
  end

  @impl true
  def call("due", _inputs, call) do
    if call.actor, do: owner!(call)

    {destinations, every, last} =
      transaction(call, fn db ->
        {destinations(db), setting(db, "every"),
         Map.new(query(db, "select destination, max(at) as at from backups group by destination"), &{&1["destination"], &1["at"]})}
      end)

    now = System.os_time(:millisecond)

    if seconds = @schedules[every] do
      # The clock calls every hour on the hour, and a backup that took a while finished after it.
      for destination <- destinations, now - (last[destination["id"]] || 0) >= seconds * 1000 - 600_000 do
        try do
          Run.back_up(call, destination)
        rescue
          _failed -> :kept_as_a_problem
        end
      end
    end

    "Done."
  end

  def call(tool, inputs, call) do
    owner!(call)
    owned(tool, inputs, call)
  end

  defp owner!(call) do
    cond do
      call.subject != "network" -> raise "Backups are the network's: ask for the network (ONE_FOR=network)."
      not Call.owns?(call) -> raise "Only the network's owners see and change its backups."
      true -> :ok
    end
  end

  defp owned("destinations", _inputs, call) do
    {destinations, every, problems, backups} =
      transaction(call, fn db ->
        {destinations(db), setting(db, "every"), Map.new(query(db, "select * from problems"), &{&1["destination"], &1}),
         query(
           db,
           "select destination, name, at, took, blobs, bytes, sent, sent_bytes from backups order by destination, at desc"
         )}
      end)

    sizes = sizes(call)
    backups = Enum.group_by(backups, & &1["destination"])

    %{
      every: every,
      kept: @kept,
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
          raise "Say a `folder` (on a `machine`), or S3-compatible storage's `endpoint`, `bucket` and Vault `item`."
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

    "Added #{destination["name"]}. Backups go there #{every(call)}."
  end

  defp owned("remove", %{"destination" => name}, call) do
    transaction(call, fn db ->
      [destination] = named(db, name)
      for table <- ~w(destinations), do: query(db, "delete from #{table} where id = ?", [destination["id"]])

      for table <- ~w(backups objects problems),
          do: query(db, "delete from #{table} where destination = ?", [destination["id"]])
    end)

    "No longer backing up to #{name}. What's there stays."
  end

  defp owned("now", inputs, call) do
    destinations = transaction(call, fn db -> if inputs["destination"], do: named(db, inputs["destination"]), else: destinations(db) end)
    if destinations == [], do: raise("There's nowhere to back up to yet: add a destination first.")
    each(destinations, &Run.back_up(call, &1))
  end

  defp owned("schedule", %{"every" => every}, call) do
    transaction(call, fn db ->
      query(db, "insert or replace into settings (name, value) values ('every', ?)", [every])
    end)

    "Backups are made #{every(call)}."
  end

  defp owned("check", inputs, call) do
    destinations = transaction(call, fn db -> if inputs["destination"], do: named(db, inputs["destination"]), else: destinations(db) end)
    each(destinations, &Run.check(call, &1))
  end

  defp owned("restore", %{"destination" => name, "blobs" => blobs} = inputs, call) do
    [destination] = transaction(call, &named(&1, name))
    Run.restore(call, destination, inputs["backup"], blobs)
  end

  # Does `fun` to each destination, and says what each said. One that failed doesn't keep the rest from being done, and
  # what failed is raised after.
  defp each(destinations, fun) do
    said =
      for destination <- destinations do
        try do
          {:ok, fun.(destination)}
        rescue
          failed -> {:failed, Exception.message(failed)}
        end
      end

    text = Enum.map_join(said, "\n", &elem(&1, 1))
    if Enum.any?(said, &match?({:failed, _}, &1)), do: raise(text), else: text
  end

  defp every(call) do
    case transaction(call, &setting(&1, "every")) do
      "never" -> "manually"
      every -> "every #{every}"
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
  Runs `fun` in a transaction on the plugin's database, with what `query/3` takes as the database.
  """
  def transaction(call, fun) do
    database = Call.plugin(call, "database")
    database.transaction(call, @schema, &fun.({database, &1}))
  end

  @doc false
  def query({database, db}, sql, values \\ []), do: database.query(db, sql, values)

  defp destinations(db), do: query(db, "select * from destinations order by added")

  defp named(db, name) do
    case query(db, "select * from destinations where name = ?", [name]) do
      [] -> raise "There's no destination called #{name}. `destinations` lists them."
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
