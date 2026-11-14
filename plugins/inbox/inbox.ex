defmodule Inbox do
  @moduledoc "Folders on a network's machines, watched for the files that arrive in them."
  @behaviour OneCore.Plugin
  alias Inbox.Watcher

  @schema [
    "create table watched (server text primary key, folder text not null)",
    """
    create table arrived (
      id integer primary key, server text not null, folder text not null, name text not null, at text not null
    )
    """
  ]

  @impl true
  def uses, do: ["database"]

  @impl true
  def guide do
    """
    Inbox watches a folder on a machine and keeps a list of the files that arrive in it: scans, downloads, exports,
    whatever lands there. `list` gives what arrived, from any machine, whether or not the one watching is up.

    The watching is done on the machine itself, by Inbox's service `Inbox.Watcher`. `services` says which servers
    run it, and `install_service` runs it on one. Then `watch` tells that machine which folder to watch, and
    `watching` says what each machine is watching now. A file that was already there when watching began isn't
    an arrival.
    """
  end

  @impl true
  def tools do
    [
      %{
        name: "watch",
        description: "Watches a folder on a machine for files that arrive in it, in place of any it watched before",
        inputs: %{
          type: "object",
          properties: %{
            folder: %{type: "string", description: "The folder's path on that machine. ~ is its user's home"},
            server: %{type: "string", description: "The server's key. The first that runs the service, if left out"}
          },
          required: ["folder"]
        }
      },
      %{
        name: "watching",
        read_only: true,
        description: "Says which folder each machine is watching, since when, and how many files arrived since",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "list",
        read_only: true,
        description: "Lists the files that arrived, the newest first",
        inputs: %{
          type: "object",
          properties: %{limit: %{type: "integer", description: "How many at most. 50 if left out"}}
        }
      }
    ]
  end

  # The service is told first, since it refuses a folder its machine doesn't have. Telling it twice does no harm,
  # which is what happens when the database passes this call on to another server.
  @impl true
  def call("watch", %{"folder" => folder} = inputs, call) do
    database = OneCore.Call.plugin(call, "database")
    server = inputs["server"] || List.first(OneCore.Service.servers(call, Watcher))
    folder = OneCore.Service.call(call, Watcher, :watch, [call.added, folder], server)

    database.transaction(call, @schema, fn db ->
      database.query(
        db,
        "insert into watched (server, folder) values (?, ?) on conflict (server) do update set folder = excluded.folder",
        [server, folder]
      )
    end)

    "Watching #{folder} on the server #{server}."
  end

  def call("watching", _inputs, call) do
    for server <- OneCore.Service.servers(call, Watcher),
        watching = OneCore.Service.call(call, Watcher, :watching, [call.added], server) do
      Map.put(watching, "server", server)
    end
  end

  def call("list", inputs, call) do
    database = OneCore.Call.plugin(call, "database")

    database.transaction(call, @schema, fn db ->
      database.query(db, "select name, folder, server, at from arrived order by id desc limit ?", [
        Map.get(inputs, "limit", 50)
      ])
    end)
  end

  @doc "The folder a server's watcher was last told to watch, for the watcher when it starts."
  def watched(call, server) do
    database = OneCore.Call.plugin(call, "database")

    database.transaction(call, @schema, fn db ->
      case database.query(db, "select folder from watched where server = ?", [server]) do
        [%{"folder" => folder}] -> folder
        [] -> nil
      end
    end)
  end

  @doc "Keeps a file that a server's watcher saw arrive."
  def arrived(call, server, folder, name) do
    database = OneCore.Call.plugin(call, "database")
    at = DateTime.to_iso8601(DateTime.utc_now(:second))

    database.transaction(call, @schema, fn db ->
      database.query(db, "insert into arrived (server, folder, name, at) values (?, ?, ?, ?)", [
        server,
        folder,
        name,
        at
      ])
    end)

    :ok
  end
end
