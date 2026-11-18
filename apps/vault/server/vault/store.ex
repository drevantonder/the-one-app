defmodule Vault.Store do
  @moduledoc """
  Where someone's Vault is kept. Each item is an artifact of its own, `<owner>/made/vault/<id>`, so soft delete deletes,
  restores and purges one as it does anything an app made, and the Vault keeps no lifecycle of its own: it stores and
  encrypts. An item's record is the blob `<id>/item`, encrypted with the Vault's keys, and changed in one step
  (`change!/3`); its contents are blobs beside it, `<id>/contents/<blob>`, each sealed under a key the record holds.
  Their Vault's database, encrypted the same, says which id each name is (`artifacts`), so finding one by name is one
  read, and one soft delete took aside isn't there to read until it's restored. It keeps what's been asked for
  (`Vault.Requests`) and the sign-ins under way (`Vault.SignIns`) too.
  """

  alias OneCore.{Artifact, Call, Server}

  # A database runs the statements it hasn't, as many as its `user_version` counts (`Database.transaction/3`). The
  # Vaults made before the schema was squashed to what follows ran 20 statements, so a new one counts the same: a
  # statement added at the end is the 21st for a Vault of either kind.
  @squashed List.duplicate("select 1", 17)

  @schema @squashed ++ [
    # Which item each name is, as each item is an artifact of its own.
    "create table artifacts (id text primary key, name text not null)",
    """
    create table requests (
      id text primary key, title text not null, fields text not null, hosts text not null, app text, why text,
      state text not null, at integer not null, by text
    ) without rowid
    """,
    # Where each sign-in began, the apps it's for, and how it ended, which its page says (`Vault.SignIns`).
    """
    create table sign_ins (
      state text primary key, verifier text not null, service text not null, app text not null,
      item text not null, at integer not null, began text, apps text, ended text, ending text
    ) without rowid
    """
  ]

  # Someone's Vault is theirs alone: no relation tuple has anyone else ask for a user, the network's owner included.
  defp mine!(%Call{subject: subject, actor: actor} = call) do
    if String.starts_with?(subject, "users/") and actor not in [subject, nil],
      do: raise("#{subject}'s Vault is theirs alone.")

    call
  end

  @doc "The artifact of the database of whoever a call of the Vault's own tools is for."
  def items_artifact(call), do: Call.make(mine!(call), "items")

  @doc "Their Vault, opened: on a server they chose, which this call ends up on."
  def own(call), do: encryption(call).encrypted(call, items_artifact(call))

  @doc "The same artifact by its name, for an app that uses the Vault, which finds it added as `vault`."
  def theirs(call), do: "#{mine!(call).subject}/made/vault/items"

  @doc "Their Vault, for an app that uses it: it reaches it as an editor, which a tuple says once it has access to an item."
  def reached(call), do: encryption(call).encrypted(call, Call.reach(call, theirs(call), "editor"))

  def encryption(call), do: Call.app(call, "encryption")

  @doc "A transaction on someone's Vault's database. It's theirs, so this server's copy of it is kept by its name."
  def transaction(call, artifact, fun) do
    database = Call.app(call, "database")
    call = %{call | own: artifact, folder: Path.join(Server.home(), Artifact.name(artifact))}
    database.transaction(call, @schema, &fun.({database, &1}))
  end

  def query({database, db}, sql, values \\ []), do: database.query(db, sql, values)

  @doc "Where someone's items are: their Vault's database, with its keys and its reach, at the folder it's in."
  def root(%Artifact{} = db), do: %Artifact{db | prefix: String.replace_suffix(db.prefix, "items/", "")}

  @doc "An item's artifact, by its id, as soft delete knows it."
  def artifact(db, id), do: root(db).prefix <> id

  @doc "An item's record, by its id, or nil where it's gone or aside."
  def record(db, id) do
    case Artifact.read(root(db), "#{id}/item") do
      {:ok, data, _tag} -> JSON.decode!(data)
      :absent -> nil
    end
  end

  @doc "Changes an item's record in one step: `change` is given it as it is, and gives what it's to be."
  def change!(db, item, change) do
    changed =
      Artifact.update(root(db), "#{item["id"]}/item", fn
        nil -> nil
        data -> data |> JSON.decode!() |> change.() |> Map.put("changed", now()) |> JSON.encode!()
      end)

    if changed, do: JSON.decode!(changed), else: raise("There's no item named #{item["name"]} in the Vault.")
  end

  @doc "The item named `name`, any case: the first whose record is there."
  def find(call, db, name) do
    ids = transaction(call, db, &for(%{"id" => id} <- query(&1, "select id from artifacts where name = ? collate nocase", [name]), do: id))
    Enum.find_value(ids, &record(db, &1))
  end

  def find!(call, db, name), do: find(call, db, name) || raise("There's no item named #{name} in the Vault.")

  @doc "Every item that's there."
  def everything(call, db) do
    ids = transaction(call, db, &for(%{"id" => id} <- query(&1, "select id from artifacts"), do: id))
    for {:ok, data, _tag} <- Artifact.read_all(root(db), Enum.map(ids, &"#{&1}/item")), do: JSON.decode!(data)
  end

  @doc "Whether a name is another item's than `id`'s."
  def taken?(call, db, name, id \\ nil), do: transaction(call, db, &(taken(&1, db, name, id) != []))

  defp taken(d, db, name, id) do
    for %{"id" => other} <- query(d, "select id from artifacts where name = ? collate nocase", [name]),
        other != id,
        record(db, other),
        do: other
  end

  @doc "A new item, made once its name is free: `record` is all of it but its id, name and when it changed."
  def create(call, db, name, record) do
    if String.trim(name) == "", do: raise("An item has a name.")
    id = random(9)

    transaction(call, db, fn d ->
      if taken(d, db, name, nil) != [], do: raise("There's an item named #{name} already.")
      query(d, "insert into artifacts (id, name) values (?, ?)", [id, name])
    end)

    item = Map.merge(record, %{"id" => id, "name" => name, "changed" => now()})
    :ok = Artifact.write(root(db), "#{id}/item", JSON.encode!(item))
    item
  end

  @doc "Renames an item, once the name is free."
  def rename(call, db, item, to) do
    if String.trim(to) == "", do: raise("An item has a name.")

    transaction(call, db, fn d ->
      if taken(d, db, to, item["id"]) != [], do: raise("There's an item named #{to} already.")
      query(d, "update artifacts set name = ? where id = ?", [to, item["id"]])
    end)

    change!(db, item, &Map.put(&1, "name", to))
  end

  @doc "What items are called, by their artifacts, deleted ones too, from the database, which keeps each name."
  def names(call, db) do
    for %{"id" => id, "name" => name} <- transaction(call, db, &query(&1, "select id, name from artifacts")), do: {artifact(db, id), name}
  end

  @doc "A short-lived link to one blob of an item's, as what's handed on takes it: JSON's keys."
  def link(db, blob, mode), do: %{root(db) | codec: nil} |> Artifact.link(blob, mode) |> JSON.encode!() |> JSON.decode!()

  def random(bytes), do: Base.url_encode64(:crypto.strong_rand_bytes(bytes), padding: false)
  def now, do: System.os_time(:second)

  @doc false
  # Runs what handles secrets, so that nothing it raises says one: only what this app raised in its own words
  # is passed on, and anything else is said as what was being done. A call that another server ran ends as it does.
  def quietly(what, fun) do
    fun.()
  rescue
    error in RuntimeError -> reraise error, __STACKTRACE__
    error -> raise "#{what} didn't work (#{inspect(error.__struct__)})."
  end
end
