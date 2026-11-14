defmodule Tracker do
  @moduledoc "Trackers for anything counted over time, kept in the plugin's database."
  @behaviour OneCore.Plugin

  @schema [
    """
    create table entries (
      id integer primary key, tracker text not null, amount numeric not null, note text, at text not null
    )
    """,
    "create index entries_by_tracker on entries (tracker, id)"
  ]

  @impl true
  def uses, do: ["database"]

  @impl true
  def guide do
    """
    Trackers count things over time: glasses of water, push-ups, hours slept. Add an entry when someone says they
    did something they track, and make a new tracker just by adding to it. An entry is stamped with the time it's
    added.
    """
  end

  @impl true
  def apps, do: [%{name: "Tracker", page: "app.html", icon: "icon.svg"}]

  @impl true
  def tools do
    [
      %{
        name: "add",
        description: "Adds an entry to a tracker, making the tracker if it's new",
        inputs: %{
          type: "object",
          properties: %{
            tracker: %{type: "string", description: "The tracker's name, such as water"},
            amount: %{type: "number", description: "How much. 1 if left out"},
            note: %{type: "string", description: "Anything worth remembering about this entry"}
          },
          required: ["tracker"]
        }
      },
      %{
        name: "list",
        read_only: true,
        description: "Lists the trackers with their totals, or one tracker's entries",
        inputs: %{
          type: "object",
          properties: %{
            tracker: %{type: "string", description: "The tracker whose entries to list. All trackers if left out"}
          }
        }
      }
    ]
  end

  @impl true
  def call("add", %{"tracker" => tracker} = inputs, call) do
    database = OneCore.Call.plugin(call, "database")
    entry = [tracker, Map.get(inputs, "amount", 1), inputs["note"], DateTime.to_iso8601(DateTime.utc_now(:second))]

    database.transaction(call, @schema, fn db ->
      database.query(db, "insert into entries (tracker, amount, note, at) values (?, ?, ?, ?)", entry)

      [%{"total" => total, "entries" => entries}] =
        database.query(db, "select sum(amount) as total, count(*) as entries from entries where tracker = ?", [tracker])

      "Added. #{tracker} is at #{total} over #{entries} entries."
    end)
  end

  def call("list", %{"tracker" => tracker}, call) do
    database = OneCore.Call.plugin(call, "database")

    database.transaction(call, @schema, fn db ->
      case database.query(db, "select at, amount, note from entries where tracker = ? order by id", [tracker]) do
        [] -> raise "There's no tracker named #{tracker}"
        entries -> entries
      end
    end)
  end

  def call("list", _inputs, call) do
    database = OneCore.Call.plugin(call, "database")

    database.transaction(call, @schema, fn db ->
      database.query(db, """
      select tracker, sum(amount) as total, count(*) as entries from entries group by tracker order by tracker
      """)
    end)
  end
end
