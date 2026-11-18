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
  alias OneCore.Apps.{Installed, Calls, LoadedVersion, Precompiled, Store, Version}
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
  built first (`OneCore.Entrypoint`). `from` is the git repo its files came from, as `OneCore.Apps.Store.keep/6` takes
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
    artifact = Apps.app(owner, name) || "#{owner}/apps/#{name}"
    trusted = if trusted == :stored, do: Store.trusted?(artifact), else: trusted != false
    # What it commits against, read before it writes anything (`OneCore.Apps.Store.keep/8`).
    was = Store.latest(artifact)

    stage = &State.stage(State.id(), &1)

    with :ok <- check(name, files),
         {:ok, files} <- Precompiled.fill(files),
         _ = stage.("Building its app"),
         {:ok, built} <- built(owner, name, files, trusted),
         built = Store.deployment(built),
         _ = stage.(if(trusted, do: "Compiling its code", else: "Compiling its code in its sandbox")),
         {:ok, version, warnings} <- loaded(artifact, built, trusted) do
      # The deploy holds the version it loaded until it ends, however it ends: what it's installed for holds it after.
      try do
        install = install?.(version)

        with :ok <- if(why = Version.waiting(name, version), do: {:error, why}, else: :ok),
             :ok <- outputs(version),
             :ok <- categorized(version),
             :ok <- icon(name, version),
             :ok <- if(install, do: fits(owner, name, version), else: :ok),
             {:ok, warnings} <- if(trusted, do: {:ok, warnings}, else: Sandbox.check(owner, version, built)),
             _ = stage.("Keeping it in the bucket"),
             # Its files, what it describes and that it's listed are written first, and none is read until `latest`
             # names them.
             :ok <- listed(artifact),
             {:ok, number} <- Store.keep(artifact, name, version, built, files, from, trusted, was),
             :ok <- committed(owner, name, number, version, install) do
          warnings = Enum.join(Enum.reject([warnings | untasked(name, version, built)], &(&1 == "")), "\n")
          {:ok, number, warnings}
        end
      after
        LoadedVersion.release([version])
      end
    end
  end

  # After `latest`: its author has it installed by the installation machine's own move for an app they deployed, and
  # the servers of whoever has it take it, this one at once, with the version the deploy loaded.
  defp committed(owner, name, number, %Version{artifact: artifact} = version, install) do
    forget()
    moved = if install, do: Installed.change(owner, name, {:deployed, artifact, nil}), else: %{}
    if install, do: Apps.sync(owner, [{artifact, number, {:ok, version}}])
    Server.notify_servers(Enum.uniq([owner | Installed.installed_for(artifact)]))

    case moved do
      {:error, why} -> {:error, "#{name} is deployed as version #{number}, and isn't installed for #{owner}: #{why}"}
      _ -> :ok
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

  defp categorized(version) do
    case Version.listing(version).category do
      category when category in @categories ->
        :ok

      category ->
        {:error,
         "Its skill's metadata says its category is #{inspect(category)}. It says one of: " <>
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
  tools: [names], requires: %{name => why}, trusted: ..., title: ..., icon: ..., subtitle: ..., about: ...,
  category: ..., entrypoints: [...], widgets: [...]}]`, what a person reads of it as `Version.listing/1` gives it, its
  requirements as `requires/0` gives them, and whether it's trusted. It's what lists every app there is, whether or
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
          do: Map.merge(listing, %{app: artifact, trusted: Store.trusted?(about)})

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
    with :ok <- launcher(owner, name, version),
         :ok <- required(owner, name, Map.keys(version.requires)),
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
          Servers.runs?(whose, Store.trusted?(artifact)),
          built_with(artifact) not in [nil, code],
          do: {whose, artifact}

    ours = for whose <- Enum.uniq(for {whose, _} <- stale, do: whose), rebuilds?(whose, code), do: whose

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
    deploy(whose, name, files, :stored, :stored, install?)
  rescue
    error -> {:error, Exception.message(error)}
  end

  # Whether this server builds the apps of `whose` apps again against the UI library `code`. The first to ask
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
            Servers.ready?(server) == false and
              (Bucket.replace(blob, claim, tag) == :ok or rebuilds?(whose, code))

          _another ->
            Bucket.replace(blob, claim, tag) == :ok or rebuilds?(whose, code)
        end
    end
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
    case Enum.reject(requires, &is_map_key(Apps.running(owner), &1)) do
      [] ->
        :ok

      [missing] ->
        {:error,
         "#{name} requires #{missing}, which isn't installed for #{owner} or the network. Add it for them first, " <>
           "or deploy it."}

      missing ->
        {:error,
         "#{name} requires #{Enum.join(missing, ", ")}, which aren't installed for #{owner} or the network. " <>
           "Add them for them first, or deploy them."}
    end
  end

  defp check(name, files) do
    cond do
      Installed.named(name) != :ok ->
        Installed.named(name)

      not (is_map(files) and map_size(files) > 0 and
               Enum.all?(files, fn {path, text} -> path?(path) and is_binary(text) end)) ->
        {:error, "An app's files are text by their paths, with no path part starting with a dot"}

      true ->
        :ok
    end
  end

  defp path?(path), do: is_binary(path) and Enum.all?(String.split(path, "/"), &(&1 =~ ~r/^[^.]/))
end
