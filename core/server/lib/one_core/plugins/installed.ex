defmodule OneCore.Plugins.Installed do
  @moduledoc """
  What's installed for an owner, a user, a group or the network, and where each plugin is in its life there: one
  explicit state machine per plugin per owner, in three parts side by side.

  Its lifecycle:

      not installed ─Install─► installing ─► installed
      installed ─Uninstall─► recently deleted ─30 days─► removed (its data gone)
      recently deleted ─Restore─► installed
      installing │ updating ─fails─► failed(why) ─Try Again─► installing │ updating
                                                ─Dismiss─► not installed │ installed (the old version)

  with a `stage` inside installing and updating: `fetching` its code, `building` it, then `starting` it. Whether
  it's `trusted`, or untrusted and runs in a sandbox: switching it is an update to the same version, the other way.
  And, while it's installed, its `update`: none while it's up to date, then `available` with the newer version, or
  `needs_approval` where that version asks for more permissions. A failed update leaves its version running.

  Each owner's are one blob, `<owner>/installs`: whether its `automatic` updates are on (they are unless it says),
  and each plugin's record by the name it's installed by. A record lives for years and every server and client reads
  it, while no one process lives that long, so the machine is the transition table `move/3` over records, each move
  made with `OneCore.Artifact.update/3`: two servers moving one record at once each see the other's move first, and a
  move the table doesn't have is refused, saying why.

  A call that runs for someone finds a plugin by its name among what runs for them, and then among what runs for the
  network (`OneCore.Plugins.chain/1`): the version each record says, which is its own, so each owner's install
  updates on its own. A plugin has an artifact of its own for each one it's installed for, `<theirs>/own/<name>`.
  That someone installed a plugin is kept with the plugin too, as the blob `installed/<who>` in its artifact, which
  says the version they have, and ` deleted` after it while it's in their Recently Deleted: so a deploy tells their
  servers it has a newer one, its history keeps the version they have (`OneCore.Plugins.Store.prune/1`), and it's
  in use for them only while it isn't deleted.
  """
  alias OneCore.{Artifact, Bucket}

  @blob "installs"
  @running ~w(installed updating failed)
  @transient ~w(installing updating)

  @doc """
  Each plugin's record, as `<owner>/installs` keeps it, by the name it's installed by, and whether the owner's
  automatic updates are on: `{records, automatic}`.
  """
  def read(owner) do
    case Bucket.read("#{owner}/#{@blob}") do
      {:ok, json, _tag} ->
        kept = JSON.decode!(json)
        {kept["plugins"] || %{}, kept["automatic"] != false}

      :absent ->
        {copied(owner), true}
    end
  end

  # What a server from before installs had state kept, `<owner>/installed`: each plugin's name with its artifact,
  # running its latest version. It's copied once to the shape it has now, installed at that version, and the blob it
  # was is left as it is.
  defp copied(owner) do
    case Bucket.read("#{owner}/installed") do
      {:ok, json, _tag} ->
        records =
          for {name, plugin} <- JSON.decode!(json), {number, _code, _tag} <- [OneCore.Plugins.Store.latest(plugin)], into: %{} do
            {name, record(plugin, number, OneCore.Plugins.Store.about(plugin)["trusted"] != false)}
          end

        # Only a server acting for them writes what's theirs; another reads it as it is until one has.
        if OneCore.Servers.here?(owner) do
          Bucket.write("#{owner}/#{@blob}", JSON.encode!(%{plugins: records}))
          for {_name, record} <- records, do: marked(owner, record)
        end

        records

      :absent ->
        %{}
    end
  end

  @doc "A record of a plugin installed at `number`."
  def record(plugin, number, trusted),
    do: %{"plugin" => plugin, "version" => number, "trusted" => trusted, "state" => "installed"}

  @doc """
  The plugins that run for `owner`, by the name each is installed by: its artifact, the version that runs and
  whether it's trusted. Not one in Recently Deleted, nor one installing until it's starting.
  """
  def running(owner) do
    {records, _automatic} = read(owner)

    for {name, %{"state" => state, "version" => number} = record} <- records,
        state != "recently_deleted" and is_integer(number),
        into: %{},
        do: {name, {record["plugin"], number, record["trusted"] != false}}
  end

  @doc """
  Moves the record of the plugin `name` for `owner` by `event`, as `move/3` says, and gives the record it's at
  now, `nil` for none, or `{:error, why}` where the table hasn't that move. The plugin's artifact is told which
  version they have.
  """
  def change(owner, name, event) do
    changed =
      Artifact.update(%Artifact{prefix: "#{owner}/"}, @blob, fn was ->
        kept = if was, do: JSON.decode!(was), else: %{"plugins" => elem(read(owner), 0)}
        records = kept["plugins"] || %{}

        case move(records[name], event, {owner, name}) do
          {:ok, nil} -> JSON.encode!(Map.put(kept, "plugins", Map.delete(records, name)))
          {:ok, record} -> JSON.encode!(Map.put(kept, "plugins", Map.put(records, name, record)))
          {:error, why} -> throw({__MODULE__, why})
        end
      end)

    record = JSON.decode!(changed)["plugins"][name]
    if record, do: marked(owner, record)
    record
  catch
    {__MODULE__, why} -> {:error, why}
  end

  @doc """
  Changes `owner`'s records as `change` says of them, by name, for what isn't a move in any plugin's life: a plugin
  renamed is the same plugin, by another name, for everyone who has it (`OneCore.Plugins.Rename`). Each plugin's
  artifact is told which version they have.
  """
  def edit(owner, change) do
    changed =
      Artifact.update(%Artifact{prefix: "#{owner}/"}, @blob, fn was ->
        kept = if was, do: JSON.decode!(was), else: %{"plugins" => elem(read(owner), 0)}
        JSON.encode!(Map.put(kept, "plugins", change.(kept["plugins"] || %{})))
      end)

    for {_name, record} <- JSON.decode!(changed)["plugins"], do: marked(owner, record)

    :ok
  end

  @doc "Turns `owner`'s automatic updates on or off."
  def automatic(owner, on) do
    Artifact.update(%Artifact{prefix: "#{owner}/"}, @blob, fn was ->
      kept = if was, do: JSON.decode!(was), else: %{"plugins" => elem(read(owner), 0)}
      JSON.encode!(Map.put(kept, "automatic", on))
    end)

    :ok
  end

  # The plugin's artifact says the version `owner` has, so its history keeps that version's files, and whether it's in
  # their Recently Deleted.
  defp marked(owner, %{"plugin" => plugin, "version" => number} = record) when is_integer(number) do
    blob = "#{plugin}/installed/#{owner}"
    data = "#{number}#{if record["state"] == "recently_deleted", do: " deleted"}"

    case Bucket.read(blob) do
      {:ok, ^data, _tag} -> :ok
      {:ok, _was, tag} -> Bucket.replace(blob, data, tag)
      :absent -> Bucket.write(blob, data)
    end
  end

  defp marked(_owner, _record), do: :ok

  @doc "Says the plugin `artifact` is no longer installed for `owner`, unless they have it by another name."
  def unmarked(artifact, owner) do
    {records, _automatic} = read(owner)
    if not Enum.any?(records, fn {_name, record} -> record["plugin"] == artifact end),
      do: Bucket.remove("#{artifact}/installed/#{owner}")
  end

  @doc """
  The transition table: where a record goes on `event`, from where it is (`nil` for not installed). `{:ok, record}`,
  `{:ok, nil}` for not installed any more, or `{:error, why}` for a move that isn't in it. `at` is whose and which
  plugin, for what it says.

  Events, as the lifecycle names them: `{:install, plugin, trusted}` (Install, or Try Again on one that failed to
  install), `{:stage, stage}`, `{:start, number, commit}` (the version it's starting, which runs from then on), `:done`,
  `{:fail, why}`, `:retry` (Try Again), `:dismiss`, `:uninstall`, `:restore` and `:purge` (30 days on); as updates
  do: `{:available, update}`, `{:update, approve}` (Update, or Approve where `approve`), `:decline` (Not now);
  `{:trust, trusted}`; and two that move its version without updating: `{:deployed, plugin, number, trusted, commit}` where
  whoever it's installed for deployed it, which is theirs at once, and `{:follow, number}` for a plugin the server
  ships, which updates with the server.
  """
  def move(record, event, at)

  def move(nil, {:install, plugin, trusted}, _at),
    do: {:ok, %{"plugin" => plugin, "version" => nil, "trusted" => trusted} |> moved("installing", "fetching")}

  def move(%{"state" => "failed", "version" => nil} = record, {:install, plugin, trusted}, _at),
    do: {:ok, %{record | "plugin" => plugin, "trusted" => trusted} |> moved("installing", "fetching")}

  def move(%{"state" => state} = record, {:stage, stage}, _at) when state in @transient and stage in ~w(fetching building),
    do: {:ok, moved(record, state, stage)}

  # From starting, the version it starts is the one that runs, and a failure goes back to what ran before. `commit`
  # is the one at its git source it was deployed from, if it was, which an update's commits are counted from.
  def move(%{"state" => state} = record, {:start, number, commit}, _at) when state in @transient do
    was = Map.take(record, ["version", "trusted", "commit"])
    trusted = get_in(record, ["target", "trusted"])
    record = record |> Map.put("version", number || record["version"]) |> Map.put("commit", commit) |> drop_nil()
    record = if is_boolean(trusted), do: %{record | "trusted" => trusted}, else: record
    {:ok, record |> moved(state, "starting") |> Map.put("was", was)}
  end

  def move(%{"state" => state, "stage" => "starting"} = record, :done, _at) when state in @transient do
    update = record["update"]
    later = update != nil and newer?(update, record) and newer?(update, record["target"] || %{})
    {:ok, record |> Map.put("update", if(later, do: Map.delete(update, "why"))) |> moved("installed")}
  end

  def move(%{"state" => state} = record, {:fail, why}, _at) when state in @transient do
    record = if was = record["was"], do: record |> Map.drop(["version", "trusted", "commit", "was"]) |> Map.merge(was), else: record

    record =
      case record do
        # An update that failed is still there to take, saying why; switching trust isn't an update.
        %{"update" => %{} = update, "target" => target} when is_map_key(target, "version") or is_map_key(target, "commit") ->
          %{record | "update" => Map.put(update, "why", why)}
        _ -> record
      end

    {:ok, record |> moved("failed") |> Map.put("why", why)}
  end

  def move(%{"state" => "failed", "version" => nil} = record, :retry, _at), do: {:ok, moved(record, "installing", "fetching")}
  def move(%{"state" => "failed", "target" => %{}} = record, :retry, _at), do: {:ok, moved(record, "updating", "fetching")}
  def move(%{"state" => "failed", "version" => nil}, :dismiss, _at), do: {:ok, nil}
  def move(%{"state" => "failed"} = record, :dismiss, _at), do: {:ok, moved(record, "installed")}

  def move(%{"state" => "installed"} = record, :uninstall, _at),
    do: {:ok, record |> moved("recently_deleted") |> Map.put("deleted", now())}

  def move(%{"state" => "recently_deleted"} = record, :restore, _at),
    do: {:ok, record |> Map.delete("deleted") |> moved("installed")}

  def move(%{"state" => "recently_deleted"}, :purge, _at), do: {:ok, nil}

  # A newer version that appears is one to take while it runs, whatever else it's doing; one no newer than what it
  # has, or is offered already, changes nothing.
  def move(%{"state" => state} = record, {:available, update}, _at) when state in @running do
    cond do
      not newer?(update, record) -> {:ok, record}
      record["update"] != nil and not newer?(update, record["update"]) -> {:ok, record}
      update["permissions"] not in [nil, []] -> {:ok, Map.put(record, "update", Map.put(update, "state", "needs_approval"))}
      true -> {:ok, Map.put(record, "update", Map.put(update, "state", "available"))}
    end
  end

  def move(%{"state" => "installed", "update" => %{"state" => "available"} = update} = record, {:update, approve}, _at) do
    if update["permissions"] not in [nil, []] and approve != true,
      do: {:ok, %{record | "update" => %{update | "state" => "needs_approval"}}},
      else: {:ok, updating(record, update, approve == true)}
  end

  def move(%{"state" => "installed", "update" => %{"state" => "needs_approval"} = update} = record, {:update, true}, _at),
    do: {:ok, updating(record, update, true)}

  def move(%{"state" => "installed", "update" => %{"state" => "needs_approval"} = update} = record, :decline, _at),
    do: {:ok, %{record | "update" => %{update | "state" => "available"}}}

  def move(%{"state" => "installed"} = record, {:trust, trusted}, _at) when is_boolean(trusted) do
    if trusted == (record["trusted"] != false),
      do: {:ok, record},
      else: {:ok, record |> Map.put("target", %{"trusted" => trusted}) |> moved("updating", "building")}
  end

  def move(record, {:deployed, plugin, number, trusted, commit}, _at)
      when record == nil or :erlang.map_get("state", record) in ["installed", "failed"] do
    update = record && record["update"]
    later = update != nil and newer?(update, %{"version" => number}) and not is_map_key(update, "commit")
    record = plugin |> record(number, trusted) |> Map.put("commit", commit) |> drop_nil()
    {:ok, if(later, do: Map.put(record, "update", update), else: record)}
  end

  def move(%{"state" => state, "version" => was} = record, {:follow, number}, _at) when state in @running do
    if is_integer(was) and number > was, do: {:ok, %{record | "version" => number}}, else: {:ok, record}
  end

  def move(record, event, {owner, name}), do: {:error, refused(record, event, name, owner)}

  # An update approved takes a version that asks for more than the one running, which is checked again once it's
  # built where only then is that known.
  defp updating(record, update, approved) do
    target = %{"version" => update["version"], "commit" => update["commit"], "trusted" => record["trusted"] != false}
    target = if approved, do: Map.put(target, "approved", true), else: target
    record |> Map.put("target", drop_nil(target)) |> moved("updating", "fetching")
  end

  defp drop_nil(map), do: Map.reject(map, &is_nil(elem(&1, 1)))

  # A record in `state`, at `stage` within it if it has one, since now.
  defp moved(record, state, stage \\ nil) do
    record
    |> Map.merge(%{"state" => state, "at" => now()})
    |> Map.drop(["stage", "why"])
    |> then(&if(stage, do: Map.put(&1, "stage", stage), else: &1))
    |> then(&if(state in @transient or state == "failed", do: &1, else: Map.drop(&1, ["target", "was"])))
  end

  # Whether what `update` offers is newer than what `than` has or offers: a version by its number, or a commit at the
  # plugin's git source, which comes after the version it was found from.
  defp newer?(%{"version" => number}, %{"version" => than}) when is_integer(number) and is_integer(than), do: number > than
  defp newer?(%{"commit" => commit}, %{"commit" => commit}), do: false
  defp newer?(%{"commit" => commit}, _than) when is_binary(commit), do: true
  defp newer?(%{"version" => number}, _than) when is_integer(number), do: true
  defp newer?(_update, _than), do: false

  # What a refused move says: where it is, and what it can do from there.
  defp refused(nil, _event, name, owner), do: "#{name} isn't installed for #{owner}."

  defp refused(%{"state" => "recently_deleted"}, _event, name, owner),
    do: "#{name} is in Recently Deleted for #{owner}. Restore it (plugins_restore) to use it again."

  defp refused(%{"state" => state, "stage" => stage}, _event, name, _owner) when state in @transient,
    do: "#{name} is #{state} (#{stage}). Wait until it's done."

  defp refused(%{"state" => "failed", "why" => why}, _event, name, _owner),
    do: "#{name} failed: #{why}\n\nTry again (plugins_retry) or dismiss it (plugins_dismiss) first."

  defp refused(%{"state" => "installed"} = record, event, name, owner) do
    case {event, record["update"]} do
      {{:install, _, _}, _} -> "#{name} is installed for #{owner} already."
      {{:update, _}, nil} -> "#{name} is up to date."
      {:decline, _} -> "#{name} has no update waiting for approval."
      _ -> "#{name} is installed for #{owner}, and that doesn't apply to it."
    end
  end

  defp refused(_record, _event, name, owner), do: "That doesn't apply to #{name} for #{owner} as it is."

  defp now, do: System.system_time(:millisecond)

  @doc "Who the plugin `artifact` is installed for, as its artifact says, besides whoever's it is: in their Recently Deleted too."
  def installed_for(artifact) do
    for blob <- Bucket.list("#{artifact}/installed/"), do: String.replace_prefix(blob, "#{artifact}/installed/", "")
  end

  @doc "Who the plugin `artifact` is in use for, as its artifact says: installed for them, and not in their Recently Deleted."
  def in_use_for(artifact) do
    blobs = Bucket.list("#{artifact}/installed/")

    for {blob, {:ok, marked, _tag}} <- Enum.zip(blobs, Bucket.read_all(blobs)), not String.ends_with?(marked, " deleted"),
        do: String.replace_prefix(blob, "#{artifact}/installed/", "")
  end

  @doc "The versions of the plugin `artifact` that someone has installed, as its artifact says."
  def versions_installed(artifact) do
    blobs = Bucket.list("#{artifact}/installed/")

    for {:ok, marked, _tag} <- Bucket.read_all(blobs), {number, _deleted} <- [Integer.parse(marked)], uniq: true,
        do: number
  end

  @doc "Whether `name` is one a plugin is installed by: `:ok`, or `{:error, why}`."
  def named(name) do
    cond do
      not (is_binary(name) and name =~ ~r/^[a-z][a-z0-9-]*$/) ->
        {:error, "A plugin's name is lowercase letters, digits and hyphens, starting with a letter"}

      # A client finds the UI library where it finds a plugin's files, by this name (`OneCore.MCP`).
      name == "ui" ->
        {:error, "ui is the UI library's, which is One Core's own: name the plugin something else"}

      true ->
        :ok
    end
  end
end
