defmodule Locker do
  @moduledoc """
  Keeps one note encrypted, as a blob and as a row of a database, for the tests of encryption
  (`plugins/vault/test/vault.sh`): the least a plugin does to keep something that whoever runs the bucket can't read.
  """
  @behaviour OneCore.Plugin
  alias OneCore.{Artifact, Call}

  @schema ["create table notes (text text not null)"]

  @impl true
  def uses, do: ["encryption", "database"]

  @impl true
  def tools do
    none = %{type: "object", properties: %{}}
    text = %{type: "string", format: "secret", unlocked_on: "unlocked_on", description: "What to keep"}
    keys = %{type: "string", format: "artifact-keys", unlocked_on: "unlocked_on"}

    [
      %{name: "ping", read_only: true, description: "Answers, calling nothing", inputs: none, outputs: none},
      %{
        name: "keep",
        description: "Keeps a note, encrypted",
        inputs: %{type: "object", properties: %{text: text}, required: ["text"]},
        outputs: none
      },
      %{
        name: "read",
        read_only: true,
        description: "Opens the note, and gives its SHA-256 and the server that opened it",
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{blob: %{type: "string"}, rows: %{type: "array", items: %{type: "string"}}, opened_on: %{type: "string"}},
          required: ["blob", "rows", "opened_on"]
        }
      },
      # What the encryption plugin answers, as its own tools say.
      %{name: "unlocked_on", read_only: true, description: "Says which servers open your locker", inputs: none, outputs: none},
      %{
        name: "grant",
        description: "Chooses the servers that open your locker",
        inputs: %{
          type: "object",
          properties: %{servers: %{type: "array", items: %{type: "string"}}, keys: keys},
          required: ["servers"]
        },
        outputs: %{type: "object", properties: %{servers: %{type: "array", items: %{type: "string"}}}, required: ["servers"]}
      }
    ]
  end

  @impl true
  def call("ping", _inputs, _call), do: %{}
  def call("unlocked_on", _inputs, call), do: encryption(call).unlocked_on(call, Call.make(call, "notes"))
  def call("grant", %{"keys" => keys}, call), do: %{servers: encryption(call).choose(call, Call.make(call, "notes"), keys)}

  def call("keep", %{"text" => text}, call) do
    notes = notes(call)
    text = encryption(call).secret(call, text)
    Artifact.write(notes, "note", text)
    transaction(call, notes, fn database, db -> database.query(db, "insert into notes (text) values (?)", [text]) end)
    %{}
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
