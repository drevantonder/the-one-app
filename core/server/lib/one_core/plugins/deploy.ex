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
  alias OneCore.Call.State
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
  `trusted` whether it's trusted, where an untrusted plugin runs in a sandbox (`OneCore.Sandbox`), each as the version
  before had it if `:stored`. It's theirs, so what's installed for them runs it at once; whoever else has it installed
  sees it as an update (`OneCore.Plugins.Lifecycle`). Gives its version's number and the compiler's warnings.
  """
  def deploy(owner, name, files, from \\ :stored, trusted \\ :stored),
    do: deploy(owner, name, files, from, trusted, fn _version -> true end)

  @doc "Deploys as `deploy/5` does, installing it for `owner` only where `install?` says so of the version."
  def deploy(owner, name, files, from, trusted, install?) do
    with {:ok, number, warnings, version} <- keep(owner, name, files, from, trusted, install?),
         artifact = version.artifact,
         :ok <- installed(owner, name, version, install?.(version)) do
      others = Installed.installed_for(artifact) -- [owner]
      Server.notify_servers(Enum.uniq([owner | others]))
      # The servers told look for themselves; this one looks here.
      for other <- others, Servers.here?(other), do: Task.start(fn -> OneCore.Plugins.Lifecycle.reconcile(other) end)
      {:ok, number, warnings}
    end
  end

  @doc """
  Builds, checks and keeps a new version of the plugin `name` of `owner`'s, as `deploy/6` does, and gives its number,
  the compiler's warnings and the version, loaded, or described for an untrusted one, without installing it. Where
  `install?` says it's to be installed for `owner`, it's checked to fit them first.
  """
  def keep(owner, name, files, from, trusted, install?) do
    artifact = Plugins.plugin(owner, name) || "#{owner}/plugins/#{name}"
    trusted = if trusted == :stored, do: stored_trust(owner, name, artifact), else: trusted != false

    stage = &State.stage(State.id(), &1)

    with :ok <- check(name, files),
         _ = stage.("Building its app"),
         {:ok, built} <- built(owner, name, files, trusted),
         _ = stage.(if(trusted, do: "Compiling its code", else: "Compiling its code in its sandbox")),
         {:ok, version, warnings} <- loaded(artifact, built, trusted) do
      version = %{version | artifact: artifact}
      install = install?.(version)

      with :ok <- outputs(version),
           :ok <- if(install, do: fits(owner, name, version), else: :ok),
           {:ok, warnings} <- if(trusted, do: {:ok, warnings}, else: Sandbox.check(owner, version, built)),
           _ = stage.("Keeping it in the bucket"),
           {:ok, number} <- Store.keep(artifact, name, built, files, from, trusted),
           :ok <- Store.describe(version) do
        listed(artifact, version)
        warnings = Enum.join(Enum.reject([warnings | untasked(name, version, built)], &(&1 == "")), "\n")
        {:ok, number, warnings, %{version | number: number}}
      else
        {:error, why} ->
          Version.unload(version)
          {:error, why}
      end
    end
  end

  # A version deployed for whoever it's installed for runs for them at once, since it's theirs.
  defp installed(owner, name, version, true) do
    commit = Store.about(version.artifact)["from"]["commit"]

    with %{} <- Installed.change(owner, name, {:deployed, version.artifact, version.number, version.trusted, commit}) do
      Plugins.reload(owner, [{{version.artifact, version.number, version.trusted}, {:ok, version}}])
      :ok
    end
  end

  defp installed(_owner, _name, version, false) do
    Version.unload(version)
    :ok
  end

  # As it's installed for `owner`, where it is, and otherwise as its latest version was deployed.
  defp stored_trust(owner, name, artifact) do
    case elem(Installed.read(owner), 0) do
      %{^name => %{"plugin" => ^artifact, "trusted" => trusted}} -> trusted != false
      _ -> Store.about(artifact)["trusted"] != false
    end
  end

  # A tool that says what it's doing (`OneCore.Call.stage/2`) or asks a person (`OneCore.Call.ask/2`) is one that
  # should be an MCP task: one that isn't says neither to anyone, and can't ask. Each such tool is warned of, by
  # what its call/3 clause runs and the plugin's own functions that runs, which is what reading its code shows.
  defp untasked(name, version, files) do
    quoted =
      for {path, text} <- files,
          String.starts_with?(path, "server/") and Path.extname(path) == ".ex" and is_binary(text),
          {:ok, quoted} <- [Code.string_to_quoted(text)],
          do: quoted

    {functions, tools} = Enum.reduce(quoted, {%{}, %{}}, &defined/2)

    for tool <- version.tools,
        tool[:task_support] == nil,
        Enum.any?(Map.get(tools, tool.name, []), &tasky?(&1, functions, MapSet.new())) do
      "#{name}_#{tool.name} says what it's doing or asks a person (OneCore.Call.stage or OneCore.Call.ask), which " <>
        "only a task shows or asks: give it task_support: :optional beside its name in tools/0."
    end
  end

  # Each function the code defines, by its name and arity, and each tool's call/3 clause, by the tool's name.
  defp defined(quoted, defined) do
    {_quoted, defined} =
      Macro.prewalk(quoted, defined, fn
        {kind, _, [head, [do: body]]} = node, {functions, tools} when kind in [:def, :defp] ->
          case head do
            {:when, _, [head | _]} -> {node, function(head, body, functions, tools)}
            head -> {node, function(head, body, functions, tools)}
          end

        node, defined ->
          {node, defined}
      end)

    defined
  end

  defp function({:call, _, [tool, _inputs, _call]}, body, functions, tools) when is_binary(tool),
    do: {functions, Map.update(tools, tool, [body], &[body | &1])}

  defp function({name, _, arguments}, body, functions, tools) when is_atom(name) and is_list(arguments),
    do: {Map.update(functions, {name, length(arguments)}, [body], &[body | &1]), tools}

  defp function(_head, _body, functions, tools), do: {functions, tools}

  # Whether `body` calls `OneCore.Call.stage` or `ask`, itself or through the code's own functions.
  defp tasky?(body, functions, seen) do
    {_body, found} =
      Macro.prewalk(body, {false, seen}, fn
        node, {true, seen} ->
          {node, {true, seen}}

        {{:., _, [{:__aliases__, _, module}, function]}, _, _} = node, {false, seen}
        when module in [[:Call], [:OneCore, :Call]] and function in [:stage, :ask] ->
          {node, {true, seen}}

        {name, _, arguments} = node, {false, seen} when is_atom(name) and is_list(arguments) ->
          called = {name, length(arguments)}

          if Map.has_key?(functions, called) and not MapSet.member?(seen, called) do
            seen = MapSet.put(seen, called)
            {node, {Enum.any?(functions[called], &tasky?(&1, functions, seen)), seen}}
          else
            {node, {false, seen}}
          end

        node, found ->
          {node, found}
      end)

    elem(found, 0)
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

  # A version's code, compiled and loaded here, or for an untrusted plugin only described, by its sandbox's VM.
  defp loaded(artifact, built, true), do: Version.load(Path.basename(artifact), built)

  defp loaded(artifact, built, false) do
    with {:ok, version} <- Sandbox.describe(artifact, built), do: {:ok, version, ""}
  end

  @doc """
  Every plugin anyone has deployed, as its latest version has it: `[%{plugin: artifact, title: ..., description:
  its skill's, subtitle: ..., about: ..., icon: ..., apps: [...], tools: [names], permissions: [...], trusted:
  ..., essential: ...}]`, what a person reads of it as `Version.listing/1` gives it, its permissions as
  `permissions/0` gives them, whether it's trusted and whether every network has it (`essential`). It's what lists
  every app there is, whether or not anyone has it now. One that was removed, by the plugin that keeps what's
  deleted, has no latest version, and isn't listed.

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
          trusted: description["trusted"] != false,
          essential: description["essential"] == true
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
        trusted: version.trusted,
        essential: version.essential
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
         {installed, _automatic} = Installed.read(@network),
         install? = &(installed[name] == nil and &1.essential or installed[name]["state"] not in [nil, "recently_deleted"]),
         {:ok, number, _warnings} = deployed <- deploy(@network, name, files, nil, true, install?),
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
  defp built(owner, name, files, trusted) do
    if App.app?(files),
      do: built_before(Plugins.plugin(owner, name), files) || App.build(files, trusted),
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
  than this server's, installed or not, as a server acting for whoever the plugin is, as they would deploy it again.
  A server does it once it's up, on a build no published server build is newer than: so a server release rebuilds
  every app once, and a server left on an older build never builds them back. For each whose plugins they are, one
  server does it (`rebuilds?/2`), and the others run what it built as each version reaches them. One another server
  built meanwhile against the same UI library is done, not a failure.
  """
  def rebuild do
    code = App.library().code

    stale =
      for %{plugin: artifact} <- if(newest?(), do: deployed(), else: []),
          whose = Store.whose(artifact),
          Servers.runs?(whose) or (Store.about(artifact)["trusted"] == false and Servers.here?(whose)),
          built_with(artifact) not in [nil, code],
          do: {whose, artifact}

    ours = for whose <- Enum.uniq(for {whose, _} <- stale, do: whose), rebuilds?(whose, code), do: whose

    for {whose, artifact} <- stale,
        whose in ours,
        name = Path.basename(artifact),
        {_number, hash, _tag} <- [Store.latest(artifact)],
        # It stays as installed, or not, as it was.
        install? = fn _version -> Plugins.plugin(whose, name) == artifact end,
        {:error, why} <- [rebuilt(whose, name, Store.source(artifact, hash), install?)],
        built_with(artifact) != code,
        do: Logger.error("#{artifact}'s app wasn't built again against this server's UI library: #{why}")
  end

  # One app that couldn't be built again is said, and the rest are still built.
  defp rebuilt(whose, name, files, install?) do
    deploy(whose, name, files, :stored, :stored, install?)
  rescue
    error -> {:error, Exception.message(error)}
  end

  # Whether this server builds the apps of `whose` plugins again against the UI library `code`. The first to ask
  # writes that it does, `<whose>/rebuild`, and the others leave it to that one while it's on: four servers building
  # the same apps at once wrote the same blobs at once, which R2 refuses ("Reduce your concurrent request rate for the
  # same object"), and lost to each other. One that's not on any more left it unfinished, and the next to ask carries
  # on from what's left.
  defp rebuilds?(whose, code) do
    {blob, me} = {"#{whose}/rebuild", Server.key()}
    claim = JSON.encode!(%{ui: code, server: me})

    case Bucket.read(blob) do
      :absent ->
        Bucket.write(blob, claim) == :ok or rebuilds?(whose, code)

      {:ok, was, tag} ->
        case JSON.decode!(was) do
          %{"ui" => ^code, "server" => ^me} ->
            true

          %{"ui" => ^code, "server" => server} ->
            Servers.seen(server) not in [:on, nil] and
              (Bucket.replace(blob, claim, tag) == :ok or rebuilds?(whose, code))

          _another ->
            Bucket.replace(blob, claim, tag) == :ok or rebuilds?(whose, code)
        end
    end
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
          (bundled(&1) != %{} and OneCore.Plugins.Lifecycle.bring(owner, &1, "#{@network}/plugins/#{&1}") == :ok))

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
