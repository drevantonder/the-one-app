defmodule RecentlyDeleted do
  @moduledoc """
  Recently Deleted: where what someone removes waits for 30 days, so it can be brought back as it was, before it's erased
  for good. Anything a plugin made for someone (`OneCore.Call.make/2`) is removed with this plugin's `remove`, which a
  plugin calls for its person as Git Repos does for a repo, and which an agent calls too. So is a plugin someone deployed
  (`<owner>/plugins/<name>`), once nobody has it installed: removing it is how what's deployed is deleted, as
  `deploy` is how it's made.

  Removing an artifact takes its blobs aside, byte for byte as they are in the bucket, to an artifact this plugin
  makes for its owner, `<owner>/made/recently-deleted/<id>`, and leaves nothing at its name. So it's gone for every plugin,
  whatever reads it, and stays under its owner's prefix, where the same servers reach it. An encrypted artifact's
  blobs go aside still encrypted, keys and all, and a database's log, snapshots and lease go together, so each
  opens again as it was once it's back. This plugin's own artifact keeps a line for each, `removed/<id>`: the
  artifact it was, who removed it and when.

  `restore` puts the blobs back at the artifact's name, only if nothing has been made there since, and `purge`,
  which the clock calls each day, erases what was removed more than 30 days ago. Who removes and restores an
  artifact is whoever owns it, as the plugins that validate `reach` say. Each of the three is told to every plugin
  that hooks into the hook this plugin offers, `recently-deleted.removed`, so one that keeps a list of what it made
  keeps it true.
  """
  @behaviour OneCore.Plugin

  # Any plugin deletes what it made through it, by its tool, without naming it among what it uses.
  @impl true
  def essential, do: true

  alias OneCore.{Artifact, Bucket, Hooks, User}

  # How long what's removed can be restored, after which it's erased.
  @kept :timer.hours(24 * 30)
  @made ~r{^(users/[^/]+|groups/[^/]+|network)/made/([^/]+)/[A-Za-z0-9_-]+$}
  @plugin ~r{^(users/[^/]+|groups/[^/]+|network)/plugins/[a-z][a-z0-9-]*$}

  @impl true
  def uses, do: ["database"]

  @impl true
  def offers do
    [
      %{
        name: "removed",
        kind: :observe,
        description:
          "What a plugin made for someone was removed, restored or purged. It's about the `artifact`'s name, and " <>
            "`what` happened: `removed`, `restored` or `purged`. A plugin that keeps a list of what it made observes it " <>
            "to keep its list true."
      }
    ]
  end

  @impl true
  def jobs, do: [%{tool: "purge", every: 86_400}]

  @impl true
  def tools do
    artifact = %{
      type: "string",
      description: "What was made, by its name: users/alice/made/git-repos/notes, or a plugin: users/alice/plugins/notes"
    }

    only = fn properties, required -> %{type: "object", properties: properties, required: required} end

    [
      %{
        name: "remove",
        description:
          "Removes something you own that an app made, such as a repo, or a plugin you deployed that nobody has installed: it's gone from everywhere, until `restore` brings it back as it was. After 30 days it's erased for good",
        inputs: only.(%{artifact: artifact}, ["artifact"])
      },
      %{
        name: "list",
        read_only: true,
        description:
          "What you own that was removed in the last 30 days, the latest first: what it was, who removed it, and when",
        inputs: only.(%{}, [])
      },
      %{
        name: "restore",
        description: "Brings back something you own that was removed in the last 30 days, as it was",
        inputs: only.(%{artifact: artifact}, ["artifact"])
      },
      %{
        name: "purge",
        description: "Erases for good everything removed more than 30 days ago. The clock calls it each day",
        inputs: only.(%{}, [])
      }
    ]
  end

  @impl true
  def call("remove", %{"artifact" => name}, call) do
    owns!(call, name)
    unused!(name)
    blobs = Bucket.list(name <> "/")
    if blobs == [], do: raise("There's nothing at #{name}. `recently-deleted list` lists what was removed.")
    id = Base.encode16(:crypto.strong_rand_bytes(12), case: :lower)
    aside = aside(name, id)
    moved(blobs, &(aside <> String.replace_prefix(&1, name <> "/", "")))
    line = %{"artifact" => name, "at" => now(), "by" => call.actor || call.subject}
    :ok = Artifact.write(call.own, "removed/" <> id, JSON.encode!(line))
    notify(call, name, "removed")
    "Removed #{name}. `restore` brings it back as it was, for 30 days."
  end

  def call("list", _inputs, call) do
    for {_id, line} <- removed(call), owns?(call, line["artifact"]) do
      %{artifact: line["artifact"], by: line["by"], at: line["at"]}
    end
    |> Enum.sort_by(& &1.at, :desc)
  end

  def call("restore", %{"artifact" => name}, call) do
    owns!(call, name)

    {id, _line} =
      removed(call)
      |> Enum.filter(fn {_id, line} -> line["artifact"] == name end)
      |> Enum.max_by(fn {_id, line} -> line["at"] end, fn ->
        raise "Nothing at #{name} was removed in the last 30 days. `recently-deleted list` lists what was."
      end)

    if Bucket.list(name <> "/") != [],
      do: raise("Something new is at #{name} since it was removed. Remove that first to bring the old one back.")

    aside = aside(name, id)
    moved(Bucket.list(aside), &(name <> "/" <> String.replace_prefix(&1, aside, "")))
    Artifact.remove(call.own, "removed/" <> id)
    notify(call, name, "restored")
    "Restored #{name}. It's as it was."
  end

  def call("purge", _inputs, call) do
    purged =
      for {id, line} <- removed(call), now() - line["at"] > @kept do
        for blob <- Bucket.list(aside(line["artifact"], id)), do: Bucket.remove(blob)
        Artifact.remove(call.own, "removed/" <> id)
        notify(call, line["artifact"], "purged")
        line["artifact"]
      end

    if purged == [], do: "Nothing to purge.", else: "Purged #{Enum.join(purged, ", ")}."
  end

  # Each blob, as it is in the bucket, to where `to` says, written only where there's none, and then removed from
  # where it was.
  defp moved(blobs, to) do
    blobs
    |> Task.async_stream(
      fn blob ->
        {:ok, data, _tag} = Bucket.read(blob)
        if Bucket.write(to.(blob), data) == :exists, do: raise("#{to.(blob)} is there already")
      end,
      max_concurrency: 16,
      timeout: :infinity
    )
    |> Stream.run()

    Task.async_stream(blobs, &Bucket.remove/1, max_concurrency: 16, timeout: :infinity) |> Stream.run()
  end

  # Every line of what's removed, by its id.
  defp removed(call) do
    ids = for "removed/" <> id <- Artifact.list(call.own, "removed/"), do: id

    for {id, {:ok, line, _tag}} <- Enum.zip(ids, Artifact.read_all(call.own, Enum.map(ids, &("removed/" <> &1)))),
        do: {id, JSON.decode!(line)}
  end

  # Where an artifact's blobs wait while it's removed: an artifact of this plugin's, made for the same owner.
  defp aside(name, id), do: "#{User.owner(name)}/made/recently-deleted/#{id}/"

  defp owns!(call, name) do
    if not (name =~ @plugin or (name =~ @made and Enum.at(Regex.run(@made, name), 2) != "recently-deleted")),
      do:
        raise(
          "#{name} isn't something an app made or a plugin, as its name says: users/alice/made/git-repos/notes, users/alice/plugins/notes"
        )

    with {:refuse, why} <- owner(call, name), do: raise(why)
  end

  # A plugin is removed only once nobody has it installed: whoever it's installed for, and whoever's it is.
  defp unused!(name) do
    if name =~ @plugin do
      owner = User.owner(name)
      {installed, _tag} = OneCore.Plugins.Installed.read(owner)
      using = OneCore.Plugins.Installed.installed_for(name) ++ for({_, ^name} <- installed, do: owner)

      if using != [],
        do:
          raise(
            "#{name} is installed for #{Enum.join(Enum.uniq(using), ", ")}. It's removed once nobody has it: `uninstall` it first."
          )
    end
  end

  defp owns?(call, name), do: owner(call, name) == :ok

  # Whether whoever the call runs for owns the artifact, as the plugins that validate `reach` say.
  defp owner(call, name),
    do:
      Hooks.validate(
        "reach",
        %{"who" => call.subject, "relation" => "owner", "artifact" => name},
        {call.actor, call.subject}
      )

  defp notify(call, name, what), do: OneCore.Call.run_hook(call, "removed", %{"artifact" => name, "what" => what})

  defp now, do: System.system_time(:millisecond)
end
