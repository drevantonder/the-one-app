defmodule Tally do
  @moduledoc "Notes installed one at a time to the plugin's database, for the tests that write and read one."
  @behaviour OneCore.Plugin

  @schema ["create table notes (id integer primary key, note text not null)"]

  @impl true
  def uses, do: ["database"]

  @impl true
  def tools do
    [
      %{
        name: "add",
        description: "Adds a note",
        inputs: %{type: "object", properties: %{note: %{type: "string"}}, required: ["note"]}
      },
      %{name: "list", read_only: true, description: "Lists the notes", inputs: %{type: "object"}}
    ]
  end

  @impl true
  def call("add", %{"note" => note}, call) do
    database = OneCore.Call.plugin(call, "database")

    database.transaction(call, @schema, fn db ->
      database.query(db, "insert into notes (note) values (?)", [note])
      "Installed."
    end)
  end

  def call("list", _inputs, call) do
    database = OneCore.Call.plugin(call, "database")
    database.transaction(call, @schema, &database.query(&1, "select note from notes order by id"))
  end
end
