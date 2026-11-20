defmodule OneCore.Apps.Deploy do
  @moduledoc """
  Deploying an app: a new one, or a new version of one. The builds of its NIFs and programs it lacks are downloaded
  (`OneCore.Apps.Precompiled`), its app, if it has one, is built (`OneCore.Entrypoint`),
  its Elixir is compiled and loaded (`OneCore.Apps.Version`), and it's kept in the bucket
  (`OneCore.Apps.Store`) and runs at once.
  """
  require Logger
  alias OneCore.Call.State
  alias OneCore.{Entrypoint, Bucket, Builds, Sandbox, Server, Servers}
  alias OneCore.Apps
  alias OneCore.Apps.{Installed, Calls, Lifecycle, LoadedVersion, Precompiled, Requires, Store, Version}
  alias OneCore.Apps.Version.Compile

  @network "network"
  @looks_after :timer.minutes(5)
  # What each owner has deployed, as the network keeps it: `deployed/<owner>/<name>`.
  @deployed "network/deployed/"
  @outputs "Each tool gives its outputs beside its inputs: a JSON Schema of the object it answers, which the skill " <>
             "building-apps says how to write."
  # What an app is for, as its skill's `metadata` says (`OneCore.App`): one of these.
  @categories [
    "Connectors",
    "Privacy & Security",
    "Data",
    "AI",
    "Agent tools",
    "Devices",
    "Developer tools",
    "Utilities",
    "Admin"
  ]

  @doc "What an app may say it's for, its `category`, in the order a listing shows them."
  def categories, do: @categories

  @doc """
  Deploys an app for `owner` from its files by their paths: the app that `name` is for them is updated, for
  everyone who has it, and if there's none it's a new app of theirs, installed for them. Its app, if it has one, is
  built first (`OneCore.Entrypoint`). `from` is the git repo its files came from, as `OneCore.Apps.Store.keep/8` takes
  it, and `trusted` whether it's trusted, where an untrusted app runs in a sandbox (`OneCore.Sandbox`), each as the
  version before had it if `:stored`. Gives its version's number and the compiler's warnings.

  It's committed when its `latest` is swapped, as a git ref is: one that fails or stops before changes nothing anyone
  reads, and what it wrote is swept by `OneCore.Apps.Store.prune/1`. Everything after is derived from `latest` by
  whoever looks (`OneCore.Apps.sync/2`, `deployed/0`), so one that stops after is deployed all the same.
  """
  def deploy(owner, name, files, from \\ :stored, trusted \\ :stored),
    do: deploy(owner, name, files, from, trusted, fn _version -> true end)

  @doc "Deploys as `deploy/5` does, installing it for `owner` only where `install?` says so of the version."
  def deploy(owner, name, files, from, trusted, install?) do
    artifact = target(owner, name, from)
    trusted = if trusted == :stored, do: Store.trusted?(artifact), else: trusted != false
    # What it commits against, read before it writes anything (`OneCore.Apps.Store.keep/8`).
    was = Store.latest(artifact)

    stage = &State.stage(State.id(), &1)

    with :ok <- check(name, files),
         # An untrusted app runs no NIF or program of its own, so nothing is downloaded for one.
         {:ok, files} <- if(trusted, do: Precompiled.fill(files), else: {:ok, files}),
         _ = stage.("Building its app"),
         {:ok, built} <- built(owner, name, files, trusted),
         built = Store.deployment(built),
         _ = stage.(if(trusted, do: "Compiling its code", else: "Compiling its code in its sandbox")),
         {:ok, version, warnings} <- loaded(artifact, built, trusted) do
      # The deploy holds the version it loaded until it ends, however it ends: what it's installed for holds it after.
      try do
        install = install?.(version)

        with {:ok, styling} <- Entrypoint.lint(files, version, trusted),
             :ok <- if(why = Version.waiting(name, version), do: {:error, why}, else: :ok),
             :ok <- described(name, version),
             :ok <- if(install, do: fits_deploy(owner, name, version), else: :ok),
             {:ok, warnings} <- if(trusted, do: {:ok, warnings}, else: Sandbox.check(owner, version, built)),
             _ = stage.("Keeping it in the bucket"),
             # Its files, what it describes and that it's listed are written first, and none is read until `latest`
             # names them.
             :ok <- listed(artifact),
             {:ok, number} <- Store.keep(artifact, name, version, built, files, from, trusted, was),
             :ok <- committed(owner, name, number, version, install) do
          warnings = Enum.join(Enum.reject([warnings, styling | untasked(name, version, built)], &(&1 == "")), "\n")
          {:ok, number, warnings}
        end
      after
        LoadedVersion.release([version])
      end
    end
  end

  # The app a deploy for `owner` makes a version of: the one `name` is for them, or a new one of theirs. Where that's
  # a narrower owner's copy, a person's own installed for everyone, and what's deployed comes from the same git repo,
  # it's the same app, and it moves to them: it's deployed as theirs, `<owner>/apps/<name>`, which supersedes the
  # copy (`OneCore.Apps.Lifecycle.supersede/2`), so the app has one copy, the broadest.
  defp target(owner, name, from) do
    ours = "#{owner}/apps/#{name}"

    case Apps.app(owner, name) do
      nil ->
        ours

      app ->
        moves = broader?(owner, Store.whose(app)) and Store.repo(from) != nil and Store.repo(from) == Store.repo(app)
        if moves, do: ours, else: app
    end
  end

  # Whether `owner` is broader than `whose`: the network than a group or a user, and a group than a user.
  defp broader?(owner, whose), do: breadth(owner) > breadth(whose)
  defp breadth(@network), do: 2
  defp breadth("groups/" <> _), do: 1
  defp breadth(_user), do: 0

  # After `latest`: its author has it installed by the installation machine's own move for an app they deployed, and
  # the servers of whoever has it take it, this one at once, with the version the deploy loaded. It supersedes the
  # narrower copies of itself.
  defp committed(owner, name, number, %Version{artifact: artifact} = version, install) do
    forget()
    moved = if install, do: Installed.change(owner, name, {:deployed, artifact, nil}), else: %{}
    if install, do: Apps.sync(owner, [{artifact, number, {:ok, version}}])
    Server.notify_servers(Enum.uniq([owner | Installed.installed_for(artifact)]))

    case moved do
      {:error, why} ->
        {:error, "#{name} is deployed as version #{number}, and isn't installed for #{owner}: #{why}"}

      _ ->
        if install, do: Lifecycle.supersede(owner, artifact)
        :ok
    end
  end

  # A tool that says what it's doing (`OneCore.Call.stage/2`) or asks a person (`OneCore.Call.ask/2`) is one that
  # should be an MCP task: one that isn't says neither to anyone, and can't ask. Each such tool is warned of, by
  # what its call/3 clause runs and the app's own functions that runs, which is what reading its code shows.
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

  # What every version deployed now says of itself, all that's missing at once, so one deploy puts it right.
  defp described(name, version) do
    refused =
      for {:error, why} <- [outputs(version), titled(version), categorized(version), icon(name, version)], do: why

    if refused == [], do: :ok, else: {:error, Enum.join(refused, "\n")}
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

  # A person reads an app by its title, never by the name it's installed as.
  defp titled(%{title: nil}),
    do:
      {:error,
       "SKILL.md's `metadata` has no `title`, its name wherever a person reads it: `title: Hello`, beside its " <>
         "subtitle and category (the skill building-apps)."}

  defp titled(_version), do: :ok

  defp categorized(version) do
    case Version.listing(version).category do
      category when category in @categories ->
        :ok

      nil ->
        {:error,
         "SKILL.md's `metadata` has no `category`, which is one of: " <>
           Enum.join(@categories, ", ") <> " (the skill building-apps)."}

      category ->
        {:error,
         "SKILL.md's `metadata.category` is #{inspect(category)}, which isn't one of: " <>
           Enum.join(@categories, ", ") <> " (the skill building-apps)."}
    end
  end

  # Every app has an icon of its own, shown wherever it is (the App Store, the Vault), as an Android package does.
  defp icon(name, %{icon: nil}),
    do:
      {:error,
       "#{name} has no icon. icon/0 names one of its files, its app's icon or one of its own, as the skill " <>
         "building-apps says."}

  defp icon(_name, _version), do: :ok

  # A version's code, compiled and loaded here, or for an untrusted app only described, by its sandbox's VM.
  defp loaded(artifact, built, true),
    do: LoadedVersion.load(artifact, Version.code(built), &Compile.compile(artifact, built, &1))

  defp loaded(artifact, built, false) do
    with {:ok, version} <- Sandbox.describe(artifact, built), do: {:ok, version, ""}
  end

  @doc """
  Every app anyone has deployed, as its latest version has it: `[%{app: artifact, description: its skill's,
  tools: [names], requires: %{name => why}, trusted: ..., repo: ..., title: ..., icon: ..., subtitle: ..., about: ...,
  category: ..., entrypoints: [...], widgets: [...]}]`, what a person reads of it as `Version.listing/1` gives it, its
  requirements as `requires/0` gives them, whether it's trusted, and the git repo it was deployed from
  (`Store.repo/1`), which tells copies of the same app (`Store.same?/2`). It's what lists every app there is, whether or
  not anyone has it now. One that was removed, by the app that keeps what's deleted, has no latest version, and
  isn't listed.

  Which apps are listed is the network's blobs `deployed/<whose>/<name>`, and what each says is read from its
  `latest`, as that version described itself (`OneCore.Apps.Store.described/2`): so it's never older than what
  runs. It's three trips to the bucket with a request for each app, and changes only when someone deploys, so
  it's kept: read again behind a request once it's five minutes old, as `OneCore.Apps` looks at what's installed,
  and forgotten when this server deploys or is told to look again (`forget/0`).
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
    named =
      for blob <- Bucket.list(@deployed),
          [owner, name] <- [Regex.run(~r{^#{@deployed}(.+)/([^/]+)$}, blob, capture: :all_but_first)],
          do: "#{owner}/apps/#{name}"

    abouts =
      for {artifact, {:ok, about, _tag}} <- Enum.zip(named, Bucket.read_all(Enum.map(named, &"#{&1}/latest"))),
          do: {artifact, JSON.decode!(about)}

    described = Bucket.read_all(for {artifact, about} <- abouts, do: "#{artifact}/described/#{about["code"]}")

    deployed =
      for {{artifact, about}, {:ok, description, _tag}} <- Enum.zip(abouts, described),
          {:ok, listing} <- [Version.listed(artifact, description)],
          do: Map.merge(listing, %{app: artifact, trusted: Store.trusted?(about), repo: Store.repo(about["from"])})

    :ets.insert(__MODULE__, {:deployed, deployed, System.monotonic_time(:millisecond)})
    deployed
  end

  # The app `artifact` is among what everyone has deployed, under whose it is: a person who deploys the network's
  # app lists the network's. Written before its version is kept, it lists nothing until one is.
  defp listed(artifact) do
    blob = "#{@deployed}#{Store.whose(artifact)}/#{Path.basename(artifact)}"
    with :exists <- Bucket.write(blob, artifact), do: :ok
  end

  @doc "Makes the table that keeps what everyone has deployed (`deployed/0`), which the process calling it holds."
  def table, do: :ets.new(__MODULE__, [:named_table, :public, read_concurrency: true])

  @doc """
  Whether an app fits whoever it's installed for: the apps it requires are there for them, and one with scheduled
  jobs requires an app that keeps them. `:ok`, or `{:error, why}`.
  """
  def fits(owner, name, version) do
    with {:error, why, _details} <- fits_deploy(owner, name, version), do: {:error, why}
  end

  defp fits_deploy(owner, name, version) do
    with :ok <- launcher(owner, name, version),
         :ok <- required(owner, name, version.requires),
         do: scheduled(owner, name, version)
  end

  @doc """
  Whether a version has the launcher: the app in the category `launcher`, which every client opens first. A network
  has exactly one, installed for the network.
  """
  def launcher?(version), do: Enum.any?(version.entrypoints, &(&1.category == "launcher"))

  defp launcher(owner, name, version) do
    others = for {other, running} <- Apps.versions([@network]), other != name, launcher?(running), do: other

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

  # An app's scheduled jobs are kept by an app it requires.
  defp scheduled(_owner, _name, %Version{jobs: []}), do: :ok

  defp scheduled(owner, name, version) do
    case Calls.keepers(Apps.required(owner, version)) do
      [_ | _] -> :ok
      [] -> {:error, "#{name} has scheduled jobs, so it requires an app that keeps its schedule: one with schedule/3"}
    end
  end

  # An app's entrypoint is built against this server's UI library (`OneCore.Entrypoint`).
  defp built(owner, name, files, trusted) do
    if Entrypoint.client?(files),
      do: built_before(Apps.app(owner, name), files) || Entrypoint.build(files, trusted),
      else: {:ok, files}
  end

  # The apps as the app's running version built them, where that was from the same source against the same UI library,
  # since building it again would make the same: `{:ok, files}`, or `nil`.
  defp built_before(nil, _files), do: nil

  defp built_before(artifact, files) do
    client = &Map.filter(&1, fn {path, _} -> Version.client?(path) end)

    with %Version{ui: ui, code: hash} <- Apps.version(artifact),
         true <- ui == Entrypoint.library().code,
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
  Has the UI library this server serves be the build in `folder`, and the apps built against another built again, by
  the server that builds them: as a local network's code reloader has it, as it changes.
  """
  def ui(folder) do
    Application.put_env(:one_core, :ui, folder)
    :persistent_term.erase({OneCore.Entrypoint, :library})
    {:ok, _pid} = Task.Supervisor.start_child(OneCore.Asks, &rebuild/0)
    :ok
  end

  @doc """
  Builds again, from its source, the app of each app anyone has deployed that was built against another UI library
  than this server's, installed or not, as a server acting for whoever the app is, as they would deploy it again.
  A server does it once it's up, on a build no published server build is newer than: so a server release rebuilds
  every app once, and a server left on an older build never builds them back. For each whose apps they are, one
  server does it (`rebuilds?/2`), and the others run what it built as each version reaches them. One another server
  built meanwhile against the same UI library is done, not a failure.
  """
  def rebuild do
    code = Entrypoint.library().code

    stale =
      for %{app: artifact} <- if(newest?(), do: deployed(), else: []),
          whose = Store.whose(artifact),
          Apps.runs?(whose, Store.trusted?(artifact)),
          built_with(artifact) not in [nil, code],
          do: {whose, artifact}

    # One server builds each one's apps again: four building the same apps at once wrote the same blobs at once,
    # which R2 refuses ("Reduce your concurrent request rate for the same object"), and lost to each other.
    # Claimed for this build, as the network's core apps are taken to it under the same claim, just before.
    ours = for whose <- Enum.uniq(for {whose, _} <- stale, do: whose), Servers.claims?("#{whose}/rebuild", Builds.build()), do: whose

    for {whose, artifact} <- stale,
        whose in ours,
        name = Path.basename(artifact),
        {_number, hash, _tag} <- [Store.latest(artifact)],
        # It stays as installed, or not, as it was.
        install? = fn _version -> Apps.app(whose, name) == artifact end,
        {:error, why} <- [rebuilt(whose, name, Store.source(artifact, hash), install?)],
        built_with(artifact) != code,
        do: Logger.error("#{artifact}'s app wasn't built again against this server's UI library: #{why}")
  end

  # One app that couldn't be built again is said, and the rest are still built.
  defp rebuilt(whose, name, files, install?) do
    with {:error, why, _details} <- deploy(whose, name, files, :stored, :stored, install?), do: {:error, why}
  rescue
    error -> {:error, Exception.message(error)}
  end

  # The code of the UI library the latest version of an app's entrypoint was built against, as the bucket has it, or `nil`
  # for one with no app.
  defp built_with(artifact) do
    with {_number, hash, _tag} <- Store.latest(artifact),
         {:ok, code} <- Store.file(artifact, hash, "client/ui"),
         do: code,
         else: (_ -> nil)
  end

  # Whether no server build the network has published is newer than this server's, by the count of commits in its
  # build metadata (`tooling/version`), which only grows on main. A network that has published none, as one being
  # built here, has none newer, and one from before builds had versions is older than any that has one.
  defp newest? do
    case Builds.latest()["server-code"] do
      %{"build" => build} -> Builds.count(build) <= Builds.count(Builds.build())
      nil -> true
    end
  end

  # The apps an app requires are there before it is: installed for `owner`, or for the network.
  defp required(owner, name, requires) do
    running = Apps.running(owner)

    case Enum.reject(Enum.sort(requires), fn {req, _why} -> is_map_key(running, req) end) do
      [] ->
        :ok

      missing ->
        why =
          case Enum.map(missing, &elem(&1, 0)) do
            [one] ->
              "#{name} requires #{one}, which isn't installed for #{owner} or the network. Add it for them first, " <>
                "or deploy it."

            many ->
              "#{name} requires #{Enum.join(many, ", ")}, which aren't installed for #{owner} or the network. " <>
                "Add them for them first, or deploy them."
          end

        details = for {req, reason} <- missing, do: Map.put(Requires.source(owner, req, owner), :why, reason)
        {:error, why, %{missing: details}}
    end
  end

  defp check(name, files) do
    cond do
      Installed.named(name) != :ok ->
        Installed.named(name)

      not (is_map(files) and map_size(files) > 0 and Enum.all?(Map.values(files), &is_binary/1)) ->
        {:error, "An app's files are text by their paths"}

      true ->
        Enum.find_value(Map.keys(files), :ok, &with(:ok <- Bucket.check_key(&1), do: nil))
    end
  end
end
