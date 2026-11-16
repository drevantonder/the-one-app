defmodule Shelf do
  @moduledoc """
  Notes as rows of a database, and files as blobs, each in an artifact of whoever it runs for, for the test of
  backups (`plugins/backups/test/backups.sh`): what a person's or a group's data is, in the least a plugin does to keep it.
  """
  @behaviour OneCore.Plugin
  alias OneCore.{Artifact, Call}

  @schema ["create table notes (id integer primary key, text text not null)"]

  @impl true
  def uses, do: ["database"]

  @impl true
  def tools do
    text = %{type: "object", properties: %{text: %{type: "string"}}, required: ["text"]}
    file = %{type: "object", properties: %{name: %{type: "string"}}, required: ["name"]}
    done = %{type: "object"}

    [
      %{name: "add", description: "Adds a note", inputs: text, outputs: done},
      %{
        name: "change",
        description: "Changes every note that says `text` to say `to`",
        inputs: %{
          type: "object",
          properties: %{text: %{type: "string"}, to: %{type: "string"}},
          required: ["text", "to"]
        },
        outputs: done
      },
      %{
        name: "put",
        description: "Keeps a file: `text`, or random bytes as big as `size`",
        inputs: %{
          type: "object",
          properties: %{name: %{type: "string"}, text: %{type: "string"}, size: %{type: "integer"}},
          required: ["name"]
        },
        outputs: file
      },
      %{name: "delete", description: "Deletes a file", inputs: file, outputs: file},
      %{
        name: "all",
        read_only: true,
        description: "The notes, and each file's SHA-256 by its name",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            notes: %{type: "array", items: %{type: "string"}},
            files: %{type: "object", additionalProperties: %{type: "string"}}
          },
          required: ["notes", "files"]
        }
      }
    ]
  end

  @impl true
  def call("add", %{"text" => text}, call) do
    transaction(call, fn database, db -> database.query(db, "insert into notes (text) values (?)", [text]) end)
    %{}
  end

  def call("change", %{"text" => text, "to" => to}, call) do
    transaction(call, fn database, db -> database.query(db, "update notes set text = ? where text = ?", [to, text]) end)
    %{}
  end

  def call("put", %{"name" => name} = inputs, call) do
    :ok = Artifact.write(files(call), name, inputs["text"] || :crypto.strong_rand_bytes(inputs["size"] || 0))
    %{name: name}
  end

  def call("delete", %{"name" => name}, call) do
    Artifact.remove(files(call), name)
    %{name: name}
  end

  def call("all", _inputs, call) do
    notes = transaction(call, fn database, db -> database.query(db, "select text from notes order by id") end)
    files = files(call)

    %{
      notes: Enum.map(notes, & &1["text"]),
      files:
        for name <- Artifact.list(files), {:ok, data, _tag} <- [Artifact.read(files, name)], into: %{} do
          {name, Base.encode16(:crypto.hash(:sha256, data), case: :lower)}
        end
    }
  end

  defp files(call), do: Call.make(call, "files")

  defp transaction(call, fun) do
    database = Call.plugin(call, "database")
    notes = Call.make(call, "notes")
    call = %{call | own: notes, folder: Path.join(OneCore.Server.home(), Artifact.name(notes))}
    database.transaction(call, @schema, &fun.(database, &1))
  end
end
