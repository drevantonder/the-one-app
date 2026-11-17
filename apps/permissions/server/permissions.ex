defmodule Permissions do
  @moduledoc """
  Relation tuples: who may do what in a network, each one line of an artifact, a relation and who. They're kept in
  this app's database, and it decides the `reach` hook from them, so an app reaches an artifact only where a
  tuple says both it and whoever it runs for may.

  Nothing here knows how anyone signs in. Who asks is `call.subject`, as One Core says it.
  """
  @behaviour OneCore.App

  # One Core asks it who may reach what: without it, only the network's first owner reaches anything.
  @impl true
  def title, do: "Permissions"

  @impl true
  def icon, do: "icon.svg"

  # An owner is an editor, and an editor is a viewer: what each relation is enough for. An installer adds apps
  # for a user, a group or the network, which their owner does too, and is nothing more.
  @enough %{
    "viewer" => ~w(viewer editor owner),
    "editor" => ~w(editor owner owner),
    "owner" => ~w(owner owner owner),
    "installer" => ~w(installer owner owner)
  }

  @schema [
    """
    create table tuples (
      artifact text not null, relation text not null, who text not null, primary key (artifact, relation, who)
    ) without rowid
    """,
    "create index tuples_by_who on tuples (who, relation)",
    # What each group is called, as whoever made it wrote it. A group made before this is called what it's named.
    "create table names (artifact text primary key, called text not null) without rowid",
    "insert into names select distinct artifact, substr(artifact, 8) from tuples where artifact like 'groups/%'"
  ]

  # What's followed from one who or artifact to the next: the groups a who is a member of, and an artifact's
  # parents.
  @groups "select artifact as next from tuples where relation = 'member' and who = ?"
  @parents "select who as next from tuples where relation = 'parent' and artifact = ?"
  @members "select who as next from tuples where relation = 'member' and artifact = ?"
  @having "select who from tuples where artifact = ? and relation = ? order by who"
  @tuple "select 1 from tuples where artifact = ? and who = ? and relation in (?, ?, ?) limit 1"

  @impl true
  def requires, do: %{"core" => "~> 0.19.0", "database" => "to keep who may reach what"}

  @impl true
  def tools do
    artifact = %{type: "string", description: "The artifact's name, such as users/alice/made/notes/7"}

    who = %{
      type: "string",
      description: "Who: users/<name>, groups/<name>, everyone, an app's artifact's name, or a link's holder"
    }

    relation = %{type: "string", enum: ~w(viewer editor owner installer member parent), description: "The relation"}
    reaching = %{type: "string", enum: ~w(viewer editor owner), description: "The relation. viewer if left out"}
    tuple = %{type: "object", properties: %{artifact: artifact, relation: relation, who: who}}
    tuple = Map.put(tuple, :required, ["artifact", "relation", "who"])

    [
      %{
        name: "share",
        description: "Writes a relation tuple: who has which relation to an artifact",
        inputs: tuple,
        outputs: tuple
      },
      %{name: "unshare", description: "Removes a relation tuple", inputs: tuple, outputs: tuple},
      %{
        name: "shared",
        read_only: true,
        description: "Lists an artifact's relation tuples: who it's shared with, and as what",
        inputs: %{type: "object", properties: %{artifact: artifact}, required: ["artifact"]},
        outputs: %{
          type: "object",
          properties: %{
            tuples: %{
              type: "array",
              items: %{type: "object", properties: %{relation: relation, who: who}, required: ["relation", "who"]}
            }
          },
          required: ["tuples"]
        }
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
        },
        outputs: %{
          type: "object",
          properties: %{
            who: who,
            relation: reaching,
            artifact: artifact,
            may: %{type: "boolean", description: "Whether they may"}
          },
          required: ["who", "relation", "artifact", "may"]
        }
      },
      %{
        name: "users",
        read_only: true,
        description:
          "Lists the users who have a relation to an artifact, through the groups they're in: who owns the network, " <>
            "or may install apps for it",
        inputs: %{
          type: "object",
          properties: %{artifact: artifact, relation: Map.put(relation, :enum, ~w(viewer editor owner installer))},
          required: ["artifact", "relation"]
        },
        outputs: %{type: "object", properties: %{users: %{type: "array", items: %{type: "string"}, description: "users/<name>, or everyone"}}, required: ["users"]}
      },
      %{
        name: "members",
        read_only: true,
        description: "Lists the users in a group, through the groups in it, or every user the network's tuples name",
        inputs: %{
          type: "object",
          properties: %{of: %{type: "string", description: "groups/<name>, or network"}},
          required: ["of"]
        },
        outputs: %{type: "object", properties: %{members: %{type: "array", items: %{type: "string"}, description: "users/<name>, or everyone"}}, required: ["members"]}
      },
      %{
        name: "group",
        description: "Makes a group, with you as its owner and its first member",
        inputs: %{
          type: "object",
          properties: %{name: %{type: "string", description: "What it's called, as people write it: The Smiths"}},
          required: ["name"]
        },
        outputs: %{
          type: "object",
          properties: %{
            group: %{type: "string", description: "groups/<name>"},
            called: %{type: "string", description: "What it's called, as people write it"}
          },
          required: ["group", "called"]
        }
      },
      %{
        name: "groups",
        read_only: true,
        description: "Lists the groups you're in, each with what it's called, its members and its owners",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            groups: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  group: %{type: "string", description: "groups/<name>"},
                  called: %{type: "string", description: "What it's called, as people write it"},
                  members: %{type: "array", items: %{type: "string"}},
                  owners: %{type: "array", items: %{type: "string"}}
                },
                required: ["group", "called", "members", "owners"]
              }
            }
          },
          required: ["groups"]
        }
      },
      %{
        name: "forget",
        description:
          "Removes every tuple naming someone removed from the network as subject: the groups they were in, and what " <>
            "was shared with them. Taking someone out calls it",
        inputs: %{
          type: "object",
          properties: %{who: %{type: "string", pattern: "^users/[a-z][a-z0-9-]*$", description: "Who: users/<name>"}},
          required: ["who"]
        },
        outputs: %{
          type: "object",
          properties: %{who: %{type: "string"}, tuples: %{type: "integer", description: "How many tuples named them"}},
          required: ["who", "tuples"]
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
        },
        outputs: %{
          type: "object",
          properties: %{
            link: %{type: "string", description: "The link, which its holder shows: keep it as a secret"},
            who: %{type: "string", description: "Its holder, as tuples name them"},
            relation: %{type: "string", enum: ~w(viewer editor)},
            artifact: artifact
          },
          required: ["link", "who", "relation", "artifact"]
        }
      }
    ]
  end

  @impl true
  def hooks, do: ["reach", "owner"]

  # The network is the top parent of every artifact, and its first owner's tuple is the one nobody shares: it's
  # written as One Core makes them.
  @impl true
  def hook("owner", %{"who" => who}, call) do
    transaction(call, fn db ->
      query(db, "insert or ignore into tuples (artifact, relation, who) values ('network', 'owner', ?)", [who])
    end)

    :ok
  end

  def hook("reach", %{"who" => who, "relation" => "member", "artifact" => of}, call) do
    if transaction(call, &member?(&1, who, of)),
      do: :ok,
      else: {:refuse, "#{who} isn't a member of #{of}. Whoever owns it adds them, with permissions_share."}
  end

  def hook("reach", %{"who" => who, "relation" => "installer", "artifact" => artifact}, call) do
    if transaction(call, &may?(&1, who, "installer", artifact)),
      do: :ok,
      else:
        {:refuse,
         "#{who} can't install apps for #{artifact}. Whoever can installs it for them, or its owner lets them, " <>
           "with permissions_share --relation installer."}
  end

  def hook("reach", %{"who" => who, "relation" => relation, "artifact" => artifact}, call) do
    if transaction(call, &may?(&1, who, relation, artifact)),
      do: :ok,
      else: {:refuse, "#{who} isn't #{a(relation)} of #{artifact}. Whoever owns it shares it, with permissions_share."}
  end

  @impl true
  def call("share", %{"artifact" => artifact, "relation" => relation, "who" => who}, call) do
    transaction(call, fn db ->
      shares!(db, call, artifact, relation)
      query(db, "insert or ignore into tuples (artifact, relation, who) values (?, ?, ?)", [artifact, relation, who])
      %{artifact: artifact, relation: relation, who: who}
    end)
  end

  def call("unshare", %{"artifact" => artifact, "relation" => relation, "who" => who}, call) do
    transaction(call, fn db ->
      shares!(db, call, artifact, relation)
      removed = "delete from tuples where artifact = ? and relation = ? and who = ? returning who"

      case query(db, removed, [artifact, relation, who]) do
        [_] -> %{artifact: artifact, relation: relation, who: who}
        [] -> raise "No tuple says #{who} is #{a(relation)} of #{artifact}. `shared` lists the ones there are."
      end
    end)
  end

  def call("shared", %{"artifact" => artifact}, call) do
    transaction(call, fn db ->
      reaches!(db, call.subject, "viewer", artifact)
      %{tuples: query(db, "select relation, who from tuples where artifact = ? order by relation, who", [artifact])}
    end)
  end

  def call("check", %{"artifact" => artifact} = inputs, call) do
    who = Map.get(inputs, "who", call.subject)
    relation = Map.get(inputs, "relation", "viewer")

    transaction(call, fn db ->
      if who != call.subject, do: reaches!(db, call.subject, "viewer", artifact)
      %{who: who, relation: relation, artifact: artifact, may: may?(db, who, relation, artifact)}
    end)
  end

  # Whoever may see an artifact, or is in it, sees who has which relation to it: everyone in the network sees who
  # owns it and installs its apps. A tuple's who is a user, a group, whose members are followed, or everyone,
  # which is said as it is.
  def call("users", %{"artifact" => artifact, "relation" => relation}, call) do
    transaction(call, fn db ->
      if not (member?(db, call.subject, artifact) or may?(db, call.subject, "viewer", artifact)),
        do: raise("#{call.subject} isn't in #{artifact}, nor sees it, so doesn't see who has it.")

      up = followed(db, @parents, Enum.uniq([artifact, OneCore.User.owner(artifact) || artifact, "network"]), [])
      tuples = "select who as next from tuples where artifact = ? and relation in (?, ?, ?)"
      named = for at <- up, %{"next" => who} <- query(db, tuples, [at | Map.fetch!(@enough, relation)]), do: who
      %{users: Enum.sort(Enum.filter(followed(db, @members, named, []), &(user?(&1) or &1 == "everyone")))}
    end)
  end

  # Every user is in the network, so its members are every user a tuple names, as who or as whose an artifact is.
  def call("members", %{"of" => "network"}, call) do
    if not user?(call.subject), do: raise("Only a user lists the network's users.")

    transaction(call, fn db ->
      named = query(db, "select distinct who as name from tuples union select distinct artifact from tuples", [])

      members = for %{"name" => name} <- named, owner <- [OneCore.User.owner(name)], owner, user?(owner), do: owner
      %{members: Enum.sort(Enum.uniq(members))}
    end)
  end

  def call("members", %{"of" => "groups/" <> _ = group}, call) do
    transaction(call, fn db ->
      if not (member?(db, call.subject, group) or may?(db, call.subject, "viewer", group)),
        do: raise("#{call.subject} isn't in #{group}, so doesn't see who is.")

      %{members: Enum.sort(Enum.filter(followed(db, @members, [group], []), &user?/1))}
    end)
  end

  def call("members", %{"of" => of}, _call), do: raise("#{of} isn't a group or the network.")

  # Someone out of the network is in nothing, so their name is free for someone new to join by and inherits none
  # of it. Who's in the network is the app that signs people in's to say, so it's asked: it took them out first.
  def call("forget", %{"who" => who}, call) do
    if Enum.any?(OneCore.Call.tool(call, "sign-in", "everyone", %{})["people"], &(&1["who"] == who)),
      do: raise("#{who} is in the network. Taking them out with sign-in_remove_person forgets them here too.")

    transaction(call, fn db ->
      forgotten = query(db, "delete from tuples where who = ? returning artifact", [who])
      %{who: who, tuples: length(forgotten)}
    end)
  end

  # A group is named after what it's called, in lowercase letters, digits and hyphens: The Smiths is
  # groups/the-smiths, and Wiśniewski groups/wisniewski.
  def call("group", %{"name" => called}, call) do
    called = String.trim(called)
    plain = called |> String.normalize(:nfd) |> String.replace(~r/\p{Mn}/u, "") |> String.downcase()
    name = plain |> String.replace(~r/[^a-z0-9]+/, "-") |> String.trim("-")
    if not (name =~ ~r/^[a-z]/), do: raise("A group's name starts with a letter.")
    group = "groups/" <> name

    transaction(call, fn db ->
      if query(db, "select 1 from tuples where artifact = ? limit 1", [group]) != [],
        do: raise("There's a group called #{called} already.")

      for relation <- ["owner", "member"],
          do: query(db, "insert into tuples (artifact, relation, who) values (?, ?, ?)", [group, relation, call.subject])

      query(db, "insert into names (artifact, called) values (?, ?)", [group, called])
      %{group: group, called: called}
    end)
  end

  def call("groups", _inputs, call) do
    transaction(call, fn db ->
      groups =
        for "groups/" <> _ = group <- Enum.sort(followed(db, @groups, [call.subject], []) -- [call.subject]) do
        having = &for %{"who" => who} <- query(db, @having, [group, &1]), do: who
        # A group that sharing with it made, and not `group`, is called what it's named.
        called =
          with [%{"called" => called}] <- query(db, "select called from names where artifact = ?", [group]),
               do: called,
               else: (_ -> String.replace_prefix(group, "groups/", ""))

          %{group: group, called: called, members: having.("member"), owners: having.("owner")}
        end

      %{groups: groups}
    end)
  end

  # The link is random, after whose the artifact is, and the tuple names it by its hash, as One Core names whoever
  # shows it. So the tuples don't hold the link, and nothing but the link gives it.
  def call("link", %{"artifact" => artifact} = inputs, call) do
    relation = Map.get(inputs, "relation", "viewer")
    owner = OneCore.User.owner(artifact) || raise "#{artifact} has no owner, so no link leads to it."
    link = owner <> "/" <> Base.url_encode64(:crypto.strong_rand_bytes(24), padding: false)
    who = OneCore.User.link(link)

    transaction(call, fn db ->
      shares!(db, call, artifact, relation)
      query(db, "insert into tuples (artifact, relation, who) values (?, ?, ?)", [artifact, relation, who])
      %{link: link, who: who, relation: relation, artifact: artifact}
    end)
  end

  # Raises unless whoever asks may write or remove a tuple with this relation to the artifact: an owner or an
  # editor shares up to their own relation and says who's a member, and only an owner adds or removes an owner
  # or a parent, or says who installs apps.
  defp shares!(db, call, artifact, relation) do
    reaches!(db, call.subject, if(relation in ["owner", "installer", "parent"], do: "owner", else: "editor"), artifact)
  end

  defp reaches!(db, who, relation, artifact) do
    if not may?(db, who, relation, artifact), do: raise("#{who} isn't #{a(relation)} of #{artifact}.")
  end

  # Whether `who` has the relation to the artifact. `whos` is everyone `who` counts as: themselves, everyone if
  # they're a user, and each group they're a member of, however far that goes. `up` is the artifact, whose it is, and
  # their parents, as far as those go, ending at the network. An artifact's name starts with whose it is, so a user
  # owns what their name starts, and a group's members what was made for the group, with no tuple. Each step is one
  # small question of the database, which is many times faster than asking it all of this at once.
  defp may?(db, who, relation, artifact) do
    whos = followed(db, @groups, if(user?(who), do: [who, "everyone"], else: [who]), [])
    enough = Map.fetch!(@enough, relation)

    up = Enum.uniq([artifact, OneCore.User.owner(artifact) || artifact, "network"])

    Enum.any?(followed(db, @parents, up, []), fn artifact ->
      Enum.any?(whos, &(String.starts_with?(artifact, theirs(&1)) or query(db, @tuple, [artifact, &1 | enough]) != []))
    end)
  end

  # How the names start of what's a who's own with no tuple saying so.
  defp theirs("groups/" <> _ = group), do: group <> "/made/"
  defp theirs(who), do: who <> "/"

  # Whether `who` is in a group, themselves or through a group that is, or is a user asking about the network,
  # which every user is in.
  defp member?(db, who, of) do
    (of == "network" and user?(who)) or of in (followed(db, @groups, [who], []) -- [who])
  end

  # A user is in the network, as the holder of a link that leads to what's theirs isn't.
  defp user?(who), do: String.starts_with?(who, "users/") and not OneCore.User.link?(who)

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
    database = OneCore.Call.app(call, "database")
    database.transaction(call, @schema, &fun.({database, &1}))
  end

  defp query({database, db}, sql, values), do: database.query(db, sql, values)
end
