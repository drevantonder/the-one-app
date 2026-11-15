defmodule Locker do
  @moduledoc """
  Keeps one note encrypted, as a blob and as a row of a database, for the proofs of encryption
  (`core/test/vault.sh`): the least a plugin does to keep something that whoever runs the bucket can't read.
  """
  @behaviour OneCore.Plugin
  alias OneCore.{Artifact, Call}

  @schema ["create table notes (text text not null)"]

  @impl true
  def uses, do: ["encryption", "database"]

  @impl true
  def tools do
    none = %{type: "object", properties: %{}}
    text = %{type: "string", format: "secret", opened_by: "opened_by", description: "What to keep"}
    keys = %{type: "string", format: "artifact-keys", opened_by: "opened_by"}

    [
      %{name: "ping", read_only: true, description: "Answers, calling nothing", inputs: none},
      %{
        name: "keep",
        description: "Keeps a note, encrypted",
        inputs: %{type: "object", properties: %{text: text}, required: ["text"]}
      },
      %{
        name: "read",
        read_only: true,
        description: "Opens the note, and gives its SHA-256 and the server that opened it",
        inputs: none
      },
      %{name: "opened_by", read_only: true, description: "Says which servers open your locker", inputs: none},
      %{
        name: "grant",
        description: "Chooses the servers that open your locker",
        inputs: %{
          type: "object",
          properties: %{servers: %{type: "array", items: %{type: "string"}}, keys: keys},
          required: ["servers"]
        }
      }
    ]
  end

  @impl true
  def call("ping", _inputs, _call), do: "pong"
  def call("opened_by", _inputs, call), do: encryption(call).opened_by(call, Call.make(call, "notes"))
  def call("grant", %{"keys" => keys}, call), do: encryption(call).choose(call, Call.make(call, "notes"), keys)

  def call("keep", %{"text" => text}, call) do
    notes = notes(call)
    text = encryption(call).secret(call, text)
    Artifact.write(notes, "note", text)
    transaction(call, notes, fn database, db -> database.query(db, "insert into notes (text) values (?)", [text]) end)
    "Kept."
  end

  def call("read", _inputs, call) do
    notes = notes(call)
    {:ok, blob, _tag} = Artifact.read(notes, "note")
    rows = transaction(call, notes, fn database, db -> database.query(db, "select text from notes") end)
    sha256 = &Base.encode16(:crypto.hash(:sha256, &1), case: :lower)
    %{blob: sha256.(blob), rows: Enum.map(rows, &sha256.(&1["text"])), opened_on: call.server}
  end

  defp encryption(call), do: Call.plugin(call, "encryption")
  defp notes(call), do: encryption(call).encrypted(call, Call.make(call, "notes"))

  defp transaction(call, notes, fun) do
    database = Call.plugin(call, "database")
    call = %{call | own: notes, folder: Path.join(OneCore.Server.home(), Artifact.name(notes))}
    database.transaction(call, @schema, &fun.(database, &1))
  end
end
