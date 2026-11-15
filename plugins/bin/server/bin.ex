defmodule Bin do
  @moduledoc """
  The bin: where what someone removes waits for 30 days, so it can be brought back as it was, before it's erased
  for good. Anything a plugin made for someone (`OneCore.Call.make/2`) is removed with this plugin's `remove`, which a
  plugin calls for its person as Repos does for a repo, and which an agent calls too.

  Removing an artifact takes its blobs aside, byte for byte as they are in the bucket, to an artifact this plugin
  makes for its owner, `<owner>/made/bin/<id>`, and leaves nothing at its name. So it's gone for every plugin,
  whatever reads it, and stays under its owner's prefix, where the same servers reach it. An encrypted artifact's
  blobs go aside still encrypted, keys and all, and a database's log, snapshots and lease go together, so each
  opens again as it was once it's back. This plugin's own artifact keeps a line for each, `removed/<id>`: the
  artifact it was, who removed it and when.

  `restore` puts the blobs back at the artifact's name, only if nothing has been made there since, and `erase`,
  which the clock calls each day, erases what was removed more than 30 days ago. Who removes and restores an
  artifact is whoever owns it, as the plugins that decide `reach` say. Each of the three is told to every plugin
  that hears `removed`, so one that keeps a list of what it made keeps it true.
  """
  @behaviour OneCore.Plugin
  alias OneCore.{Artifact, Bucket, Hooks, User}

  # How long what's removed can be restored, after which it's erased.
  @kept :timer.hours(24 * 30)
  @made ~r{^(users/[^/]+|groups/[^/]+|network)/made/([^/]+)/[A-Za-z0-9_-]+$}

  @impl true
  def uses, do: ["database"]

  @impl true
  def jobs, do: [%{tool: "erase", every: 86_400}]

  @impl true
  def tools do
    artifact = %{type: "string", description: "What was made, by its name: users/alice/made/repos/notes"}
    only = fn properties, required -> %{type: "object", properties: properties, required: required} end

    [
      %{
        name: "remove",
        description:
          "Removes something you own that an app made, such as a repo: it's gone from everywhere, until `restore` brings it back as it was. After 30 days it's erased for good",
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
        name: "erase",
        description: "Erases for good everything removed more than 30 days ago. The clock calls it each day",
        inputs: only.(%{}, [])
      }
    ]
  end

  @impl true
  def call("remove", %{"artifact" => name}, call) do
    owns!(call, name)
    blobs = Bucket.list(name <> "/")
    if blobs == [], do: raise("There's nothing at #{name}. `bin list` lists what was removed.")
    id = Base.encode16(:crypto.strong_rand_bytes(12), case: :lower)
    aside = aside(name, id)
    moved(blobs, &(aside <> String.replace_prefix(&1, name <> "/", "")))
    line = %{"artifact" => name, "at" => now(), "by" => call.by || call.for}
    :ok = Artifact.write(call.own, "removed/" <> id, JSON.encode!(line))
    told(call, name, "removed")
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
        raise "Nothing at #{name} was removed in the last 30 days. `bin list` lists what was."
      end)

    if Bucket.list(name <> "/") != [],
      do: raise("Something new is at #{name} since it was removed. Remove that first to bring the old one back.")

    aside = aside(name, id)
    moved(Bucket.list(aside), &(name <> "/" <> String.replace_prefix(&1, aside, "")))
    Artifact.remove(call.own, "removed/" <> id)
    told(call, name, "restored")
    "Restored #{name}. It's as it was."
  end

  def call("erase", _inputs, call) do
    erased =
      for {id, line} <- removed(call), now() - line["at"] > @kept do
        for blob <- Bucket.list(aside(line["artifact"], id)), do: Bucket.remove(blob)
        Artifact.remove(call.own, "removed/" <> id)
        told(call, line["artifact"], "erased")
        line["artifact"]
      end

    if erased == [], do: "Nothing to erase.", else: "Erased #{Enum.join(erased, ", ")}."
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
  defp aside(name, id), do: "#{User.owner(name)}/made/bin/#{id}/"

  defp owns!(call, name) do
    if not (name =~ @made) or Enum.at(Regex.run(@made, name), 2) == "bin",
      do: raise("#{name} isn't something an app made, as its name says: users/alice/made/repos/notes")

    with {:refuse, why} <- owner(call, name), do: raise(why)
  end

  defp owns?(call, name), do: owner(call, name) == :ok

  # Whether whoever the call runs for owns the artifact, as the plugins that decide `reach` say.
  defp owner(call, name),
    do: Hooks.decide("reach", %{"who" => call.for, "relation" => "owner", "artifact" => name}, {call.by, call.for})

  defp told(call, name, what), do: Hooks.heard("removed", %{"artifact" => name, "what" => what}, {call.by, call.for})

  defp now, do: System.system_time(:millisecond)
end
