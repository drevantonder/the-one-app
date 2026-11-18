defmodule Vault.Access do
  @moduledoc """
  Who may use an item: one list per item, `access`, of `"agents"` (its person's agents, with `fetch`) and the
  artifacts of the apps switched on for it. Its websites are apart from it (`Vault.Items`).

  An app with access to any item reaches its person's Vault as an editor, which a relation tuple says; this module is
  the one place that list changes, so the tuple is shared with an app as it gets access to its first item, and
  taken back as it loses access to its last. An app that's no longer installed loses its access where an item is read
  (`prune/4`), so the list never names one that isn't there.
  """

  alias OneCore.{Apps, Artifact}
  alias OneCore.Apps.Calls
  alias Vault.Store

  @doc "The item, if `by` may use it: its person, an app with access, or agents where its person let them."
  def may!(item, :person), do: item

  def may!(item, :agents) do
    if "agents" not in item["access"],
      do: raise("#{item["name"]} isn't one agents may use. Ask its person, with vault_request.")

    item
  end

  def may!(item, {:app, app}) do
    if app not in item["access"],
      do: raise("#{item["name"]} isn't switched on for #{Path.basename(app)}. Its person switches it on, in the Vault.")

    item
  end

  @doc "Whoever an app is called by its name, as its person picks it: its artifact, or `\"agents\"`."
  def who!(_call, "agents"), do: "agents"
  def who!(call, app), do: Apps.app(call.subject, app) || Apps.app("network", app) || raise("There's no app named #{app}.")

  @doc """
  Gives `who` (`"agents"` or an app's artifact) access to an item, or with `on` false takes it away, and keeps the
  app's tuple on the Vault in step. Gives the item.
  """
  def switch(call, db, item, who, on) do
    changed = Store.change!(db, item, &Map.update!(&1, "access", fn access -> if on, do: Enum.uniq(access ++ [who]), else: access -- [who] end))
    if who != "agents", do: in_step(call, db, who, on)
    changed
  end

  @doc """
  The item without access for an app that's no longer installed (`names`, by artifact, as `Vault.Items.names/1`), kept
  so, and its tuple on the Vault taken back: where an item is read, as it's found.
  """
  def prune(call, db, item, names) do
    gone = for who <- item["access"], who != "agents", not is_map_key(names, who), do: who

    if gone == [] do
      item
    else
      pruned = Store.change!(db, item, &Map.update!(&1, "access", fn access -> access -- gone end))
      for who <- gone, do: share(call, "unshare", Artifact.name(db), who)
      pruned
    end
  end

  @doc "Keeps an app's tuple on someone's Vault as its access says: shared while it has access to an item."
  def in_step(call, db, app, true), do: share(call, "share", Artifact.name(db), app)

  def in_step(call, db, app, false) do
    if not Enum.any?(Vault.Items.all(call, db), &(app in &1["access"])), do: share(call, "unshare", Artifact.name(db), app)
    :ok
  end

  @doc "Shares someone's Vault with the app making a call, before it has an item there: the tuple it reaches it by."
  def reach(call), do: share(call, "share", Store.theirs(call), call.who)

  # A relation tuple is the permissions app's to write, as whoever asked.
  defp share(call, tool, artifact, who) do
    inputs = %{"artifact" => artifact, "relation" => "editor", "who" => who}

    case Calls.call("permissions", tool, inputs, {call.actor, call.subject}) do
      {:ok, _said} -> :ok
      {:forwarded, %{"isError" => true, "content" => [%{"text" => why} | _]}} -> if(tool == "share", do: raise(why), else: :ok)
      {:forwarded, _result} -> :ok
      {:error, why} -> if(tool == "share", do: raise(why), else: :ok)
    end
  end
end
