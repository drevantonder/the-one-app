defmodule OneCore.Plugins.Deploy do
  @moduledoc """
  Deploying a plugin: a new one, or a new version of one. Its app, if it has one, is built first (`OneCore.App`),
  its Elixir is compiled and loaded (`OneCore.Plugins.Version`), and it's kept in the bucket
  (`OneCore.Plugins.Store`) and runs at once.

  A server deploys each plugin it ships with for the network: its `:bundled` setting, each one's folder by its name.
  One that says it's essential (`OneCore.Plugin`) is installed for the network; the rest are only available, in the
  App Store for anyone to add, until someone does. Deploying or adding a plugin installs the ones it uses that the
  server ships with for whoever it's installed for, where they aren't there.
  """
  require Logger
  alias OneCore.{App, Bucket, Builds, Folder, Sandbox, Server, Servers}
  alias OneCore.Plugins
  alias OneCore.Plugins.{Installed, Calls, Store, Version}

  @network "network"
  @looks_after :timer.minutes(5)
  # What each owner has deployed, as the network keeps it: `deployed/<owner>/<name>`.
  @deployed "network/deployed/"
  @outputs "Each tool gives its outputs beside its inputs: a JSON Schema of the object it answers, which the skill " <>
             "building-plugins says how to write."

  @doc """
  Deploys a plugin for `owner` from its files by their paths: the plugin that `name` is for them is updated, and
  if there's none it's a new plugin of theirs, installed for them. Its app, if it has one, is built first
  (`OneCore.App`). `from` is the git repo its files came from, as `OneCore.Plugins.Store.keep/6` takes it, and
  `sandbox` whether it runs in sandbox mode (`OneCore.Sandbox`), each as the version before had it if `:stored`. Gives
  its version's number and the compiler's warnings.
  """
  def deploy(owner, name, files, from \\ :stored, sandbox \\ :stored),
    do: deploy(owner, name, files, from, sandbox, fn _version -> true end)

  @doc "Deploys as `deploy/5` does, installing it for `owner` only where `install?` says so of the version."
  def deploy(owner, name, files, from, sandbox, install?) do
    artifact = Plugins.plugin(owner, name) || "#{owner}/plugins/#{name}"
    sandbox = if sandbox == :stored, do: Store.about(artifact)["sandbox"] == true, else: sandbox == true

    with :ok <- check(name, files),
         {:ok, built} <- built(owner, name, files, sandbox),
         {:ok, version, warnings} <- loaded(artifact, built, sandbox) do
      version = %{version | artifact: artifact}
      install = install?.(version)

      with :ok <- outputs(version),
           :ok <- if(install, do: fits(owner, name, version), else: :ok),
           {:ok, warnings} <- if(sandbox, do: Sandbox.check(owner, version, built), else: {:ok, warnings}),
           {:ok, number} <- Store.keep(artifact, name, built, files, from, sandbox),
           :ok <- Store.describe(version),
           :ok <- if(install, do: Installed.note(owner, &{:ok, Map.put(&1, name, artifact)}), else: :ok) do
        listed(artifact, version)
        if install, do: Plugins.reload(owner, [{artifact, number, {:ok, version}}]), else: Version.unload(version)
        Server.notify_servers(Enum.uniq([owner | Installed.installed_for(artifact)]))
        {:ok, number, warnings}
      else
        {:error, why} ->
          Version.unload(version)
          {:error, why}
      end
    end
  end

  # Every tool of a version deployed now says what it answers, which every answer is checked against: a version
  # deployed before tools said it keeps running, and the next one deployed says it.
  defp outputs(version) do
    case for(tool <- version.tools, tool[:outputs] == nil, do: tool.name) do
      [] ->
        :ok

      [one] ->
        {:error, "#{one} doesn't say what it answers. #{@outputs}"}

      missing ->
        {:error, "#{Enum.join(missing, ", ")} don't say what they answer. #{@outputs}"}
    end
  end

  # A version's code, compiled and loaded here, or in sandbox mode only described, by its own VM.
  defp loaded(artifact, built, false), do: Version.load(Path.basename(artifact), built)

  defp loaded(artifact, built, true) do
    with {:ok, version} <- Sandbox.describe(artifact, built), do: {:ok, version, ""}
  end

  @doc """
  Every plugin anyone has deployed, as its latest version has it: `[%{plugin: artifact, title: ..., description:
  its skill's, subtitle: ..., about: ..., icon: ..., apps: [...], tools: [names], permissions: [...], sandbox:
  ...}]`, what a person reads of it as `Version.listing/1` gives it, its permissions as
  `permissions/0` gives them
  and whether it runs in sandbox mode. It's what lists every app there is, whether or not it's on anyone's page
  now. One that was removed, by the plugin that keeps what's deleted, has no latest version, and isn't listed.

  It's two trips to the bucket with a request for each plugin, and changes only when someone deploys, so it's kept:
  read again behind a request once it's five minutes old, as `OneCore.Plugins` looks at what's installed, and
  forgotten when this server deploys or is told to look again (`forget/0`).
  """
  def deployed do
    now = System.monotonic_time(:millisecond)

    case :ets.lookup(__MODULE__, :deployed) do
      [{:deployed, deployed, at}] ->
        if at <= now - @looks_after and :ets.insert_new(__MODULE__, {:reading}),
          do: Task.start(fn -> try do: read_deployed(), after: :ets.delete(__MODULE__, :reading) end)

        deployed

      [] ->
        read_deployed()
    end
  end

  @doc "Forgets what was kept of what everyone has deployed, which has changed: the next ask reads it again."
  def forget, do: :ets.delete(__MODULE__, :deployed)

  defp read_deployed do
    blobs = Bucket.list(@deployed)
    named = for blob <- blobs, do: Regex.run(~r{^#{@deployed}(.+)/([^/]+)$}, blob, capture: :all_but_first)
    latest = for [owner, name] <- named, do: "#{owner}/plugins/#{name}/latest"
    {descriptions, latest} = Enum.split(Bucket.read_all(blobs ++ latest), length(blobs))

    deployed =
      for {[owner, name], {:ok, description, _tag}, {:ok, _latest, _}} <- Enum.zip([named, descriptions, latest]) do
        description = JSON.decode!(description)

        apps =
          for app <- description["apps"],
              do: %{name: app["name"], page: app["page"], icon: app["icon"], category: app["category"]}

        %{
          plugin: "#{owner}/plugins/#{name}",
          title: description["title"] || name,
          description: description["description"] || "",
          subtitle: description["subtitle"],
          about: description["about"],
          icon: description["icon"],
          apps: apps,
          tools: description["tools"] || [],
          permissions: description["permissions"] || [],
          sandbox: description["sandbox"] == true
        }
      end

    :ets.insert(__MODULE__, {:deployed, deployed, System.monotonic_time(:millisecond)})
    deployed
  end

  # The plugin `artifact`, with the apps this version has, among what everyone has deployed, under whose it is: a
  # person who deploys the network's plugin updates the network's.
  defp listed(artifact, version) do
    blob = "#{@deployed}#{Store.whose(artifact)}/#{Path.basename(artifact)}"

    listing =
      %{
        apps: version.apps,
        title: version.title,
        description: OneCore.Search.described(version),
        icon: version.icon,
        tools: Enum.map(version.tools, & &1.name),
        permissions: version.permissions,
        sandbox: version.sandbox
      }
      |> Map.merge(Version.listing(version))
      |> JSON.encode!()

    case Bucket.read(blob) do
      {:ok, ^listing, _tag} -> :ok
      {:ok, _was, tag} -> Bucket.replace(blob, listing, tag)
      :absent -> Bucket.write(blob, listing)
    end

    forget()
  end

  @doc """
  Deploys for the network each plugin this server ships with that the network has never had, and each that runs
  what a server once shipped but not what this one ships: so an updated server brings its plugins along. One whose
  latest version came from files no server shipped was changed on purpose, and is left as it is. Gives what
  `deploy/3` gave for each, by its name, and says each in the log and in `deploying_bundled/0` as it goes. One a
  server shipped that this one, the newest, doesn't, and that's still the repo's, isn't listed any more.

  What servers have shipped of a plugin is the plugin's blob `bundled`: the hashes of every set of files they
  deployed it from.
  """
  def deploy_bundled do
    unlist_unshipped()

    names =
      for {name, _folder} <- Enum.sort(Application.get_env(:one_core, :bundled, %{})),
          deploy_bundled?(name, Version.code(bundled(name))),
          do: name

    for name <- names, do: deploying(name, "waiting")
    for name <- names, deploy_bundled?(name, Version.code(bundled(name))), do: {name, deploy_bundled(name)}
  end

  @doc """
  Waits until this server has deployed the plugins it bundles, as it does once it's started (`deploy_bundled/0`).
  """
  def await_bundled do
    case List.keyfind(Supervisor.which_children(OneCore.Supervisor), :bundled, 0) do
      {:bundled, pid, _type, _modules} when is_pid(pid) ->
        deploying = Process.monitor(pid)
        receive do: ({:DOWN, ^deploying, _, _, _} -> :ok)

      _ ->
        :ok
    end
  end

  @doc """
  What of the plugins this server ships it's still deploying, or failed to, by name: `"waiting"`, `"building"`, or
  why it failed. Empty once every one is deployed.
  """
  def deploying_bundled, do: Agent.get(__MODULE__, & &1)

  @doc false
  def child_spec(_), do: %{id: __MODULE__, start: {Agent, :start_link, [&started/0, [name: __MODULE__]]}}

  # What's deploying, and a table of what everyone has deployed (`deployed/0`), which the Agent holds.
  defp started do
    :ets.new(__MODULE__, [:named_table, :public, read_concurrency: true])
    %{}
  end

  defp deploying(name, nil), do: Agent.update(__MODULE__, &Map.delete(&1, name))
  defp deploying(name, status), do: Agent.update(__MODULE__, &Map.put(&1, name, status))

  # Deploys one plugin this server ships for the network, and notes its files among what servers have shipped. It's
  # installed for the network if it's essential or the network has it already.
  defp deploy_bundled(name) do
    files = bundled(name)
    source = Version.code(files)
    started = System.monotonic_time(:millisecond)
    Logger.info("Shipping #{name}")
    deploying(name, "building")

    with true <- files != %{} || {:error, "There are no files to deploy it from"},
         {installed, _tag} = Installed.read(@network),
         install? = &(&1.essential or is_map_key(installed, name)),
         {:ok, number, _warnings} = deployed <- deploy(@network, name, files, nil, false, install?),
         :ok <- note_bundled("#{@network}/plugins/#{name}/bundled", source) do
      Logger.info("Shipped #{name} as version #{number} in #{System.monotonic_time(:millisecond) - started} ms")
      deploying(name, nil)
      deployed
    else
      {:error, why} = failed ->
        Logger.error("#{name} wasn't deployed: #{why}")
        deploying(name, why)
        failed
    end
  end

  defp note_bundled(blob, source) do
    case Bucket.read(blob) do
      {:ok, json, tag} ->
        sources = bundled_sources(json)

        cond do
          source in sources -> :ok
          Bucket.replace(blob, JSON.encode!([source | sources]), tag) == :ok -> :ok
          true -> note_bundled(blob, source)
        end

      :absent ->
        with :exists <- Bucket.write(blob, JSON.encode!([source])), do: note_bundled(blob, source)
    end
  end

  # Whether the plugin `name` that this server ships, from files whose hash is `source`, is to be deployed for the
  # network: it never had it, it runs what a server shipped once and that isn't this, or it isn't listed. One whose
  # latest version doesn't say what it was deployed from is from before versions said so, and was shipped too.
  defp deploy_bundled?(name, source) do
    artifact = "#{@network}/plugins/#{name}"

    case Store.latest_source(artifact) do
      nil ->
        true

      # A server that didn't ship it stopped listing it.
      ^source ->
        Bucket.size("#{@deployed}#{@network}/#{name}") == :absent

      _latest ->
        shipped?(name)
    end
  end

  # Whether the network's plugin `name` runs files a server shipped, rather than ones someone deployed on purpose.
  defp shipped?(name) do
    artifact = "#{@network}/plugins/#{name}"

    case Bucket.read(artifact <> "/bundled") do
      {:ok, json, _tag} -> Store.latest_source(artifact) in bundled_sources(json)
      :absent -> false
    end
  end

  # A plugin servers shipped that this one, the newest there is, doesn't, and that still runs what they shipped or
  # what was deployed by hand from the repo they ship from, isn't listed in the App Store any more: it was renamed
  # or taken out of the repo. One that moved to another repo is listed as it was. Whoever has it installed keeps it,
  # and deploying it lists it again.
  defp unlist_unshipped do
    ships = Application.get_env(:one_core, :bundled, %{})
    prefix = "#{@deployed}#{@network}/"
    unshipped = for blob <- Bucket.list(prefix), not is_map_key(ships, Path.basename(blob)), do: Path.basename(blob)

    if unshipped != [] and newest?() do
      repos = MapSet.new(Map.keys(ships), &Store.about("#{@network}/plugins/#{&1}")["from"]["source"])

      gone =
        for name <- unshipped, ours?(name, MapSet.delete(repos, nil)) do
          Logger.info("#{name} isn't shipped any more, so it isn't listed")
          Bucket.remove(prefix <> name)
        end

      if gone != [] do
        forget()
        Server.notify_servers([@network])
      end
    end
  end

  # Whether the network's plugin `name` is one servers shipped, and runs what they shipped or what was deployed from
  # one of `repos`.
  defp ours?(name, repos) do
    artifact = "#{@network}/plugins/#{name}"

    case Bucket.read(artifact <> "/bundled") do
      {:ok, json, _tag} ->
        about = Store.about(artifact)
        about["source"] in bundled_sources(json) or about["from"]["source"] in repos

      :absent ->
        false
    end
  end

  defp bundled_sources(json),
    do: with(sources when is_list(sources) <- JSON.decode!(json), do: sources, else: (_ -> []))

  @doc """
  Whether a plugin fits whoever it's installed for: the plugins it uses are there for them, and one with scheduled jobs
  uses a plugin that keeps them. `:ok`, or `{:error, why}`.
  """
  def fits(owner, name, version) do
    with :ok <- launcher(owner, name, version),
         :ok <- bring(owner, name, version.uses),
         do: scheduled(owner, name, version)
  end

  @doc """
  Whether a version has the launcher: the app in the category `launcher`, which every client opens first. A network
  has exactly one, installed for the network.
  """
  def launcher?(version), do: Enum.any?(version.apps, &(&1.category == "launcher"))

  @doc """
  How someone who may not install a plugin for everyone asks for it, to end a sentence: by the network's launcher,
  which takes such requests, named as it's installed, whose skill says how.
  """
  def ask_to_install do
    case for {name, version} <- Plugins.versions([@network]), launcher?(version), do: name do
      [launcher | _] -> "ask for it to be installed for everyone, by the network's launcher, #{launcher} (its skill says how)"
      [] -> "ask an owner of the network to install it for everyone"
    end
  end

  defp launcher(owner, name, version) do
    others = for {other, running} <- Plugins.versions([@network]), other != name, launcher?(running), do: other

    cond do
      not launcher?(version) ->
        :ok

      owner != @network ->
        {:error, "#{name} is a launcher, which every client opens first, so it's installed only for the network."}

      others != [] ->
        [other | _] = others

        {:error,
         "#{other} is this network's launcher, and a network has one. To replace it, deploy #{name} as #{other}'s " <>
           "new version: ONE_SUBJECT=network one deploy --name #{other} --files-from <#{name}'s folder>"}

      true ->
        :ok
    end
  end

  # A plugin's scheduled jobs are kept by a plugin it uses.
  defp scheduled(_owner, _name, %Version{jobs: []}), do: :ok

  defp scheduled(owner, name, version) do
    with {:ok, used} <- Plugins.used(owner, version.uses),
         [_ | _] <- Calls.keepers(used) do
      :ok
    else
      _ -> {:error, "#{name} has scheduled jobs, so it uses a plugin that keeps its schedule: one with schedule/3"}
    end
  end

  # A plugin's app is built against this server's UI library (`OneCore.App`).
  defp built(owner, name, files, sandbox) do
    if App.app?(files),
      do: built_before(Plugins.plugin(owner, name), files) || App.build(files, sandbox),
      else: {:ok, files}
  end

  # The apps as the plugin's running version built them, where that was from the same source against the same UI library,
  # since building it again would make the same: `{:ok, files}`, or `nil`.
  defp built_before(nil, _files), do: nil

  defp built_before(artifact, files) do
    client = &Map.filter(&1, fn {path, _} -> Version.client?(path) end)

    with %Version{ui: ui, code: hash} <- Plugins.version(artifact),
         true <- ui == App.library().code,
         true <- client.(Store.source(artifact, hash)) == client.(files) do
      {:ok,
       Map.merge(
         Map.reject(files, fn {path, _} -> Version.client?(path) end),
         client.(Store.files(artifact, hash))
       )}
    else
      _ -> nil
    end
  end

  @doc """
  Builds again, from its source, the app of each plugin anyone has deployed that was built against another UI library
  than this server's, installed or not, as a server acting for whoever the plugin is, as they would deploy it again. A server does it as it starts, on a build no published server
  build is newer than: so a server release rebuilds every app once, on the first server to take it, the others finding
  it done, and a server left on an older build never builds them back. One another server built meanwhile against the
  same UI library is done, not a failure.
  """
  def rebuild do
    code = App.library().code

    for %{plugin: artifact} <- if(newest?(), do: deployed(), else: []),
        {whose, name} = {Store.whose(artifact), Path.basename(artifact)},
        Servers.runs?(whose) or (Store.about(artifact)["sandbox"] == true and Servers.here?(whose)),
        built_with(artifact) not in [nil, code],
        {_number, hash, _tag} <- [Store.latest(artifact)],
        # It stays as installed, or not, as it was.
        install? = fn _version -> Plugins.plugin(whose, name) == artifact end,
        {:error, why} <- [deploy(whose, name, Store.source(artifact, hash), :stored, :stored, install?)],
        built_with(artifact) != code,
        do: Logger.error("#{artifact}'s app wasn't built again against this server's UI library: #{why}")
  end

  # The code of the UI library the latest version of a plugin's app was built against, as the bucket has it, or `nil`
  # for one with no app.
  defp built_with(artifact) do
    with {_number, hash, _tag} <- Store.latest(artifact),
         {:ok, code} <- Store.file(artifact, hash, "client/ui"),
         do: code,
         else: (_ -> nil)
  end

  # Whether no server build the network has published is newer than this server's, by the count a build is named with
  # (`tooling/version`). A network that has published none, as one being built here, has none newer.
  defp newest? do
    count = &with({count, _commit} <- Integer.parse(&1), do: count, else: (_ -> 0))

    Builds.latest()
    |> Enum.filter(fn {name, _} -> String.starts_with?(name, "server/") end)
    |> Enum.all?(fn {_name, %{"build" => build}} -> count.(build) <= count.(Builds.build()) end)
  end

  # The plugins a plugin uses are there before it is. One this server ships that's running from files a server
  # shipped before this one's is deployed again, by a server acting for the network, and one that's missing is
  # installed for `owner` from what the network has of it. Bundled plugins go by name, so a plugin can come before
  # one it uses that's still to be shipped.
  defp bring(owner, name, uses) do
    for used <- uses,
        Servers.here?(@network) and bundled(used) != %{} and deploy_bundled?(used, Version.code(bundled(used))),
        do: deploy_bundled(used)

    there =
      &(is_map_key(Plugins.running(owner), &1) or
          (bundled(&1) != %{} and Installed.install(owner, &1, "#{@network}/plugins/#{&1}") == :ok))

    case Enum.reject(uses, there) do
      [] ->
        :ok

      [missing] ->
        {:error,
         "#{name} uses #{missing}, which isn't installed for #{owner} or the network. Add it for them first, or deploy it."}

      missing ->
        {:error,
         "#{name} uses #{Enum.join(missing, ", ")}, which aren't installed for #{owner} or the network. " <>
           "Add them for them first, or deploy them."}
    end
  end

  # The files of a plugin this server ships with, as a deploy from its folder sends them, or none if it ships with
  # no such plugin.
  defp bundled(name) do
    with %{^name => folder} <- Application.get_env(:one_core, :bundled, %{}),
         {:ok, paths} <- Folder.files(folder) do
      Map.new(paths, &{&1, File.read!(Path.join(folder, &1))})
    else
      _ -> %{}
    end
  end

  defp check(name, files) do
    cond do
      Installed.named(name) != :ok ->
        Installed.named(name)

      not (is_map(files) and map_size(files) > 0 and
               Enum.all?(files, fn {path, text} -> path?(path) and is_binary(text) end)) ->
        {:error, "A plugin's files are text by their paths, with no path part starting with a dot"}

      true ->
        :ok
    end
  end

  defp path?(path), do: is_binary(path) and Enum.all?(String.split(path, "/"), &(&1 =~ ~r/^[^.]/))
end
