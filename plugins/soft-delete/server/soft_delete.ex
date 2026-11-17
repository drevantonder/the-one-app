defmodule SoftDelete do
  @moduledoc """
  Soft delete, which people see as Recently Deleted: what someone deletes waits for 30 days, so it can be restored as it
  was, before it's purged for good. Anything a plugin made for someone (`OneCore.Call.make/2`) is deleted with this
  plugin's `delete`, which a plugin calls for its person as Git Repos does for a repo, and which an agent calls too. So
  is a plugin someone deployed (`<owner>/plugins/<name>`), once nobody has it installed. A plugin uninstalled for
  someone (One Core's hook `uninstalled`) takes what it kept and made for them along, as one item.

  Each item is a record in this plugin's own artifact, `entries/<id>`, and it moves through states by the transition
  table `move/2`:

      active ─delete─► deleted ─30 days, or Delete Now─► purging ─► purged
      deleted ─Restore─► active, at its name, or beside it as notes-2 where something new is there since
      purging ─fails─► deleted, tried again the next day

  Active and purged are no record at all: a record says so only while the work of getting there is under way, and
  goes once it's done. An item lives for a month and any server may move it, while no process lives that long, so
  it's a table over records, each move made with `OneCore.Artifact.update/3`: two servers moving one item at once each
  see the other's move first, and a move the table doesn't have is refused with why.

  Deleting an artifact takes its blobs aside, byte for byte as they are in the bucket, to an artifact this plugin makes
  for its owner, `<owner>/made/soft-delete/<id>/<n>/`, one folder for each artifact of the item, and leaves nothing at
  its name. So it's gone for every plugin, whatever reads it, and stays under its owner's prefix, where the same
  servers reach it. An encrypted artifact's blobs go aside still encrypted, keys and all, and a database's log,
  snapshots and lease go together, so each opens again as it was once it's back. The record is written before
  anything moves and says when everything has (`moved`), so a move cut short is finished by the next one to touch the
  item, or by the clock.

  Who deletes, restores and purges an item is whoever owns it, as the plugins that validate `reach` say. Every move is
  told to every plugin that hooks into the hook this plugin offers, `soft-delete.moved`, so one that keeps a list of
  what it made keeps it true. It's collected, so each move waits for their answers. Purging puts each artifact back at
  its name first, and tells them it's `purging` before anything is erased, so the plugin that made it reads it once
  more, as one does to revoke a token it holds. One that fails or doesn't answer holds the purge back a day, once: the
  next day it's purged whatever they answer, so no plugin keeps an item for ever.
  """
  @behaviour OneCore.Plugin

  # Any plugin deletes what it made through it, by its tool, without naming it among what it uses.
  @impl true
  def icon, do: "client/icon.svg"

  @impl true
  def essential, do: true

  require Logger
  alias OneCore.{Artifact, Bucket, Call, Hooks, Plugins, Servers, User}
  alias OneCore.Plugins.Lifecycle

  # How long a deleted item can be restored, after which it's purged.
  @retention :timer.hours(24 * 30)
  # How long after a purge failed it's tried again.
  @retry :timer.hours(24)
  # How long work on an item may take before it's taken as cut short, as by its server stopping.
  @stale :timer.hours(1)
  @made ~r{^(users/[^/]+|groups/[^/]+|network)/made/([^/]+)/([A-Za-z0-9_-]+)$}
  @plugin ~r{^(users/[^/]+|groups/[^/]+|network)/plugins/([a-z][a-z0-9-]*)$}
  @own ~r{^(users/[^/]+|groups/[^/]+|network)/own/([a-z][a-z0-9-]*)$}

  @impl true
  def uses, do: ["database"]

  @impl true
  def hooks, do: ["uninstalled", "restored"]

  @impl true
  def apps,
    do: [
      %{
        name: "Recently Deleted",
        page: "client/index.html",
        category: "admin",
        subject: "user"
      }
    ]

  @impl true
  def offers do
    [
      %{
        name: "moved",
        kind: :collect,
        description:
          "An artifact moved from one state of soft delete to another. It's about the `artifact`'s name and the state it " <>
            "moved `to`: `deleted`, `purging`, `active` (restored) or `purged`. One restored beside something new at its " <>
            "name says where, `as`. On `purging` it's at its name again, nothing of it erased yet, and `readable` says so " <>
            "(false only where something new was made at its name since): the plugin that made it reads it, as one does " <>
            "to revoke a token it holds. A plugin that keeps a list of what it made keeps it true from it. Each move " <>
            "waits for the answers, and a plugin that fails or doesn't answer holds a purge back a day, once."
      }
    ]
  end

  @impl true
  def jobs, do: [%{tool: "expire", every: 86_400}]

  @impl true
  def tools do
    artifact = %{
      type: "string",
      description: "An artifact, by its name: users/alice/made/git-repos/notes, or a plugin: users/alice/plugins/notes"
    }

    item = %{type: "string", description: "A deleted item, by the id `list` gives it"}
    only = fn properties, required -> %{type: "object", properties: properties, required: required} end

    restored =
      only.(
        %{
          artifact: artifact,
          as: %{type: "string", description: "Where it is now: its name, or beside it, as notes-2"}
        },
        ["artifact", "as"]
      )

    [
      %{
        name: "delete",
        description:
          "Deletes something you own that an app made, such as a repo, or a plugin you deployed that nobody has installed: it's gone from everywhere, until `restore` brings it back as it was. After 30 days it's purged for good",
        inputs: only.(%{artifact: artifact}, ["artifact"]),
        outputs: only.(%{item: item, artifact: artifact}, ["item", "artifact"])
      },
      %{
        name: "list",
        read_only: true,
        description:
          "What you own that was deleted in the last 30 days, the latest first: each item's artifacts, who deleted it and when, and when it's purged",
        inputs: only.(%{}, []),
        outputs:
          only.(
            %{
              deleted: %{
                type: "array",
                items:
                  only.(
                    %{
                      item: item,
                      state: %{type: "string", enum: ["deleted", "purging"]},
                      artifacts: %{type: "array", items: artifact, description: "What it holds"},
                      plugin: %{
                        type: ["string", "null"],
                        description: "The plugin whose uninstalling deleted it, by the name it was installed by"
                      },
                      app: %{type: "string", description: "The name of the plugin its artifacts are of, as people read it"},
                      installed: %{
                        type: "boolean",
                        description: "Whether that plugin is installed for its owner, to open it once it's restored"
                      },
                      owner: %{type: "string", description: "Whose it is: users/<name>, groups/<name> or network"},
                      by: %{type: "string", description: "Who deleted it"},
                      at: %{type: "integer", description: "When, in milliseconds since 1970"},
                      purges: %{type: "integer", description: "When it's purged, in milliseconds since 1970"},
                      why: %{
                        type: ["string", "null"],
                        description: "Why purging it last failed; it's tried again the next day"
                      }
                    },
                    ["item", "state", "artifacts", "plugin", "app", "installed", "owner", "by", "at", "purges", "why"]
                  )
              }
            },
            ["deleted"]
          )
      },
      %{
        name: "restore",
        description:
          "Restores a deleted item as it was: each artifact at its name, or beside it as notes-2 where something new is there since. An uninstalled plugin is installed again with what it kept. One whose plugin isn't installed waits for it",
        inputs: only.(%{item: item}, ["item"]),
        outputs:
          only.(
            %{
              restored: %{type: "array", items: restored},
              installed: %{type: "boolean", description: "Whether the plugin its artifacts are of is installed, to open them"},
              app: %{type: "string", description: "That plugin's name, as people read it"}
            },
            ["restored", "installed", "app"]
          )
      },
      %{
        name: "purge",
        description: "Deletes a deleted item for good now, rather than after 30 days. It can't be restored after",
        inputs: only.(%{item: item}, ["item"]),
        outputs: only.(%{purged: %{type: "array", items: artifact, description: "What was purged"}}, ["purged"])
      },
      %{
        name: "purge_all",
        description: "Deletes everything you own in Recently Deleted for good now. None of it can be restored after",
        inputs: only.(%{}, []),
        outputs: only.(%{purged: %{type: "array", items: artifact, description: "What was purged"}}, ["purged"])
      },
      %{
        name: "expire",
        description:
          "Purges what was deleted more than 30 days ago, and finishes any move that was cut short. The clock calls it each day",
        inputs: only.(%{}, []),
        outputs: only.(%{purged: %{type: "array", items: artifact, description: "What was purged"}}, ["purged"])
      }
    ]
  end

  @doc """
  The transition table: where an item's record goes on `event`, from where it is (`nil` for none: it's active). Gives
  `{:ok, record}` or `{:error, why}` for a move the table doesn't have.

  Events: `{:delete, record}`, `:moved` (everything of it is aside), `{:restore, as}` (each artifact's name and where it
  goes back to), `:purge` (30 days on, or Delete Now), `{:back, names}` (the artifacts put back at their names while
  their plugins are told), `:erasing`, `{:fail, why, how}` (purging it failed: `:partly` once some of it was erased,
  `:unheard` where a plugin failed its hook, so the next purge goes on whatever they answer, or `:whole`) and
  `:purged`.
  """
  def move(record, event)
  def move(nil, {:delete, record}), do: {:ok, Map.merge(record, %{"state" => "deleted", "moved" => false})}
  def move(%{"state" => "deleted"} = record, :moved), do: {:ok, %{record | "moved" => true}}

  def move(%{"state" => "deleted", "partly" => true}, {:restore, _as}),
    do: {:error, "Part of it was deleted for good already, so it can't come back as it was. It's purged tomorrow."}

  def move(%{"state" => "deleted"} = record, {:restore, as}),
    do: {:ok, record |> Map.put("state", "active") |> Map.put("as", as) |> Map.put("since", now())}

  def move(%{"state" => "deleted"} = record, :purge),
    do: {:ok, record |> Map.put("state", "purging") |> Map.put("since", now())}

  def move(%{"state" => "purging"} = record, {:back, names}), do: {:ok, Map.put(record, "back", names)}
  def move(%{"state" => "purging"} = record, :erasing), do: {:ok, Map.put(record, "erasing", true)}

  def move(%{"state" => "purging"} = record, {:fail, why, how}) when how in [:partly, :unheard, :whole],
    do:
      {:ok,
       record
       |> Map.drop(["back", "erasing"])
       |> Map.merge(%{"state" => "deleted", "why" => why, "retry" => now() + @retry})
       |> Map.update("partly", how == :partly, &(&1 or how == :partly))
       |> Map.update("unheard", how == :unheard, &(&1 or how == :unheard))}

  def move(%{"state" => "purging"} = record, :purged), do: {:ok, %{record | "state" => "purged"}}
  def move(nil, _event), do: {:error, "That isn't in Recently Deleted. `soft-delete list` lists what is."}
  def move(%{"state" => "purging"}, _event), do: {:error, "It's being deleted for good right now."}
  def move(%{"state" => "active"}, _event), do: {:error, "It was restored already."}
  def move(%{"state" => "purged"}, _event), do: {:error, "It was deleted for good already."}
  def move(%{"state" => state}, event), do: {:error, "An item that's #{state} can't be moved by #{inspect(event)}."}

  @impl true
  def hook("uninstalled", %{"for" => owner, "name" => name, "plugin" => plugin}, call) do
    # It's heard where the plugin was uninstalled, and its items are kept by a server acting for the network.
    if not Servers.here?(call.installed), do: Call.forward(call, call.installed)

    made =
      for blob <- Bucket.list("#{owner}/made/#{name}/"),
          [artifact] <- [Regex.run(~r{^[^/]+(?:/[^/]+)?/made/[^/]+/[^/]+}, blob)],
          uniq: true,
          do: artifact

    own = "#{owner}/own/#{name}"
    artifacts = made ++ if(Bucket.list(own <> "/") != [], do: [own], else: [])
    record = %{
      "owner" => owner,
      "artifacts" => artifacts,
      "plugin" => %{"name" => name, "artifact" => plugin},
      "app" => title(call, Path.basename(plugin)),
      "by" => call.actor || owner,
      "at" => now()
    }

    id = id()
    {:ok, _record} = change(call, id, {:delete, record})
    # Moving a plugin's data can take longer than an observer is given, and it's finished later if it's cut short.
    Task.start(fn -> aside(call, id) end)
    :ok
  end

  # A plugin restored from Recently Deleted another way (`plugins_restore`) brings back what it kept and made too.
  def hook("restored", %{"for" => owner, "name" => name}, call) do
    if not Servers.here?(call.installed), do: Call.forward(call, call.installed)

    for {id, %{"state" => "deleted", "plugin" => %{"name" => ^name}, "owner" => ^owner} = record} <- records(call),
        do: restored(call, id, record)

    :ok
  end

  @impl true
  def call("delete", %{"artifact" => name}, call) do
    owns!(call, name)
    unused!(name)
    if Bucket.list(name <> "/") == [], do: raise("There's nothing at #{name}.\n\n`soft-delete list` lists what's deleted.")
    owner = User.owner(name)

    record = %{
      "owner" => owner,
      "artifacts" => [name],
      "app" => title(call, app_of(name)),
      "by" => call.actor || call.subject,
      "at" => now()
    }

    id = id()
    {:ok, _record} = change(call, id, {:delete, record})
    aside(call, id)
    %{item: id, artifact: name}
  end

  def call("list", _inputs, call) do
    deleted =
      for {id, %{"state" => state} = record} <- records(call), state in ["deleted", "purging"], owns_item?(call, record) do
        %{
          item: id,
          state: state,
          artifacts: record["artifacts"],
          plugin: record["plugin"]["name"],
          app: record["app"],
          installed: installed?(record),
          owner: record["owner"],
          by: record["by"],
          at: record["at"],
          purges: record["at"] + @retention,
          why: record["why"]
        }
      end

    %{deleted: Enum.sort_by(deleted, & &1.at, :desc)}
  end

  def call("restore", %{"item" => id}, call) do
    record = record!(call, id)

    if plugin = record["plugin"] do
      if Plugins.plugin(record["owner"], plugin["name"]) != nil or
           Bucket.list("#{record["owner"]}/own/#{plugin["name"]}/") != [],
         do:
           raise(
             "#{record["app"]} was installed again since it was deleted. Uninstall it first to restore the one that was."
           )
    end

    record = restored(call, id, record)

    # An uninstalled plugin comes back with it.
    with %{"name" => name} <- record["plugin"],
         {:error, why} <- Lifecycle.restore(record["owner"], name, {call.actor, call.subject}),
         do: Logger.warning("#{record["app"]} wasn't installed again for #{record["owner"]}: #{why}")

    %{
      restored: for(name <- record["artifacts"], do: %{artifact: name, as: record["as"][name]}),
      installed: installed?(record),
      app: record["app"]
    }
  end

  # Restores an item: each artifact at its name, or beside something new there since. Gives its record as it was moved.
  defp restored(call, id, record) do
    # Something new at an artifact's name since is left as it is, and this one goes beside it.
    as =
      Map.new(record["artifacts"], fn name ->
        {name, if(record["moved"] and taken?(name), do: free(name), else: name)}
      end)

    with {:error, why} <- change(call, id, {:restore, as}), do: raise(why)
    back(call, id)
  end

  def call("purge", %{"item" => id}, call) do
    record!(call, id)
    %{purged: purge!(call, id)}
  end

  def call("purge_all", _inputs, call) do
    results = for {id, %{"state" => "deleted"} = record} <- records(call), owns_item?(call, record), do: purge(call, id)
    failed = for {:error, _why} <- results, do: :failed

    if failed != [],
      do: raise("Some couldn't be deleted for good yet. They're tried again tomorrow.")

    %{purged: List.flatten(results)}
  end

  def call("expire", _inputs, call) do
    stale = now() - @stale

    purged =
      for {id, record} <- records(call), name <- expired(call, id, record, stale) do
        name
      end

    %{purged: purged}
  end

  # What the clock does with each item: purges one that's due, and finishes any work on one that was cut short.
  defp expired(call, id, record, stale) do
    case record do
      %{"state" => "active"} ->
        back(call, id)
        []

      %{"state" => "purged"} ->
        gone(call, id, record)
        []

      %{"state" => "purging", "since" => since} when since < stale ->
        put_aside(record, id)
        change(call, id, {:fail, "It was cut short.", if(record["erasing"], do: :partly, else: :whole)})
        []

      %{"state" => "deleted", "moved" => false, "at" => at} when at < stale ->
        aside(call, id)
        []

      %{"state" => "deleted", "at" => at} = record ->
        if at + @retention <= now() and Map.get(record, "retry", 0) <= now(), do: purge(call, id), else: []

      _ ->
        []
    end
  rescue
    error ->
      Logger.error("soft-delete couldn't expire #{id}: #{Exception.message(error)}")
      []
  end

  # Purges an item, or raises why it couldn't.
  defp purge!(call, id) do
    case purge(call, id) do
      {:error, why} -> raise why
      purged -> purged
    end
  end

  # Moves it to purging, puts each artifact back at its name and tells the plugins that hook `moved`, so the one that
  # made it reads it once more, then erases it, and then it's purged: its record goes. One a plugin failed to answer for
  # goes back to deleted the first time, and is purged the next day whatever they answer. One that fails part way goes
  # back to deleted, saying why, and is tried again the next day.
  defp purge(call, id) do
    with {:ok, record} <- change(call, id, :purge) do
      if not record["moved"], do: moved_aside(record, id)
      # Something new made at an artifact's name since keeps it: that one stays aside.
      {:ok, record} = change(call, id, {:back, for(name <- record["artifacts"], not taken?(name), do: name)})

      for {name, n} <- Enum.with_index(record["artifacts"]), name in record["back"],
          do: moved(Bucket.list(aside(record, id, n)), &(name <> "/" <> String.replace_prefix(&1, aside(record, id, n), "")))

      unheard = notify(call, record, "purging")

      if unheard != %{} and !record["unheard"] do
        put_aside(record, id)
        why = Enum.map_join(unheard, "\n", fn {plugin, why} -> "#{plugin}: #{why}" end)
        failed(call, id, "A plugin didn't answer as it was purged.\n#{why}", :unheard)
      else
        {:ok, record} = change(call, id, :erasing)
        erase(call, id, record)
      end
    end
  rescue
    error ->
      record = records(call)[id]

      if record["state"] == "purging" do
        try do
          put_aside(record, id)
        rescue
          again -> Logger.error("soft-delete couldn't put #{id} aside again: #{Exception.message(again)}")
        end

        failed(call, id, Exception.message(error), if(record["erasing"], do: :partly, else: :whole))
      else
        {:error, Exception.message(error)}
      end
  end

  # Erases each artifact of an item, where it is: at its name, put back, or aside.
  defp erase(call, id, record) do
    blobs =
      Enum.flat_map(Enum.with_index(record["artifacts"]), fn {name, n} ->
        Bucket.list(if(name in record["back"], do: name <> "/", else: aside(record, id, n)))
      end)

    erased =
      Enum.reduce_while(blobs, 0, fn blob, erased ->
        case Bucket.remove(blob) do
          :ok -> {:cont, erased + 1}
          other -> {:halt, {erased, "#{blob} wasn't removed: #{inspect(other)}"}}
        end
      end)

    case erased do
      {_erased, why} ->
        put_aside(record, id)
        failed(call, id, why, :partly)

      _ ->
        for name <- record["artifacts"], name =~ @plugin, do: Bucket.remove(listing(name))
        # An uninstalled plugin goes for good with it, unless it went already.
        with %{"name" => name} <- record["plugin"], do: Lifecycle.purge(record["owner"], name)
        {:ok, record} = change(call, id, :purged)
        gone(call, id, record)
        record["artifacts"]
    end
  end

  # What of an item was put back at its names to be purged goes aside again, as it is while it's deleted.
  defp put_aside(record, id) do
    for {name, n} <- Enum.with_index(record["artifacts"]), name in (record["back"] || []),
        do: moved(Bucket.list(name <> "/"), &(aside(record, id, n) <> String.replace_prefix(&1, name <> "/", "")))
  end

  defp failed(call, id, why, how) do
    with {:ok, record} <- change(call, id, {:fail, why, how}), do: notify(call, record, "deleted")
    {:error, "It couldn't be deleted for good yet. It's tried again tomorrow.\n\n#{why}"}
  end

  # A purged item's record goes, and whoever keeps a list hears it.
  defp gone(call, id, record) do
    Artifact.remove(call.own, "entries/" <> id)
    notify(call, record, "purged")
  end

  # Takes each artifact of an item aside, and says so in its record once it's all there, then tells whoever keeps a list.
  defp aside(call, id) do
    record = records(call)[id]
    moved_aside(record, id)
    {:ok, record} = change(call, id, :moved)
    notify(call, record, "deleted")
    record
  end

  defp moved_aside(record, id) do
    for {name, n} <- Enum.with_index(record["artifacts"]),
        do: moved(Bucket.list(name <> "/"), &(aside(record, id, n) <> String.replace_prefix(&1, name <> "/", "")))
  end

  # Puts what's aside back where its record says, and then it's active: its record goes.
  defp back(call, id) do
    record = records(call)[id]

    for {name, n} <- Enum.with_index(record["artifacts"]) do
      from = aside(record, id, n)
      moved(Bucket.list(from), &(record["as"][name] <> "/" <> String.replace_prefix(&1, from, "")))
    end

    relisted(record["artifacts"])
    Artifact.remove(call.own, "entries/" <> id)
    notify(call, record, "active")
    record
  end

  # Moves an item's record by `event`, as `move/2` says: `{:ok, record}`, or `{:error, why}` with it as it was.
  defp change(call, id, event) do
    written =
      Artifact.update(call.own, "entries/" <> id, fn was ->
        case move(was && JSON.decode!(was), event) do
          {:ok, record} -> JSON.encode!(record)
          {:error, why} -> throw({__MODULE__, why})
        end
      end)

    {:ok, JSON.decode!(written)}
  catch
    {__MODULE__, why} -> {:error, why}
  end

  # Each blob, as it is in the bucket, to where `to` says, and then removed from where it was. One that's there already
  # is left if it's the same, as when a move cut short is finished.
  defp moved(blobs, to) do
    copied =
      Task.async_stream(
        blobs,
        fn blob ->
          with {:ok, data, _tag} <- Bucket.read(blob),
               :exists <- Bucket.write(to.(blob), data),
               {:ok, other, _tag} when other != data <- Bucket.read(to.(blob)),
               do: "#{to.(blob)} is there already, and isn't the same",
               else: (_ -> nil)
        end,
        max_concurrency: 16,
        timeout: :infinity
      )

    with [why | _] <- for({:ok, why} when is_binary(why) <- copied, do: why), do: raise(why)
    Task.async_stream(blobs, &Bucket.remove/1, max_concurrency: 16, timeout: :infinity) |> Stream.run()
  end

  # Whether something is at a name now.
  defp taken?(name), do: Bucket.list(name <> "/") != []

  # The first name beside it that's free, as Finder names a copy: notes-2, then notes-3, and notes-3 beside notes-2.
  defp free(name) do
    {base, from} =
      case Regex.run(~r/^(.*)-(\d+)$/, name) do
        [_, base, n] -> {base, String.to_integer(n) + 1}
        nil -> {name, 2}
      end

    Enum.find_value(Stream.iterate(from, &(&1 + 1)), &if(not taken?("#{base}-#{&1}"), do: "#{base}-#{&1}"))
  end

  # A plugin's listing in the App Store, which shows it only while it has a latest version: so it's kept while it's
  # deleted, for `restore`, and goes when it's purged.
  defp listing(plugin), do: "network/deployed/#{User.owner(plugin)}/#{Path.basename(plugin)}"

  # Servers look again at what's deployed, so a plugin deleted or restored leaves the App Store or is back in it at once.
  defp relisted(names) do
    plugins = for name <- names, name =~ @plugin, do: User.owner(name)

    if plugins != [] do
      OneCore.Plugins.Deploy.forget()
      OneCore.Server.notify_servers(Enum.uniq(plugins))
    end
  end

  # Every record, by its item's id.
  defp records(call) do
    ids = for "entries/" <> id <- Artifact.list(call.own, "entries/"), do: id

    for {id, {:ok, record, _tag}} <- Enum.zip(ids, Artifact.read_all(call.own, Enum.map(ids, &("entries/" <> &1)))),
        into: %{},
        do: {id, JSON.decode!(record)}
  end

  # An item's record, if whoever the call runs for owns it.
  defp record!(call, id) do
    record = records(call)[id]

    if record == nil or not owns_item?(call, record),
      do: raise("That isn't in Recently Deleted.\n\n`soft-delete list` lists what is.")

    record
  end

  # Where an item's artifact `n` waits while it's deleted: an artifact of this plugin's, made for the same owner.
  defp aside(record, id, n), do: "#{record["owner"]}/made/soft-delete/#{id}/#{n}/"

  # The name of the plugin an artifact is of: the one that made it, or the plugin it is.
  defp app_of(name) do
    case Regex.run(@made, name) || Regex.run(@plugin, name) || Regex.run(@own, name) do
      [_, _owner, app | _] -> app
      _ -> nil
    end
  end

  # A plugin's name as people read it, as the App Store lists it, whether or not it's installed now.
  defp title(call, app),
    do: Enum.find_value(Call.deployed(call), app, &(Path.basename(&1[:plugin]) == app and is_binary(&1[:title]) and &1[:title]))

  # Whether the plugin an item's artifacts are of is installed for its owner, so they open once they're back.
  defp installed?(%{"plugin" => %{"name" => name}, "owner" => owner}), do: is_map_key(Plugins.running(owner), name)

  defp installed?(%{"artifacts" => [name], "owner" => owner}) do
    if name =~ @made, do: is_map_key(Plugins.running(owner), app_of(name)), else: true
  end

  defp owns!(call, name) do
    if not (name =~ @plugin or (name =~ @made and app_of(name) != "soft-delete")),
      do:
        raise(
          "#{name} isn't something an app made or a plugin, as its name says.\n\nusers/alice/made/git-repos/notes, users/alice/plugins/notes"
        )

    with {:refuse, why} <- owner(call, name), do: raise(why)
  end

  # Whoever owns an item's artifact owns it, and an uninstalled plugin's is whoever it was installed for.
  defp owns_item?(call, %{"plugin" => %{}, "owner" => owner}),
    do: if(call.subject == owner, do: Call.owns?(call), else: owner(call, owner) == :ok)
  defp owns_item?(call, %{"artifacts" => [name]}), do: owner(call, name) == :ok

  # A plugin is deleted only once nobody has it installed: whoever it's installed for, and whoever's it is.
  defp unused!(name) do
    if name =~ @plugin do
      owner = User.owner(name)
      {installed, _automatic} = OneCore.Plugins.Installed.read(owner)
      using = OneCore.Plugins.Installed.installed_for(name) ++ for({_, %{"plugin" => ^name}} <- installed, do: owner)

      if using != [],
        do:
          raise(
            "#{name} is installed for #{Enum.join(Enum.uniq(using), ", ")}. It's deleted once nobody has it.\n\n`uninstall` it first."
          )
    end
  end

  # Whether whoever the call runs for owns the artifact, as the plugins that validate `reach` say.
  defp owner(call, name),
    do:
      Hooks.validate(
        "reach",
        %{"who" => call.subject, "relation" => "owner", "artifact" => name},
        {call.actor, call.subject}
      )

  # Tells the plugins that hook `moved` of each artifact's move, for its owner, and gives why each that couldn't answer
  # couldn't, by its name.
  defp notify(call, record, to) do
    Enum.reduce(record["artifacts"], %{}, fn name, unheard ->
      about =
        cond do
          to == "active" and record["as"][name] != name -> %{"as" => record["as"][name]}
          to == "purging" -> %{"readable" => name in record["back"]}
          true -> %{}
        end

      {:ok, _parts, failed} =
        Hooks.collect("#{call.plugin}.moved", Map.merge(about, %{"artifact" => name, "to" => to}), {call.actor, record["owner"]})

      Map.merge(unheard, failed)
    end)
  end

  defp id, do: Base.encode16(:crypto.strong_rand_bytes(12), case: :lower)
  defp now, do: System.system_time(:millisecond)
end
