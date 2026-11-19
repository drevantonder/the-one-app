defmodule SoftDelete do
  @moduledoc """
  Soft delete, which people see as Recently Deleted: what someone deletes waits for 30 days, so it can be restored as it
  was, before it's purged for good. Anything an app made for someone (`OneCore.Call.make/2`) is deleted with this
  app's `delete`, which an app calls for its person as Git Repos does for a repo, and which an agent calls too. So
  is an app someone deployed (`<owner>/apps/<name>`), once nobody has it installed, and one superseded by the broader
  copy of itself that everyone who had it has now (One Core's hook `superseded`). An app uninstalled for
  someone (One Core's hook `uninstalled`) takes what it kept and made for them along, as one item, and this app's
  clock and Restore are the only ones it has: restoring the item installs the app again before what it kept is back
  (`OneCore.Apps.Lifecycle.restore/2`), and purging it removes the app for good once what it kept is erased
  (`OneCore.Apps.Lifecycle.purge/2`). So neither ever comes back without the other.

  Each item is a record in this app's own artifact, `items/<id>`, and it moves through states by the transition
  table `move/2`:

      active ─delete─► deleted ─30 days, or Delete Now─► purging ─► purged
      deleted ─Restore─► active, at its name, or beside it as notes-2 where something new is there since
      purging ─fails─► deleted, tried again the next day

  Active and purged are no record at all: a record says so only while the work of getting there is under way, and
  goes once it's done. An item lives for a month and any server may move it, while no process lives that long, so
  it's a table over records, each move made with `OneCore.Artifact.update/3`: two servers moving one item at once each
  see the other's move first, and a move the table doesn't have is refused with why.

  Deleting an artifact takes its blobs aside, byte for byte as they are in the bucket, to an artifact this app makes
  for its owner, `<owner>/made/soft-delete/<id>/<n>/`, one folder for each artifact of the item, and leaves nothing at
  its name. So it's gone for every app, whatever reads it, and stays under its owner's prefix, where the same
  servers reach it. An encrypted artifact's blobs go aside still encrypted, keys and all, and a database's log,
  snapshots and lease go together, so each opens again as it was once it's back. The record is written before
  anything moves and says when everything has (`moved`), so a move cut short is finished by the next one to touch the
  item, or by the clock. One restored while it's still moving aside is put back by what's moving it, once that's done,
  so two moves of an item never overlap.

  Who deletes, restores and purges an item is whoever owns it, as the apps that validate `reach` say. Every move is
  told to every app that hooks into the hook this app offers, `soft-delete.moved`, so one that keeps a list of
  what it made keeps it true. It's collected, so each move waits for their answers. Purging puts each artifact back at
  its name first, and tells them it's `purging` before anything is erased, so the app that made it reads it once
  more, as one does to revoke a token it holds. One that fails or doesn't answer holds the purge back a day, once: the
  next day it's purged whatever they answer, so no app keeps an item for ever.
  """
  @behaviour OneCore.App

  # Any app deletes what it made through it, by its tool, without naming it among what it requires.
  @impl true
  def icon, do: "client/icon.svg"

  require Logger
  alias OneCore.{Artifact, Bucket, Call, Hooks, Apps, Servers, User}
  alias OneCore.Apps.Lifecycle

  # How long a deleted item can be restored, after which it's purged.
  @retention :timer.hours(24 * 30)
  # How long after a purge failed it's tried again.
  @retry :timer.hours(24)
  # How long work on an item may take before it's taken as cut short, as by its server stopping.
  @stale :timer.hours(1)
  @made ~r{^(users/[^/]+|groups/[^/]+|network)/made/([^/]+)/([A-Za-z0-9_-]+)$}
  @app ~r{^(users/[^/]+|groups/[^/]+|network)/apps/([a-z][a-z0-9-]*)$}
  @own ~r{^(users/[^/]+|groups/[^/]+|network)/own/([a-z][a-z0-9-]*)$}

  # Its data is everyone's together: each item says whose it is, and only they reach it.
  @impl true
  def own, do: :installed

  @impl true
  def requires, do: %{"core" => ">= 0.32.0", "database" => "to keep what you deleted for 30 days"}

  @impl true
  def hooks, do: ["uninstalled", "superseded", "settings.sections"]

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
            "(false only where something new was made at its name since): the app that made it reads it, as one does " <>
            "to revoke a token it holds. An app that keeps a list of what it made keeps it true from it. Each move " <>
            "waits for the answers, and an app that fails or doesn't answer holds a purge back a day, once."
      }
    ]
  end

  @impl true
  def jobs, do: [%{tool: "expire", every: 86_400}]

  @impl true
  def tools do
    artifact = %{
      type: "string",
      description: "An artifact, by its name: users/alice/made/git-repos/notes, or an app: users/alice/apps/notes"
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
          "Deletes something you own that an app made, such as a repo, or an app you deployed that nobody has installed: it's gone from everywhere, until `restore` brings it back as it was. After 30 days it's purged for good",
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
                      app: %{
                        type: ["string", "null"],
                        description: "The app whose uninstalling deleted it, by the name it was installed by"
                      },
                      title: %{type: "string", description: "The name of the app its artifacts are of, as people read it"},
                      installed: %{
                        type: "boolean",
                        description: "Whether that app is installed for its owner, to open it once it's restored"
                      },
                      owner: %{type: "string", description: "Whose it is: users/<name>, groups/<name> or network"},
                      names: %{
                        type: ["string", "null"],
                        description:
                          "The app to ask what its artifacts are called, by its tool `names`, where their names don't say: the Vault's are ids, as its items' names are encrypted"
                      },
                      by: %{type: "string", description: "Who deleted it"},
                      at: %{type: "integer", description: "When, in milliseconds since 1970"},
                      purges: %{type: "integer", description: "When it's purged, in milliseconds since 1970"},
                      why: %{
                        type: ["string", "null"],
                        description: "Why purging it last failed; it's tried again the next day"
                      }
                    },
                    ["item", "state", "artifacts", "app", "title", "installed", "owner", "names", "by", "at", "purges", "why"]
                  )
              }
            },
            ["deleted"]
          )
      },
      %{
        name: "restore",
        description:
          "Restores a deleted item as it was: each artifact at its name, or beside it as notes-2 where something new is there since. An uninstalled app is installed again with what it kept. One whose app isn't installed waits for it",
        inputs: only.(%{item: item}, ["item"]),
        outputs:
          only.(
            %{
              restored: %{type: "array", items: restored},
              installed: %{type: "boolean", description: "Whether the app its artifacts are of is installed, to open them"},
              title: %{type: "string", description: "That app's name, as people read it"}
            },
            ["restored", "installed", "title"]
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
  their apps are told), `:erasing`, `{:fail, why, how}` (purging it failed: `:partly` once some of it was erased,
  `:unheard` where an app failed its hook, so the next purge goes on whatever they answer, or `:whole`) and
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

  # Recently Deleted is a section of Settings.
  @impl true
  def hook("settings.sections", _about, _call),
    do: [%{in: "settings", name: "Recently Deleted", component: "deleted", icon: "client/icon.svg", group: "system", order: 2}]

  # The apps that required it went with it (`with`), and are one item with it: Restore brings them all back. What they
  # kept and made that goes with them, One Core says (`artifacts`).
  def hook("uninstalled", %{"for" => owner, "name" => name, "app" => app} = about, call) do
    # It's heard where the app was uninstalled, and its items are kept by a server acting for the network.
    if not Servers.here?(call.installed), do: Call.forward(call, call.installed)
    went = for %{"name" => other, "app" => artifact} <- List.wrap(about["with"]), do: %{"name" => other, "artifact" => artifact}

    record = %{
      "owner" => owner,
      "artifacts" => about["artifacts"],
      "app" => %{"name" => name, "artifact" => app},
      "with" => went,
      "title" => title(call, Path.basename(app)),
      "by" => call.actor || owner,
      "at" => now()
    }

    id = id()
    {:ok, _record} = change(call, id, {:delete, record})
    # Moving an app's data can take longer than an observer is given, and it's finished later if it's cut short.
    Task.start(fn -> aside(call, id) end)
    :ok
  end

  # An app superseded by the broader copy of itself, which everyone who had it has now, is installed for nobody, and
  # goes as one deleted does, so it comes back as it was if it's restored. One that went already, as when two servers
  # superseded it at once, has nothing left to delete.
  def hook("superseded", %{"app" => app}, call) do
    if not Servers.here?(call.installed), do: Call.forward(call, call.installed)

    if app =~ @app and Bucket.list(app <> "/latest") != [] do
      unused!(app)
      deleted(call, app)
    end

    :ok
  end

  # Takes the artifact `name` aside as one item, whoever's it is: the item's id.
  defp deleted(call, name) do
    record = %{
      "owner" => User.owner(name),
      "artifacts" => [name],
      "title" => title(call, app_of(name)),
      "by" => call.actor || call.subject,
      "at" => now()
    }

    id = id()
    {:ok, _record} = change(call, id, {:delete, record})
    aside(call, id)
    id
  end

  # Restores an item: each artifact at its name, or beside something new there since, and an uninstalled app before
  # what it kept. Gives its record as it was moved.
  defp restored(call, id, record) do
    # Something new at an artifact's name since is left as it is, and this one goes beside it.
    as =
      Map.new(record["artifacts"], fn name ->
        {name, if(record["moved"] and taken?(name), do: free(name), else: name)}
      end)

    # Restored while it's still moving aside, whoever is moving it puts it back once its own removing is done (`aside/2`),
    # so the two never move it at once.
    moved =
      case change(call, id, {:restore, as}) do
        {:ok, moved} -> moved
        {:error, why} -> raise why
      end

    with %{"name" => name} <- record["app"],
         {:error, why} <- Lifecycle.restore(record["owner"], name),
         do: raise("#{record["title"]} wasn't installed again for #{record["owner"]}: #{why}")

    if moved["moved"], do: back(call, id), else: moved
  end

  @impl true
  def call("delete", %{"artifact" => name}, call) do
    owns!(call, name)
    unused!(name)
    if Bucket.list(name <> "/") == [], do: raise("There's nothing at #{name}.\n\n`soft-delete list` lists what's deleted.")
    %{item: deleted(call, name), artifact: name}
  end

  def call("list", _inputs, call) do
    deleted =
      for {id, %{"state" => state} = record} <- records(call), state in ["deleted", "purging"], owns_item?(call, record) do
        %{
          item: id,
          state: state,
          artifacts: record["artifacts"],
          app: record["app"]["name"],
          title: record["title"],
          installed: installed?(record),
          owner: record["owner"],
          names: names(record),
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

    if app = record["app"] do
      if Apps.app(record["owner"], app["name"]) != nil or
           Bucket.list("#{record["owner"]}/own/#{app["name"]}/") != [],
         do:
           raise(
             "#{record["title"]} was installed again since it was deleted. Uninstall it first to restore the one that was."
           )

      # What an app kept comes back only with it.
      if not match?(%{"state" => "recently_deleted"}, OneCore.Apps.Installed.read(record["owner"])[app["name"]]),
        do: raise("#{record["title"]} isn't uninstalled for #{record["owner"]} any more, so what it kept can't come back with it.")
    end

    record = restored(call, id, record)

    %{
      restored: for(name <- record["artifacts"], do: %{artifact: name, as: record["as"][name]}),
      installed: installed?(record),
      title: record["title"]
    }
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
    done = for {id, record} <- records(call), do: expired(call, id, record, stale)
    if Enum.any?(done, &elem(&1, 1)), do: Call.changed(call)
    %{purged: Enum.flat_map(done, &elem(&1, 0))}
  end

  # What the clock does with each item: purges one that's due, and finishes any work on one that was cut short. It
  # gives the names it purged, and whether it changed the item's record.
  defp expired(call, id, record, stale) do
    case record do
      %{"state" => "active", "since" => since} when since < stale ->
        back(call, id)
        {[], true}

      %{"state" => "purged"} ->
        gone(call, id, record)
        {[], true}

      %{"state" => "purging", "since" => since} when since < stale ->
        put_aside(record, id)
        change(call, id, {:fail, "It was cut short.", if(record["erasing"], do: :partly, else: :whole)})
        {[], true}

      %{"state" => "deleted", "moved" => false, "at" => at} when at < stale ->
        aside(call, id)
        {[], true}

      %{"state" => "deleted", "at" => at} = record ->
        if at + @retention <= now() and Map.get(record, "retry", 0) <= now() do
          # A purge that failed says why in the record, and is tried again tomorrow.
          case purge(call, id) do
            {:error, _why} -> {[], true}
            purged -> {purged, true}
          end
        else
          {[], false}
        end

      _ ->
        {[], false}
    end
  rescue
    error ->
      Logger.error("soft-delete couldn't expire #{id}: #{Exception.message(error)}")
      {[], true}
  end

  # Purges an item, or raises why it couldn't.
  defp purge!(call, id) do
    case purge(call, id) do
      {:error, why} -> raise why
      purged -> purged
    end
  end

  # Moves it to purging, puts each artifact back at its name and tells the apps that hook `moved`, so the one that
  # made it reads it once more, then erases it, and then it's purged: its record goes. One an app failed to answer for
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
        why = Enum.map_join(unheard, "\n", fn {app, why} -> "#{app}: #{why}" end)
        failed(call, id, "An app didn't answer as it was purged.\n#{why}", :unheard)
      else
        {:ok, record} = change(call, id, :erasing)
        erase(call, id, record)
      end
    end
  rescue
    error ->
      record = record(call, id)

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
        for name <- record["artifacts"], name =~ @app, do: Bucket.remove(listing(name))
        # An uninstalled app goes for good with it, unless it went already.
        with %{"name" => name} <- record["app"], do: Lifecycle.purge(record["owner"], name)
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
    Artifact.remove(call.own, "items/" <> id)
    notify(call, record, "purged")
  end

  # Takes each artifact of an item aside, and says so in its record once it's all there, then tells whoever keeps a list.
  # One restored meanwhile is put back by this, as the restore left it to.
  defp aside(call, id) do
    record = record(call, id)
    moved_aside(record, id)

    case change(call, id, :moved) do
      {:ok, record} ->
        notify(call, record, "deleted")
        record

      {:error, why} ->
        if record(call, id)["state"] == "active", do: back(call, id), else: raise(why)
    end
  end

  defp moved_aside(record, id) do
    for {name, n} <- Enum.with_index(record["artifacts"]),
        do: moved(Bucket.list(name <> "/"), &(aside(record, id, n) <> String.replace_prefix(&1, name <> "/", "")))
  end

  # Puts what's aside back where its record says, and then it's active: its record goes.
  defp back(call, id) do
    record = record(call, id)

    for {name, n} <- Enum.with_index(record["artifacts"]) do
      from = aside(record, id, n)
      moved(Bucket.list(from), &(record["as"][name] <> "/" <> String.replace_prefix(&1, from, "")))
    end

    relisted(record["artifacts"])
    Artifact.remove(call.own, "items/" <> id)
    notify(call, record, "active")
    record
  end

  # Moves an item's record by `event`, as `move/2` says: `{:ok, record}`, or `{:error, why}` with it as it was.
  defp change(call, id, event) do
    written =
      Artifact.update(call.own, "items/" <> id, fn was ->
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

  # An app's listing in the App Store's Marketplace, which shows it only while it has a latest version: so it's kept while it's
  # deleted, for `restore`, and goes when it's purged.
  defp listing(app), do: "network/deployed/#{User.owner(app)}/#{Path.basename(app)}"

  # Servers look again at what's deployed, so an app deleted or restored leaves the Marketplace or is back in it at once.
  defp relisted(names) do
    apps = for name <- names, name =~ @app, do: User.owner(name)

    if apps != [] do
      OneCore.Apps.Deploy.forget()
      OneCore.Server.notify_servers(Enum.uniq(apps))
    end
  end

  # Every record, by its item's id.
  defp records(call) do
    ids = for "items/" <> id <- Artifact.list(call.own, "items/"), do: id

    for {id, {:ok, record, _tag}} <- Enum.zip(ids, Artifact.read_all(call.own, Enum.map(ids, &("items/" <> &1)))),
        into: %{},
        do: {id, JSON.decode!(record)}
  end

  # An item's record, or nil.
  defp record(call, id) do
    with {:ok, record, _tag} <- Artifact.read(call.own, "items/" <> id),
         do: JSON.decode!(record),
         else: (:absent -> nil)
  end

  # An item's record, if whoever the call runs for owns it.
  defp record!(call, id) do
    record = record(call, id)

    if record == nil or not owns_item?(call, record),
      do: raise("That isn't in Recently Deleted.\n\n`soft-delete list` lists what is.")

    record
  end

  # Where an item's artifact `n` waits while it's deleted: an artifact of this app's, made for the same owner.
  defp aside(record, id, n), do: "#{record["owner"]}/made/soft-delete/#{id}/#{n}/"

  # The name of the app an artifact is of: the one that made it, or the app it is.
  defp app_of(name) do
    case Regex.run(@made, name) || Regex.run(@app, name) || Regex.run(@own, name) do
      [_, _owner, app | _] -> app
      _ -> nil
    end
  end

  # An app's name as people read it, as the Marketplace lists it, whether or not it's installed now.
  defp title(call, app),
    do: Enum.find_value(Call.deployed(call), app, &(Path.basename(&1[:app]) == app and is_binary(&1[:title]) and &1[:title]))

  # Whether the app an item's artifacts are of is installed for its owner, so they open once they're back.
  defp installed?(%{"app" => %{"name" => name}, "owner" => owner}), do: is_map_key(Apps.running(owner), name)

  defp installed?(%{"artifacts" => [name], "owner" => owner}) do
    if name =~ @made, do: is_map_key(Apps.running(owner), app_of(name)), else: true
  end

  # The app that says what an item's artifact is called, where its name doesn't: one that offers `names`, asked by
  # Recently Deleted on its person's device, so this app never reads what it says.
  defp names(%{"artifacts" => [name], "owner" => owner} = record) when not is_map_key(record, "app") do
    app = app_of(name)
    version = Apps.running(owner)[app]
    if version && Enum.any?(version.tools, &(&1.name == "names")), do: app
  end

  defp names(_record), do: nil

  defp owns!(call, name) do
    if not (name =~ @app or (name =~ @made and app_of(name) != "soft-delete")),
      do:
        raise(
          "#{name} isn't something an app made, or an app someone deployed, as its name says.\n\nusers/alice/made/git-repos/notes, users/alice/apps/notes"
        )

    with {:refuse, why} <- owner(call, name), do: raise(why)
  end

  # Whoever owns an item's artifact owns it, and an uninstalled app's is whoever it was installed for.
  defp owns_item?(call, %{"app" => %{}, "owner" => owner}),
    do: if(call.subject == owner, do: Call.owns?(call), else: owner(call, owner) == :ok)
  defp owns_item?(call, %{"artifacts" => [name]}), do: owner(call, name) == :ok

  # An app is deleted only once nobody has it installed: whoever it's installed for, and whoever's it is.
  defp unused!(name) do
    if name =~ @app do
      owner = User.owner(name)
      installed = OneCore.Apps.Installed.read(owner)
      using = OneCore.Apps.Installed.installed_for(name) ++ for({_, %{"app" => ^name}} <- installed, do: owner)

      if using != [],
        do:
          raise(
            "#{name} is installed for #{Enum.join(Enum.uniq(using), ", ")}. It's deleted once nobody has it.\n\n`uninstall` it first."
          )
    end
  end

  # Whether whoever asked owns the artifact, as the apps that validate `reach` say: a member running a call for a
  # group or the network owns only what they do themselves. A scheduled job is asked by nobody, so it's whoever it
  # runs for.
  defp owner(call, name),
    do:
      Hooks.validate(
        "reach",
        %{"who" => call.actor || call.subject, "relation" => "owner", "artifact" => name},
        {call.actor, call.subject}
      )

  # Tells the apps that hook `moved` of each artifact's move, for its owner, and gives why each that couldn't answer
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
        Hooks.collect("#{call.app}.moved", Map.merge(about, %{"artifact" => name, "to" => to}), {call.actor, record["owner"]})

      Map.merge(unheard, failed)
    end)
  end

  defp id, do: Base.encode16(:crypto.strong_rand_bytes(12), case: :lower)
  defp now, do: System.system_time(:millisecond)
end
