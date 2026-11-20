defmodule OneCore.Apps.Installed do
  @moduledoc """
  What's installed for an owner, a user, a group or the network: one install per app per owner, each one explicit
  state machine.

      not installed ─Install─► installing ─► installed ─Uninstall─► recently deleted ─purged─► not installed
                                                                    recently deleted ─restored─► installed
      installing ─fails, or its task ends first, as its server stopping ends it─► not installed (why · Try Again)
      not installed ─Deploy─► installed (whoever it's for deployed it, so it's theirs)
      installed, recently deleted or not installed ─Superseded─► not installed: the network's has it now
                                                               └─► as it was, of a group's copy, the broader one

  with a `stage` inside installing: its `requirements` installed first, then `fetching` its code, `building` it and
  `starting` it. Whatever's installed runs the app's one version, the same everywhere it's installed: its updates
  and whether it's trusted are the app's (`OneCore.Apps.Update`). Recently Deleted (the app soft-delete) purges and
  restores it, with what it kept, by its own clock and its own Restore: this machine has neither.

  An app has one copy (`OneCore.Apps.Lifecycle.supersede/2`). Installed or deployed for a group or the network, the
  same app's copy that only those it covers have is superseded by it: each of that copy's records goes into the
  network's install, which acts for them now, or moves to a group's copy, where it was in its life; and whoever it was
  for keeps their data, which is kept by the name it's installed by, not by its artifact.

  Each owner's are one blob, `<owner>/installs`, with each app's record by the name it's installed by: the
  `app` it is, its `state`, its `stage`, `why` it isn't installed where it failed to be, when it was `deleted`, who
  installed it (`by`), and the `task` installing it, the MCP task whose end, however it ends, ends installing
  (`OneCore.Apps.Lifecycle`). A record lives for years and every server and client reads it, while no one process lives
  that long, so the machine is the transition table `move/3` over records, each move made with
  `OneCore.Artifact.update/3`: two servers moving one record at once each see the other's move first, and a move the
  table doesn't have is refused, saying why. A record keeps what it held that this doesn't read.

  That someone installed an app is kept with the app too, as the blob `installed/<who>` in its artifact, from when
  it's installing, which ends `deleted` while it's in their Recently Deleted and goes where it failed to install: so a
  deploy tells their servers, and it's in use for them only while it's installing or installed.

  An app that keeps each person's data apart (`own/0`, `OneCore.Apps.Calls.whose/3`), installed for the network, has
  an own artifact for each person who keeps something with it, as an Android app installed for every user of a device
  has a data folder for each. Who they are is kept beside the network's installs, the blob
  `<owner>/subjects/<name>/<who>` for each, so its jobs run and its services act for each of them (`subjects/2`).
  """
  require Logger
  alias OneCore.{Artifact, Bucket, Cache, Call, Servers}

  @blob "installs"

  @doc "Each app's record, as `<owner>/installs` keeps it, by the name it's installed by."
  def read(owner) do
    case Bucket.read("#{owner}/#{@blob}") do
      {:ok, json, _tag} -> JSON.decode!(json)["apps"] || %{}
      :absent -> %{}
    end
  end

  @doc """
  The apps that run for `owner`, by the name each is installed by, with its artifact: what's installed, and what's
  installing once it's starting.
  """
  def running(owner) do
    for {name, %{"app" => app} = record} <- read(owner), runs?(record), into: %{}, do: {name, app}
  end

  defp runs?(%{"state" => "installed"}), do: true
  defp runs?(%{"state" => "installing", "stage" => "starting"}), do: true
  defp runs?(_record), do: false

  @doc """
  Moves the record of the app `name` for `owner` by `event`, as `move/3` says, and gives the record it's at
  now, `nil` for none, or `{:error, why}` where the table hasn't that move. The app's artifact is told.
  """
  def change(owner, name, event) do
    was = read(owner)[name]

    changed =
      Artifact.update(%Artifact{prefix: "#{owner}/"}, @blob, fn was ->
        kept = if was, do: JSON.decode!(was), else: %{"apps" => read(owner)}
        records = kept["apps"] || %{}

        case move(records[name], event, {owner, name}) do
          {:ok, nil} -> JSON.encode!(Map.put(kept, "apps", Map.delete(records, name)))
          {:ok, record} -> JSON.encode!(Map.put(kept, "apps", Map.put(records, name, record)))
          {:error, why} -> throw({__MODULE__, why})
        end
      end)

    record = JSON.decode!(changed)["apps"][name]
    if record, do: marked(owner, record)
    # A record that moved to another artifact isn't the one before's any more.
    if was && was["app"] != (record || %{})["app"], do: unmarked(was["app"], owner)
    record
  catch
    {__MODULE__, why} -> {:error, why}
  end

  # The app's artifact says it's installed for `owner`, and whether it's in their Recently Deleted.
  defp marked(owner, %{"app" => app, "state" => state}) when state in ~w(installing installed recently_deleted) do
    blob = "#{app}/installed/#{owner}"
    data = if state == "recently_deleted", do: "deleted", else: ""

    case Bucket.read(blob) do
      {:ok, ^data, _tag} -> :ok
      {:ok, _was, tag} -> Bucket.replace(blob, data, tag)
      :absent -> Bucket.write(blob, data)
    end
  end

  defp marked(owner, %{"app" => app, "state" => "not_installed"}), do: unmarked(app, owner)

  @doc "Says the app `artifact` is no longer installed for `owner`, unless they have it by another name."
  def unmarked(artifact, owner) do
    has = fn {_name, record} -> record["app"] == artifact and record["state"] != "not_installed" end
    if not Enum.any?(read(owner), has), do: Bucket.remove("#{artifact}/installed/#{owner}")
  end

  @doc """
  The transition table: where a record goes on `event`, from where it is (`nil` for not installed). `{:ok, record}`,
  `{:ok, nil}` for not installed any more, or `{:error, why}` for a move that isn't in it. `at` is whose and which
  app, for what it says.

  Events, as the machine names them: `{:install, app, by, task}` (Install, and Try Again on one that failed to),
  `{:stage, stage}`, `:done`, `{:fail, why}`, `{:uninstall, group}`, `:restore` and `:purge` (Recently Deleted's), where
  `group` says what went to Recently Deleted together, as one: `%{"with" => names}` on the one uninstalled, and
  `%{"deleted_with" => name}` on each app that required it; and
  `{:deployed, app, by}`, where whoever it's installed for deployed it, which is theirs, installed at once; and
  `{:superseded, app, inside}`, where `app` is the broader copy of the same app: `inside` where it's installed for
  someone whose apps theirs are found before (the network), whose install it's in now, so the record goes, and
  otherwise (a group's) it moves to `app` as it is. One that's installing finishes first.
  """
  def move(record, event, at)

  def move(record, {:install, app, by, task}, _at)
      when record == nil or :erlang.map_get("state", record) == "not_installed",
      do:
        {:ok,
         (record || %{})
         |> Map.delete("server")
         |> Map.merge(%{"app" => app, "by" => by, "task" => task})
         |> moved("installing", "requirements")}

  def move(%{"state" => "installing"} = record, {:stage, stage}, _at) when stage in ~w(requirements fetching building starting),
    do: {:ok, moved(record, "installing", stage)}

  def move(%{"state" => "installing"} = record, :done, _at), do: {:ok, moved(record, "installed")}

  def move(%{"state" => "installing"} = record, {:fail, why}, _at),
    do: {:ok, record |> moved("not_installed") |> Map.put("why", why)}

  def move(%{"state" => "installed"} = record, {:uninstall, group}, _at),
    do: {:ok, record |> moved("recently_deleted") |> Map.merge(Map.put(group, "deleted", now()))}

  def move(%{"state" => "recently_deleted"} = record, :restore, _at),
    do: {:ok, record |> Map.drop(["deleted", "with", "deleted_with"]) |> moved("installed")}

  def move(%{"state" => "recently_deleted"}, :purge, _at), do: {:ok, nil}

  def move(record, {:deployed, app, by}, _at)
      when record == nil or :erlang.map_get("state", record) in ["installed", "not_installed"] do
    if record && record["state"] == "installed",
      do: {:ok, %{record | "app" => app}},
      else: {:ok, (record || %{}) |> Map.merge(%{"app" => app, "by" => by}) |> moved("installed")}
  end

  def move(%{"state" => state}, {:superseded, _app, true}, _at) when state in ~w(installed recently_deleted not_installed),
    do: {:ok, nil}

  def move(%{"state" => state} = record, {:superseded, app, false}, _at)
      when state in ~w(installed recently_deleted not_installed),
      do: {:ok, %{record | "app" => app}}

  def move(record, event, {owner, name}), do: {:error, refused(record, event, name, owner)}

  # A record in `state`, at `stage` within it if it has one, since now.
  defp moved(record, state, stage \\ nil) do
    record
    |> Map.merge(%{"state" => state, "at" => now()})
    |> Map.drop(["stage", "why"])
    |> then(&if(stage, do: Map.put(&1, "stage", stage), else: &1))
  end

  # What a refused move says: where it is, and what it can do from there.
  defp refused(nil, _event, name, owner), do: "#{name} isn't installed for #{owner}."

  defp refused(%{"state" => "not_installed", "why" => why}, _event, name, owner),
    do: "#{name} isn't installed for #{owner}: #{why}\n\nTry again (apps_retry)."

  defp refused(%{"state" => "recently_deleted"}, _event, name, owner),
    do: "#{name} is in Recently Deleted for #{owner}. Restore it there (soft-delete restore) to use it again."

  defp refused(%{"state" => "installing", "stage" => stage}, _event, name, _owner),
    do: "#{name} is installing (#{stage}). Wait until it's done."

  defp refused(%{"state" => "installed"}, {:install, _, _, _}, name, owner), do: "#{name} is installed for #{owner} already."
  defp refused(_record, _event, name, owner), do: "That doesn't apply to #{name} for #{owner} as it is."

  defp now, do: System.system_time(:millisecond)

  @doc "Who the app `artifact` is installed for, as its artifact says, besides whoever's it is: in their Recently Deleted too."
  def installed_for(artifact) do
    for blob <- Bucket.list("#{artifact}/installed/"), do: String.replace_prefix(blob, "#{artifact}/installed/", "")
  end

  @doc "Who the app `artifact` is in use for, as its artifact says: installed for them, and not in their Recently Deleted."
  def in_use_for(artifact) do
    blobs = Bucket.list("#{artifact}/installed/")

    for {blob, {:ok, marked, _tag}} <- Enum.zip(blobs, Bucket.read_all(blobs)), not String.ends_with?(marked, "deleted"),
        do: String.replace_prefix(blob, "#{artifact}/installed/", "")
  end

  @doc """
  Says that `who` keeps their own data with the app installed for `owner` as `name`, which keeps each person's apart.
  Said once: each server keeps that it has. Only the owner's servers write beside their installs, so a person's own
  server running their call of a network's app (`OneCore.Apps.runs?/3`) has one of the network's say it
  (`apps/subject`), and says it again with their next call where none answered.
  """
  def subject(owner, name, who) do
    Cache.fetch(__MODULE__, {owner, name, who}, fn ->
      if Servers.here?(owner) do
        Bucket.write("#{owner}/subjects/#{name}/#{who}", "")
        {:commit, :ok}
      else
        elsewhere(owner, name, who)
      end
    end)
  end

  defp elsewhere(owner, name, who) do
    Call.run_elsewhere({"apps/subject", %{installed: owner, app: name}}, {nil, who}, false, {owner, OneCore.Apps.servers(owner)}, name)
  rescue
    unanswered ->
      Logger.warning("#{who} isn't said to keep their own data with #{name} yet: #{Exception.message(unanswered)}")
      {:ignore, :ok}
  catch
    {Call, {:forwarded, _said}} -> {:commit, :ok}
  end

  @doc "Who keeps their own data with the app installed for `owner` as `name`, besides `owner`: each user or group."
  def subjects(owner, name) do
    for blob <- Bucket.list("#{owner}/subjects/#{name}/"),
        do: String.replace_prefix(blob, "#{owner}/subjects/#{name}/", "")
  end

  @doc "Forgets who kept their own data with the app installed for `owner` as `name`, once it's removed for good."
  def unsubjected(owner, name) do
    for who <- subjects(owner, name) do
      Bucket.remove("#{owner}/subjects/#{name}/#{who}")
      Cache.invalidate(__MODULE__, {owner, name, who})
    end

    :ok
  end

  @doc "Whether `name` is one an app is installed by: `:ok`, or `{:error, why}`."
  def named(name) do
    cond do
      not (is_binary(name) and name =~ ~r/^[a-z][a-z0-9-]*\z/) ->
        {:error, "An app's name is lowercase letters, digits and hyphens, starting with a letter"}

      # A client finds the UI library where it finds an app's files, by this name (`OneCore.Mcp`), and apps' icons
      # by theirs (`OneCore.Apps.Icons`).
      name in ~w(ui icons) ->
        {:error, "#{name} is One Core's own, where clients find #{if name == "ui", do: "the UI library", else: "apps' icons"}: name the app something else"}

      true ->
        :ok
    end
  end
end
