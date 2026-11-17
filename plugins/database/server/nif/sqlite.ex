defmodule Sqlite do
  @moduledoc """
  SQLite: the database plugin's NIF (`nif/sqlite`), which its Elixir calls, and so could a plugin that uses
  `database`.

      db = Sqlite.open("/tmp/notes.sqlite")
      Sqlite.query(db, "create table notes (id integer primary key, text text)", [])
      Sqlite.query(db, "insert into notes (text) values (?) returning id", ["Buy milk"])
      #=> {:ok, [%{"id" => 1}]}

  A value is a number, text, `nil`, a boolean (kept as 1 or 0) or `{:blob, binary}` for bytes that aren't text.
  """
  use OneCore.Nif

  @doc "Opens the database file at `path`, making it if there's none. It closes when nothing holds it."
  def open(_path), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Runs one statement with its values. Gives `{:ok, rows}`, each row a map from a column's name to its value."
  def query(_database, _sql, _values), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Closes the database file now, so its files can be removed."
  def close(_database), do: :erlang.nif_error(:nif_not_loaded)
end
