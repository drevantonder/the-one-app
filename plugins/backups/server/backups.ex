defmodule Backups do
  @moduledoc """
  Backups: encrypted copies of the whole bucket, every user's and group's blobs, to places that can't read them: a
  folder on one of the network's machines, as on a disk plugged into it, through the service `Backups.Disk`, or
  S3-compatible storage, whose keys are an item in the network's Vault.

  A backup is encrypted to the network's backup key: a key derived from the network's own key, the one its owners'
  devices hold (`plugins/encryption/client/native/src/cipher.rs`, `backups`), whose public half an owner's device hands over as they add a
  place. So a server writes backups and reads none, any owner's device that holds the network's key restores, and
  the network's recovery key alone opens every one, with `one restore` and no network
  running (`plugins/backups/client/native/src/lib.rs`, which says what a place holds).

  What's kept here, in the plugin's database for the network, is what the place can't say without the key: which
  blobs each backup holds, and which of the place's objects holds what, so a blob that's there already isn't sent
  again (`Backups.Run`).
  """
  @behaviour OneCore.Plugin
  alias Backups.{Disk, Run}
  alias OneCore.{Call, Service}

  @schema [
    """
    create table places (id text primary key, name text not null, kind text not null, machine text, folder text,
      endpoint text, bucket text, region text, item text, recipient text not null, whose text not null,
      added integer not null)
    """,
    """
    create table backups (place text not null, name text not null, at integer not null, took integer not null,
      blobs integer not null, bytes integer not null, sent integer not null, sent_bytes integer not null,
      stored integer not null, digest text not null, listed blob not null, primary key (place, name))
    """,
    """
    create table objects (place text not null, object text not null, sha256 text not null, stored integer not null,
      digest text not null, primary key (place, object))
    """,
    "create index objects_by_content on objects (place, sha256)",
    "create table problems (place text primary key, at integer not null, kind text not null, text text not null)",
    "create table settings (name text primary key, value text not null)",
    # Every place's backups are encrypted to the network's backup key, not to whoever added it.
    "alter table places drop column whose"
  ]

  # How often each schedule backs up, in seconds.
  @schedules %{"hour" => 3600, "day" => 86_400, "week" => 7 * 86_400, "never" => nil}

  @kept "Every backup is kept for a day, then one a day for a week, one a week for a month and one a month for a year."

  @impl true
  def uses, do: ["database", "vault"]

  @impl true
  def apps, do: [%{name: "Backups", page: "client/index.html", icon: "client/icon.svg", settings: true}]

  @impl true
  def jobs, do: [%{tool: "due", every: 3600}]

  @impl true
  def tools do
    place = %{type: "string", description: "The place's name, as `places` lists it"}

    [
      %{
        name: "places",
        read_only: true,
        description:
          "Where backups go, each with when it last finished, how big it is, how long it took, its backups and what's wrong if anything is; how often they're made; which are kept; and the machines a folder can be on",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "add",
        description:
          "Adds a place to back up to: a folder on a machine of the network's, as on a disk plugged into it, or S3-compatible storage. It's tried first. Backups to it are encrypted to the network's backup key, so the network's recovery key restores them, and so does any owner's device that holds the network's key",
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
            },
            key: %{type: "string", format: "backup-recipient", opened_by: "opened_by"}
          }
        }
      },
      %{
        name: "remove",
        description: "Stops backing up to a place. What's there stays",
        inputs: %{type: "object", properties: %{place: place}, required: ["place"]}
      },
      %{
        name: "now",
        description: "Backs up now, to one place or to each, and answers when it's done with what was sent",
        inputs: %{type: "object", properties: %{place: place}}
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
          "Reads the newest backup at a place, or at each, back whole, and compares every object with what was sent",
        inputs: %{type: "object", properties: %{place: place}}
      },
      %{
        name: "restore",
        description:
          "Brings back what a backup holds under `blobs` that's gone from the network, such as one person's or one plugin's data, or a file. It changes nothing that's there. Your device hands over the network's backup key",
        inputs: %{
          type: "object",
          properties: %{
            place: place,
            backup: %{type: "string", description: "Which backup, as `places` lists them. The newest if left out"},
            blobs: %{
              type: "string",
              description:
                "What to bring back, by how its blobs' names start: users/alice/ for a person, users/alice/own/notes/ for what one plugin keeps for them, or one blob's whole name"
            },
            key: %{type: "string", format: "backup-key", opened_by: "opened_by"}
          },
          required: ["place", "blobs"]
        }
      },
      %{
        name: "opened_by",
        read_only: true,
        description: "Whose backup key a device hands over, the network's, and the servers it's handed to",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "due",
        description: "Backs up to each place whose time has come. The clock calls it every hour",
        inputs: %{type: "object", properties: %{}}
      }
    ]
  end

  @impl true
  def call("opened_by", _inputs, call) do
    %{owner: "network", servers: OneCore.Servers.acting(call.added)}
  end

  def call("due", _inputs, call) do
    if call.by, do: owner!(call)

    {places, every, last} =
      transaction(call, fn db ->
        {places(db), setting(db, "every"),
         Map.new(query(db, "select place, max(at) as at from backups group by place"), &{&1["place"], &1["at"]})}
      end)

    now = System.os_time(:millisecond)

    if seconds = @schedules[every] do
      # The clock calls every hour on the hour, and a backup that took a while finished after it.
      for place <- places, now - (last[place["id"]] || 0) >= seconds * 1000 - 600_000 do
        try do
          Run.back_up(call, place)
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
      call.for != "network" -> raise "Backups are the network's: ask for the network (ONE_FOR=network)."
      not Call.owns?(call) -> raise "Only the network's owners see and change its backups."
      true -> :ok
    end
  end

  defp owned("places", _inputs, call) do
    {places, every, problems, backups} =
      transaction(call, fn db ->
        {places(db), setting(db, "every"), Map.new(query(db, "select * from problems"), &{&1["place"], &1}),
         query(
           db,
           "select place, name, at, took, blobs, bytes, sent, sent_bytes from backups order by place, at desc"
         )}
      end)

    sizes = sizes(call)
    backups = Enum.group_by(backups, & &1["place"])

    %{
      every: every,
      kept: @kept,
      machines: for(machine <- Service.servers(call, Disk), do: %{key: machine, name: Service.name(machine)}),
      places:
        for place <- places do
          listed = backups[place["id"]] || []

          %{
            name: place["name"],
            kind: place["kind"],
            where: where(place),
            machine: place["machine"] && Service.name(place["machine"]),
            item: place["item"],
            size: sizes[place["id"]] || 0,
            last: List.first(listed),
            backups: for(backup <- listed, do: Map.drop(backup, ["place"])),
            problem: problems[place["id"]] && Map.take(problems[place["id"]], ["at", "kind", "text"])
          }
        end
    }
  end

  defp owned("add", inputs, call) do
    recipient =
      inputs["key"] ||
        raise "Your device hands over the network's backup key: add a place from the app, or with `one`."

    place =
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

    place =
      Map.merge(place, %{"id" => Base.encode16(:crypto.strong_rand_bytes(8), case: :lower), "recipient" => recipient})

    Backups.Place.try(call, place)

    transaction(call, fn db ->
      if query(db, "select 1 from places where name = ?", [place["name"]]) != [],
        do: raise("There's a place called #{place["name"]} already.")

      columns = ~w(id name kind machine folder endpoint bucket region item recipient)

      query(
        db,
        "insert into places (#{Enum.join(columns, ", ")}, added) values (#{Enum.map_join(columns, ", ", fn _ -> "?" end)}, ?)",
        Enum.map(columns, &place[&1]) ++ [System.os_time(:millisecond)]
      )
    end)

    "Added #{place["name"]}. Backups go there #{every(call)}, and the network's recovery key restores them."
  end

  defp owned("remove", %{"place" => name}, call) do
    transaction(call, fn db ->
      [place] = named(db, name)
      for table <- ~w(places), do: query(db, "delete from #{table} where id = ?", [place["id"]])

      for table <- ~w(backups objects problems),
          do: query(db, "delete from #{table} where place = ?", [place["id"]])
    end)

    "No longer backing up to #{name}. What's there stays."
  end

  defp owned("now", inputs, call) do
    places = transaction(call, fn db -> if inputs["place"], do: named(db, inputs["place"]), else: places(db) end)
    if places == [], do: raise("There's nowhere to back up to yet: add a place first.")
    each(places, &Run.back_up(call, &1))
  end

  defp owned("schedule", %{"every" => every}, call) do
    transaction(call, fn db ->
      query(db, "insert or replace into settings (name, value) values ('every', ?)", [every])
    end)

    "Backups are made #{every(call)}."
  end

  defp owned("check", inputs, call) do
    places = transaction(call, fn db -> if inputs["place"], do: named(db, inputs["place"]), else: places(db) end)
    each(places, &Run.check(call, &1))
  end

  defp owned("restore", %{"place" => name, "blobs" => blobs} = inputs, call) do
    given =
      inputs["key"] || raise "Your device hands over the network's backup key: restore from the app, or with `one`."

    [place] = transaction(call, &named(&1, name))
    Run.restore(call, place, inputs["backup"], blobs, given)
  end

  # Does `fun` to each place, and says what each said. One that failed doesn't keep the rest from being done, and
  # what failed is raised after.
  defp each(places, fun) do
    said =
      for place <- places do
        try do
          {:ok, fun.(place)}
        rescue
          failed -> {:failed, Exception.message(failed)}
        end
      end

    text = Enum.map_join(said, "\n", &elem(&1, 1))
    if Enum.any?(said, &match?({:failed, _}, &1)), do: raise(text), else: text
  end

  defp every(call) do
    case transaction(call, &setting(&1, "every")) do
      "never" -> "only when asked"
      every -> "every #{every}"
    end
  end

  @doc false
  def where(%{"kind" => "folder", "folder" => folder}), do: folder
  def where(%{"kind" => "s3", "endpoint" => endpoint, "bucket" => bucket}), do: "#{URI.parse(endpoint).host}/#{bucket}"

  # How many bytes each place holds: its objects, and its lists of blobs.
  defp sizes(call) do
    transaction(call, fn db ->
      rows =
        query(db, "select place, sum(stored) as size from objects group by place") ++
          query(db, "select place, sum(stored) as size from backups group by place")

      Enum.reduce(rows, %{}, fn %{"place" => place, "size" => size}, sizes ->
        Map.update(sizes, place, size, &(&1 + size))
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

  defp places(db), do: query(db, "select * from places order by added")

  defp named(db, name) do
    case query(db, "select * from places where name = ?", [name]) do
      [] -> raise "There's no place called #{name}. `places` lists them."
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
