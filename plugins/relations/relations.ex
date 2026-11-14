defmodule Relations do
  @moduledoc """
  Relation tuples: who may do what in a network, each one line of an artifact, a relation and who. They're kept in
  this plugin's database, and it decides the `reach` hook from them, so a plugin reaches an artifact only where a
  tuple says both it and whoever it runs for may.

  Nothing here knows how anyone signs in. Who asks is `call.for`, as One Core says it.
  """
  @behaviour OneCore.Plugin

  # An owner is an editor, and an editor is a viewer: what each relation is enough for.
  @enough %{"viewer" => ~w(viewer editor owner), "editor" => ~w(editor owner owner), "owner" => ~w(owner owner owner)}

  # The network is the top parent of every artifact, and its first owner is the one user a network has before
  # anyone signs in.
  @schema [
    """
    create table tuples (
      artifact text not null, relation text not null, who text not null, primary key (artifact, relation, who)
    ) without rowid
    """,
    "create index tuples_by_who on tuples (who, relation)",
    "insert into tuples (artifact, relation, who) values ('network', 'owner', 'users/anyone')"
  ]

  # What's followed from one who or artifact to the next: the groups a who is a member of, and an artifact's
  # parents.
  @groups "select artifact as next from tuples where relation = 'member' and who = ?"
  @parents "select who as next from tuples where relation = 'parent' and artifact = ?"
  @tuple "select 1 from tuples where artifact = ? and who = ? and relation in (?, ?, ?) limit 1"

  @impl true
  def uses, do: ["database"]

  @impl true
  def guide do
    """
    Relations says who may do what, as relation tuples: an artifact, a relation, and who. With it in a network,
    a plugin reaches an artifact only if a tuple says so, both for whoever it runs for and for the plugin.
    Without it, everything in a network is shared.

    - An artifact is named by where its blobs are, and its name starts with whose it is:
      `users/alice/made/notes/7` is alice's, `groups/family/made/notes/2` is the group's. A plugin's tool that
      makes something answers with its name. A group is the artifact `groups/<name>`, a plugin
      `users/anyone/plugins/<name>`, a plugin's own artifact `users/anyone/own/<name>`, and the network `network`.
    - Who is a user (`users/alice`), a group, meaning its members (`groups/family`), `everyone` in the network, a
      plugin by its artifact's name, or a sharing link's holder (`links/...`).
    - `owner`, `editor` and `viewer` are what someone may do: an owner is an editor, and an editor is a viewer.
      Whoever an artifact's name starts with owns it, a group's members own what's the group's, and the network's
      owner owns everything. `member` puts a user or a group in a group. `parent` names another artifact as
      who, and gives whoever has a relation to that one the same relation to this.
    - `share` writes a tuple and `unshare` removes one. An owner or an editor shares with viewers and editors and
      adds a group's members, and only an owner adds an owner or a parent. `shared` lists an artifact's tuples,
      and `check` says whether someone may reach it.
    - `group` makes a group, with whoever asks as its owner and first member.
    - `link` makes a sharing link to an artifact, as a viewer or an editor. Whoever holds the link reaches the
      artifact without signing in, through the tools that take a link's holder, until its tuple is removed with
      `unshare`. The link is answered once and kept nowhere, so pass it on then.
    - A plugin reaches something made by another plugin only once it has a relation to it too:
      `share --artifact users/alice/made/notes/7 --relation viewer --who users/anyone/plugins/reader`.
    - Sharing, and taking it away, are a person's decisions. Do them when asked to, and say what you did.

    In a plugin, nothing of this is called: `OneCore.Call.reach(call, name, "editor")` gives the artifact or
    raises with the reason, and `OneCore.Call.reaches?/3` asks. A plugin whose own tools are for some users only
    asks for a relation to its own artifact, as Claims does for who may approve:
    `OneCore.Call.reaches?(call, OneCore.Artifact.name(call.own), "editor")`.
    """
  end

  @impl true
  def tools do
    artifact = %{type: "string", description: "The artifact's name, such as users/alice/made/notes/7"}

    who = %{
      type: "string",
      description: "Who: users/<name>, groups/<name>, everyone, a plugin's artifact's name, or links/<name>"
    }

    relation = %{type: "string", enum: ~w(viewer editor owner member parent), description: "The relation"}
    reaching = %{type: "string", enum: ~w(viewer editor owner), description: "The relation. viewer if left out"}
    tuple = %{type: "object", properties: %{artifact: artifact, relation: relation, who: who}}
    tuple = Map.put(tuple, :required, ["artifact", "relation", "who"])

    [
      %{name: "share", description: "Writes a relation tuple: who has which relation to an artifact", inputs: tuple},
      %{name: "unshare", description: "Removes a relation tuple", inputs: tuple},
      %{
        name: "shared",
        read_only: true,
        description: "Lists an artifact's relation tuples: who it's shared with, and as what",
        inputs: %{type: "object", properties: %{artifact: artifact}, required: ["artifact"]}
      },
      %{
        name: "check",
        read_only: true,
        description: "Says whether someone may reach an artifact",
        inputs: %{
          type: "object",
          properties: %{
            artifact: artifact,
            relation: reaching,
            who: Map.put(who, :description, "Who. You, if left out")
          },
          required: ["artifact"]
        }
      },
      %{
        name: "group",
        description: "Makes a group, with you as its owner and its first member",
        inputs: %{
          type: "object",
          properties: %{name: %{type: "string", description: "Its name: lowercase letters, digits and hyphens"}},
          required: ["name"]
        }
      },
      %{
        name: "link",
        description: "Makes a sharing link to an artifact. Whoever holds it reaches the artifact without signing in",
        inputs: %{
          type: "object",
          properties: %{
            artifact: artifact,
            relation: %{type: "string", enum: ~w(viewer editor), description: "What its holder is. viewer if left out"}
          },
          required: ["artifact"]
        }
      }
    ]
  end

  @impl true
  def hooks, do: ["reach"]

  @impl true
  def hook("reach", %{"who" => who, "relation" => relation, "artifact" => artifact}, call) do
    if transaction(call, &may?(&1, who, relation, artifact)),
      do: :ok,
      else: {:refuse, "#{who} isn't #{a(relation)} of #{artifact}. Whoever owns it shares it, with relations_share."}
  end

  @impl true
  def call("share", %{"artifact" => artifact, "relation" => relation, "who" => who}, call) do
    transaction(call, fn db ->
      shares!(db, call, artifact, relation)
      query(db, "insert or ignore into tuples (artifact, relation, who) values (?, ?, ?)", [artifact, relation, who])
      "#{who} is #{a(relation)} of #{artifact}."
    end)
  end

  def call("unshare", %{"artifact" => artifact, "relation" => relation, "who" => who}, call) do
    transaction(call, fn db ->
      shares!(db, call, artifact, relation)
      removed = "delete from tuples where artifact = ? and relation = ? and who = ? returning who"

      case query(db, removed, [artifact, relation, who]) do
        [_] -> "#{who} is no longer #{a(relation)} of #{artifact}."
        [] -> raise "No tuple says #{who} is #{a(relation)} of #{artifact}. `shared` lists the ones there are."
      end
    end)
  end

  def call("shared", %{"artifact" => artifact}, call) do
    transaction(call, fn db ->
      reaches!(db, call.for, "viewer", artifact)
      query(db, "select relation, who from tuples where artifact = ? order by relation, who", [artifact])
    end)
  end

  def call("check", %{"artifact" => artifact} = inputs, call) do
    who = Map.get(inputs, "who", call.for)
    relation = Map.get(inputs, "relation", "viewer")

    transaction(call, fn db ->
      if who != call.for, do: reaches!(db, call.for, "viewer", artifact)
      %{who: who, relation: relation, artifact: artifact, may: may?(db, who, relation, artifact)}
    end)
  end

  def call("group", %{"name" => name}, call) do
    if not (name =~ ~r/^[a-z][a-z0-9-]*$/), do: raise("A group's name is lowercase letters, digits and hyphens.")
    group = "groups/" <> name

    transaction(call, fn db ->
      if query(db, "select 1 from tuples where artifact = ? limit 1", [group]) != [],
        do: raise("There's a group named #{name} already.")

      for relation <- ["owner", "member"],
          do: query(db, "insert into tuples (artifact, relation, who) values (?, ?, ?)", [group, relation, call.for])

      group
    end)
  end

  # The link is random, and the tuple names it by its hash, as One Core names whoever shows it. So the tuples
  # don't hold the link, and nothing but the link gives it.
  def call("link", %{"artifact" => artifact} = inputs, call) do
    relation = Map.get(inputs, "relation", "viewer")
    link = Base.url_encode64(:crypto.strong_rand_bytes(24), padding: false)
    who = OneCore.User.link(link)

    transaction(call, fn db ->
      shares!(db, call, artifact, relation)
      query(db, "insert into tuples (artifact, relation, who) values (?, ?, ?)", [artifact, relation, who])
      %{link: link, who: who, relation: relation, artifact: artifact}
    end)
  end

  # Raises unless whoever asks may write or remove a tuple with this relation to the artifact: an owner or an
  # editor shares up to their own relation and says who's a member, and only an owner adds or removes an owner
  # or a parent.
  defp shares!(db, call, artifact, relation) do
    reaches!(db, call.for, if(relation in ["owner", "parent"], do: "owner", else: "editor"), artifact)
  end

  defp reaches!(db, who, relation, artifact) do
    if not may?(db, who, relation, artifact), do: raise("#{who} isn't #{a(relation)} of #{artifact}.")
  end

  # Whether `who` has the relation to the artifact. `whos` is everyone `who` counts as: themselves, everyone if
  # they're a user, and each group they're a member of, however far that goes. `up` is the artifact and its
  # parents, as far as those go, ending at the network. An artifact's name starts with whose it is, so whoever's
  # name it starts with owns it with no tuple. Each step is one small question of the database, which is many
  # times faster than asking it all of this at once.
  defp may?(db, who, relation, artifact) do
    whos = followed(db, @groups, if(String.starts_with?(who, "users/"), do: [who, "everyone"], else: [who]), [])
    enough = Map.fetch!(@enough, relation)

    Enum.any?(followed(db, @parents, [artifact, "network"], []), fn artifact ->
      Enum.any?(whos, &(String.starts_with?(artifact, &1 <> "/") or query(db, @tuple, [artifact, &1 | enough]) != []))
    end)
  end

  # Everything reached from `from` by following `next`, each once.
  defp followed(_db, _next, [], found), do: found

  defp followed(db, next, [one | rest], found) do
    if one in found,
      do: followed(db, next, rest, found),
      else: followed(db, next, rest ++ for(%{"next" => next} <- query(db, next, [one]), do: next), [one | found])
  end

  defp a("viewer"), do: "a viewer"
  defp a("member"), do: "a member"
  defp a("parent"), do: "a parent"
  defp a(relation), do: "an " <> relation

  # The database module goes with the database it opened, so one `query` serves every tool.
  defp transaction(call, fun) do
    database = OneCore.Call.plugin(call, "database")
    database.transaction(call, @schema, &fun.({database, &1}))
  end

  defp query({database, db}, sql, values), do: database.query(db, sql, values)
end
