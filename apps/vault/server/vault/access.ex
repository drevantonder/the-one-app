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

  @doc """
  Raises unless the call is its person's own, made on a screen of theirs, the One App's or a browser's, where they
  read what they're doing: what widens who may use a secret, or where it's sent, is theirs alone to do. An agent, `one`
  in a terminal or an app's code asks with `request`, which they answer there.

  Which screen it's on is sign-in's to say, from the device the call came from as that's proved, never from what the
  call says of itself alone (`SignIn.screen?/1`).
  """
  def person!(call, what) do
    on_screen = call.caller == nil and OneCore.Call.app(call, "sign-in").screen?(call)
    on_screen || raise "#{what} is its person's to do, in Vault on their device, where an agent's vault_request asks them."
  end

  @doc "Whoever an app is called by its name, as its person picks it: its artifact, or `\"agents\"`."
  def who!(_call, "agents"), do: "agents"
  def who!(call, app), do: Apps.app(call.subject, app) || Apps.app("network", app) || raise("There's no app named #{app}.")

  @doc """
  Gives `who` (`"agents"` or an app's artifact) access to an item, or with `on` false takes it away, and keeps the
  app's tuple on the Vault in step. The tuple moves first, so where the permissions app refuses, the item is as it
  was. Gives the item.
  """
  def switch(call, db, item, who, on) do
    if who != "agents" do
      if on, do: in_step(call, db, who, true), else: unshare_unless_used(call, db, item, who)
    end

    Store.change!(db, item, &Map.update!(&1, "access", fn access -> if on, do: Enum.uniq(access ++ [who]), else: access -- [who] end))
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
      for who <- gone, do: unshare_unless_used(call, db, item, who)
      Store.change!(db, item, &Map.update!(&1, "access", fn access -> access -- gone end))
    end
  end

  @doc "Shares someone's Vault with an app as it gets access to an item: the tuple it reaches it by."
  def in_step(call, db, app, true), do: share(call, "share", Artifact.name(db), app)

  # An app's tuple is taken back with its access to its last item, by the records as kept, which `prune/4` reads in turn.
  defp unshare_unless_used(call, db, item, app) do
    others = Enum.filter(Store.everything(call, db), &(&1["id"] != item["id"]))
    if not Enum.any?(others, &(app in &1["access"])), do: share(call, "unshare", Artifact.name(db), app)
  end

  @doc "Shares someone's Vault with the app making a call, before it has an item there: the tuple it reaches it by."
  def reach(call), do: share(call, "share", Store.theirs(call), call.app_artifact)

  # A relation tuple is the permissions app's to write, as whoever asked.
  defp share(call, tool, artifact, who) do
    inputs = %{"artifact" => artifact, "relation" => "editor", "who" => who}

    case Calls.call("permissions", tool, inputs, {call.actor, call.subject}) do
      {:ok, _said} -> :ok
      {:forwarded, %{"isError" => true, "content" => [%{"text" => why} | _]}} -> raise(why)
      {:forwarded, _result} -> :ok
      {:error, why} -> raise(why)
    end
  end
end
