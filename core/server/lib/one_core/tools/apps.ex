defmodule OneCore.Tools.Apps do
  @moduledoc "One Core's own tools that deploy, install, update and remove apps, and read and upload to them, as `OneCore.Tools` calls them, with their definitions in `OneCore.Tools.Definitions`."
  import OneCore.Tools.Helpers
  alias OneCore.{Artifact, Bucket, Call, Hooks, Apps, User}
  alias OneCore.Apps.{Installed, Calls, Deploy, Lifecycle, Requires, Store, Update, Version}

  # An app is whoever's deployed it. Deploying over one is changing it, and a new one is installed for whoever it's
  # deployed for. Either way it's loaded where it will run.
  # An untrusted app runs on any server acting for whoever it's for, the network's among them.
  @doc "Runs its tool `name` with `inputs`, as `OneCore.Tools.call/5` hands it on."
  # What's live is a commit: changes not committed are deployed only to a network being built (`:development`), as a
  # local network is, where they're tried before they're committed (19 Nov).
  def run("deploy", %{"name" => name, "files" => files} = inputs, {_actor, subject} = principal, forward, _request) do
    if match?(%{"dirty" => true}, inputs["from"]) and not Application.get_env(:one_core, :development, false),
      do: raise("#{name} wasn't deployed: its files have changes not committed. Commit them, push, then deploy.")

    app = Apps.app(subject, name)
    trusted = Map.get(inputs, "trusted", app == nil or Store.trusted?(app))
    runs(subject, trusted, forward)

    case app do
      nil -> reaches!(principal, subject, "installer")
      # Whether it runs trusted, on its servers with their full access, is its owner's to switch, as `apps_trust` has it.
      app -> reaches!(principal, app, if(trusted == Store.trusted?(app), do: "editor", else: "owner"))
    end

    if app == nil and inputs["new"] != true, do: elsewhere!(principal, name)

    case Deploy.deploy(subject, name, Store.received(files), Map.get(inputs, "from", :stored), trusted) do
      {:ok, number, warnings} ->
        %{
          name: name,
          version: number,
          trusted: trusted,
          requires: requires(Apps.running(subject)[name]),
          warnings: if(warnings == "", do: nil, else: warnings),
          listing: Version.listing(Apps.running(subject)[name])
        }

      {:error, why, details} ->
        throw({Call, {:refused, why, details}})

      {:error, why} ->
        raise why
    end
  end

  def run("code", %{"name" => name}, {_actor, subject} = principal, _pass, _request) do
    case Apps.running(subject) do
      %{^name => version} ->
        opens!(principal, version.artifact)
        %{files: Store.sent(Store.source(version.artifact, version.code))}

      _ ->
        raise "There's no app named #{name}"
    end
  end

  def run("apps", _inputs, {_actor, subject}, _pass, _request) do
    owner = User.owner(subject)

    installed =
      for at <- Apps.chain(subject),
          {name, record} <- Enum.sort(Installed.read(at)),
          at == owner or record["state"] in ["installing", "installed"],
          do: {at, name, record}

    %{for: owner, apps: listed(installed)}
  end

  def run("install", %{"app" => app} = inputs, {actor, subject} = principal, forward, _request) do
    name = Map.get(inputs, "name", Path.basename(app))
    runs(subject, Store.trusted?(app), forward)
    opens!(principal, app)
    reaches!(principal, subject, "installer")
    moved!(subject, name, Lifecycle.install(subject, name, app, actor))
  end

  def run("uninstall", %{"name" => name} = inputs, {_actor, subject} = principal, forward, _request) do
    at(subject, forward)
    reaches!(principal, subject, "owner")
    moved!(subject, name, Lifecycle.uninstall(subject, name, principal, inputs["confirm"] == true))
  end

  def run("apps_update", %{"name" => name}, {_actor, subject} = principal, forward, _request) do
    app = updates!(principal, subject, name, forward)

    case Lifecycle.update(app) do
      {:error, why} ->
        raise why

      %{"state" => "failed", "why" => why} ->
        raise "#{why}\n\n#{name}'s version before keeps running. Try again (apps_retry)."

      _ ->
        moved!(subject, name, Installed.read(subject)[name])
    end
  end

  # Try Again: installing it, where that failed, or its app's update.
  def run("apps_retry", %{"name" => name}, {_actor, subject} = principal, forward, request) do
    case record!(subject, name) do
      %{"state" => "not_installed", "app" => app} ->
        runs(subject, Store.trusted?(app), forward)
        reaches!(principal, subject, "installer")
        moved!(subject, name, Lifecycle.retry(subject, name))

      _ ->
        run("apps_update", %{"name" => name}, principal, forward, request)
    end
  end

  # An app's trust is its owner's to switch, for everyone who has it, where it's deployed again.
  def run("apps_trust", %{"name" => name, "trusted" => trusted}, {_actor, subject} = principal, forward, _request) do
    app = record!(subject, name)["app"]
    whose = Store.whose(app)
    runs(whose, trusted, forward)
    reaches!(principal, app, "owner")
    with {:error, why} <- Lifecycle.trust(app, trusted), do: raise(why)
    moved!(subject, name, Installed.read(subject)[name])
  end

  def run("apps_check", _inputs, {_actor, subject}, _pass, _request) do
    owner = User.owner(subject) || "network"
    records = Installed.read(owner)

    apps = for {_name, %{"app" => app, "state" => "installed"}} <- records, Lifecycle.takes?(app), do: app
    Enum.each(Enum.uniq(apps), &Lifecycle.check/1)

    %{for: owner, apps: listed(for {name, record} <- Enum.sort(Installed.read(owner)), do: {owner, name, record})}
  end

  # A file sent this way lands in the app's own artifact, whoever's its calls for them reach (`Calls.whose/3`), under a
  # name nobody could guess.
  # A member uploads for a group or the network only as an editor of what the app made for them, as a tool that
  # changes something is called (`OneCore.Apps.Calls`).
  def run("upload", %{"app" => app}, {actor, subject} = principal, forward, _request) do
    with %{^app => version} <- Apps.running(subject),
         {installed, ^app} <- Apps.installed(subject, version.artifact) do
      if actor != nil and actor != subject, do: reaches!(principal, "#{subject}/made/#{app}", "editor")
      at(installed, forward)
      name = "uploads/" <> Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)
      own = %Artifact{prefix: "#{Calls.whose(installed, version, subject)}/own/#{app}/"}
      Map.put(Artifact.link(own, name, :write), :upload, name)
    else
      _ -> raise "There's no app named #{app}"
    end
  end

  defp requires(nil), do: %{}

  defp requires(version), do: version.requires

  # Has a server that runs an app installed for `owner`, `trusted` or not, run this call, if this one doesn't, since
  # what it deploys or installs runs there (`OneCore.Apps.runs?/2`): a trusted app on a server that runs their
  # apps, and an untrusted one on any acting for them.
  defp runs(owner, trusted, forward) do
    cond do
      Apps.runs?(owner, trusted) ->
        true

      not trusted ->
        forward.(owner)

      Apps.servers(owner) == [] ->
        raise "An app for #{owner} needs a server of their own, since its code runs there and never on the network's. " <>
                "Set one up in Devices, ask whoever may install apps for everyone to install it for everyone, " <>
                "or deploy it untrusted (trusted: false), to run in a sandbox, which any of the network's servers " <>
                "runs for anyone."

      true ->
        forward.({:running, owner})
    end
  end

  # Each install as `apps` lists it, with its app's version, trust, update and where it came from, read at once.
  defp listed(installed) do
    apps = for {_at, _name, record} <- installed, do: record["app"]
    latest = Bucket.read_all(Enum.map(apps, &(&1 <> "/latest")))
    updates = Bucket.read_all(Enum.map(apps, &(&1 <> "/update")))

    for {{at, name, record}, latest, update} <- Enum.zip([installed, latest, updates]) do
      about = with {:ok, json, _tag} <- latest, do: JSON.decode!(json), else: (_ -> %{})
      update = with {:ok, json, _tag} when json != "" <- update, do: JSON.decode!(json), else: (_ -> nil)

      at
      |> moved(name, record, about, update)
      |> Map.merge(%{source: about["source"], from: about["from"], required_by: Requires.required_by(at, name)})
    end
  end

  # Where an install is in its life for `owner`, with its app's version, trust and update, as a tool that moves
  # either answers: `record` is its record, or `nil` once it's gone.
  defp moved(owner, name, record) do
    app = record && record["app"]
    moved(owner, name, record, nil, app && Update.read(app))
  end

  defp moved(owner, name, record, about, update) do
    record = record || %{}
    app = record["app"]
    about = about || if(app, do: Store.about(app), else: %{})

    %{
      name: name,
      for: owner,
      app: app,
      state: record["state"],
      stage: record["stage"],
      why: record["why"],
      deleted: record["deleted"],
      version: about["number"],
      trusted: if(app, do: Store.trusted?(about)),
      update:
        update &&
          %{
            state: update["state"],
            stage: update["stage"],
            commit: update["commit"],
            commits: update["commits"] || [],
            why: update["why"]
          },
      requires: requires(Apps.running(owner)[name]),
      platform: platform(owner, name, app)
    }
  end

  # Whether an app's version runs on this server's platform: one running here does, and only one that doesn't is
  # read from what it said of itself.
  defp platform(_owner, _name, nil), do: nil

  defp platform(owner, name, app) do
    if is_map_key(Apps.running(owner), name), do: %{state: :runs, why: nil}, else: Store.platform(name, app)
  end

  # What failed is an error for whoever asked, as it is where it was asked: the record says so too, until it's tried
  # again.
  defp moved!(_owner, _name, {:error, why}), do: raise(why)

  defp moved!(_owner, name, %{"state" => "not_installed", "why" => why}),
    do: raise("#{why}\n\n#{name} wasn't installed. Try again (apps_retry).")

  defp moved!(owner, name, record) do
    moved = moved(owner, name, record)
    if also = record["also"], do: Map.put(moved, :also, also), else: moved
  end

  defp record!(owner, name), do: Installed.read(owner)[name] || raise("#{name} isn't installed for #{owner}.")

  # The app `name` is for `owner`, whose update whoever asked may take: someone who may install for whoever's the app
  # is, or an editor of it, since its update is for everyone who has it, not only for whoever installed this copy.
  # Its update is deployed where it is: on a server of whoever the app is, which runs their apps where it's trusted.
  defp updates!(principal, owner, name, forward) do
    app = record!(owner, name)["app"]
    whose = Store.whose(app)
    runs(whose, Store.trusted?(app), forward)

    if reaches(principal, whose, "installer") != :ok,
      do: reaches!(principal, app, "editor")

    app
  end

  # Raises where an app named `name` is installed for someone else whoever asked is in, whose apps they find
  # too: a new one would answer first for whoever it's deployed for, empty, as a second copy.
  defp elsewhere!({actor, subject}, name) do
    member = &(Hooks.validate("reach", %{"who" => actor, "relation" => "member", "artifact" => &1}, {actor, &1}) == :ok)
    groups = for {"groups/" <> _ = group, %{^name => _}} <- Apps.installed(), member.(group), do: group
    others = for owner <- Enum.uniq([User.owner(actor), "network" | groups]), owner not in [nil, subject], do: owner

    case for(owner <- others, Apps.app(owner, name), do: owner) do
      [] ->
        :ok

      [owner | _] ->
        to = if owner == actor, do: "for yourself (no ONE_SUBJECT)", else: "for #{owner} (ONE_SUBJECT=#{owner})"

        raise "#{name} is installed for #{owner}, so this would make a second, empty #{name} for #{subject} that answers " <>
                "first. To update the one there is, deploy it #{to}; to make a second one, say `new: true` (`--new`)."
    end
  end

  # Raises unless whoever asked may read an app's code and add it: a `viewer` of it, or a member of a group or
  # the network that it's whoever's or installed for, as the apps that validate `reach` say. That's VISION's "what's
  # made is open to everyone inside once it's in use": what only its author has is theirs and whoever they share
  # it with, and taking it away from a group takes it away from them. Only its code is opened: what it keeps for
  # anyone is never the app's artifact, and is reached as ever.
  defp opens!(principal, app), do: with({:refuse, why} <- Lifecycle.opens(principal, app), do: raise(why))
end
