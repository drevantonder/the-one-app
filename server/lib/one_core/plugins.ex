defmodule OneCore.Plugins do
  @moduledoc """
  Runs a network's plugins on this server, and adds, updates and removes them while it keeps running.

  A plugin is an artifact whose blobs are code, and it's whoever's deployed it: `<theirs>/plugins/<name>`. Each
  set of files deployed is kept at `code/<its hash>/`, and its `newest` says which set it runs now, with that
  version's number. A deploy replaces `newest` only if it's unchanged, so two deploys at once can't both be the
  plugin's next version, and first writes the version it replaces to `versions/<number>`, so a plugin keeps its
  history.

  A plugin is added for a user, for a group or for the network, and that's who it runs for. What's added for
  someone is one blob under their prefix, `added`: each plugin's name there, with the plugin it is. A call that
  runs for someone finds a plugin by its name among what's added for them, and then among what's added for the
  network, which is for everyone in it. A plugin has an artifact of its own for each one it was added for,
  `<theirs>/own/<name>`, and the plugins it uses are found the same way from whoever it was added for.

  A call runs on the version that was newest when it began. That version stays loaded until its last call ends,
  however many versions follow it. A plugin's processes hold a version the same way, and move to the newest
  (`OneCore.Plugins.Process`).

  Adding a plugin brings along the ones it uses that the server ships with: its `:shipped` setting, each one's
  folder by its name. Those are added for the network.

  A server looks in the bucket at what's added for the network when it starts, and at what's added for anyone
  else when it's first asked for them. It looks again when it's told to, when it's asked for a plugin it doesn't
  run, and every so often (`OneCore.Server.look/0`).
  """
  use GenServer
  require Logger
  alias ExJsonSchema.Validator.Error
  alias OneCore.{Artifact, Bucket, Call, Hooks, Jobs, Server, Services, User}
  alias OneCore.Plugins.Version

  @network "network"

  # This, and after it what runs plugins' processes and what tells the plugins that hear a hook, which hold
  # versions here.
  def child_spec(_) do
    children = [%{id: __MODULE__, start: {GenServer, :start_link, [__MODULE__, nil, [name: __MODULE__]]}}]
    children = children ++ __MODULE__.Process.children() ++ [{Task.Supervisor, name: Hooks}]
    start = {Supervisor, :start_link, [children, [strategy: :rest_for_one]]}
    %{id: __MODULE__, type: :supervisor, start: start}
  end

  @doc "Whose plugins something that runs for `who` finds, in order: their own, then the network's."
  def chain(who), do: Enum.uniq(List.wrap(User.owner(who)) ++ [@network])

  @doc """
  Deploys a plugin for `owner` from its files by their paths: the plugin that `name` is for them is updated, and
  if there's none it's a new plugin of theirs, added for them. Gives its version's number and the compiler's
  warnings.
  """
  def deploy(owner, name, files) do
    with :ok <- check(name, files),
         {:ok, version, warnings} <- Version.load(files) do
      artifact = plugin(owner, name) || "#{owner}/plugins/#{name}"
      version = %{version | artifact: artifact}

      with :ok <- bring(owner, name, version.uses),
           :ok <- scheduled(owner, name, version),
           {:ok, number} <- keep(artifact, name, files),
           :ok <- note(owner, &{:ok, Map.put(&1, name, artifact)}) do
        looked(owner, [{artifact, number, {:ok, version}}])
        Server.tell_servers()
        {:ok, number, warnings}
      else
        {:error, why} ->
          Version.unload(version)
          {:error, why}
      end
    end
  end

  @doc "Adds the plugin `artifact` for `owner`, who find it by `name`."
  def add(owner, name, artifact) do
    added = fn
      %{^name => ^artifact} = added ->
        {:ok, added}

      %{^name => other} ->
        {:error, "#{other} is added for #{owner} as #{name}. Remove it first, or add this by another name."}

      added ->
        {:ok, Map.put(added, name, artifact)}
    end

    cond do
      not name?(name) -> {:error, "A plugin's name is lowercase letters, digits and hyphens, starting with a letter"}
      newest(artifact) == nil -> {:error, "There's no plugin #{artifact}"}
      true -> with :ok <- note(owner, added), do: told(owner)
    end
  end

  @doc "Removes a plugin from what's added for `owner`. It stays whoever's it is, and what it kept stays."
  def remove(owner, name) do
    used_by = for {other, version} <- versions([owner]), name in version.uses, do: other

    removed = fn
      %{^name => _artifact} = added -> {:ok, Map.delete(added, name)}
      _ -> {:error, "No plugin named #{name} is added for #{owner}"}
    end

    case used_by do
      [] -> with :ok <- note(owner, removed), do: told(owner)
      _ -> {:error, "#{name} is used by #{Enum.join(used_by, ", ")}. Remove what uses it first."}
    end
  end

  # What's added for an owner changed: it's looked at here, and the other servers are told to.
  defp told(owner) do
    look(owner)
    Server.tell_servers()
  end

  @doc """
  Deploys for the network each plugin this server ships with that the network has never had. One the network has
  is left as it is: deploying it again is what updates it. Gives what `deploy/3` gave for each, by its name.
  """
  def ship do
    for {name, folder} <- Enum.sort(Application.get_env(:one_core, :shipped, %{})),
        newest("#{@network}/plugins/#{name}") == nil do
      case shipped(name) do
        files when files == %{} -> {name, {:error, "There are no files in #{folder}"}}
        files -> {name, deploy(@network, name, files)}
      end
    end
  end

  @doc """
  Calls a plugin's tool for `asked`: who asked and who it's for, `{by, for}`, or one who that is both. Gives
  `{:ok, answer}`, or `{:error, why}` with what the caller is told. A call another server ran instead gives
  `{:passed_on, result}`, that server's result. `passed_on` says another server passed this call on to this one,
  and `:passed_back` that this one would pass it on in turn.
  """
  def call(name, tool, inputs, asked, passed_on \\ false) do
    {_by, who} = asked = asked(asked)

    run(chain(who), name, fn at, versions ->
      version = versions[name]

      case Enum.find(version.tools, &(&1.name == tool)) do
        nil ->
          {:error, "#{name} has no tool named #{tool}. It has: #{Enum.map_join(version.tools, ", ", & &1.name)}"}

        %{takes: takes} = found ->
          # What's passed on is what was asked, so the server it's passed to hooks into the same thing.
          ask = {"tools/call", %{name: "#{name}_#{tool}", arguments: inputs}}
          call = called(at, name, versions, ask, asked, passed_on)

          with :ok <- open_to(found, name, asked),
               :ok <- given(takes, inputs),
               {:ok, about} <- hooked(%{"plugin" => name, "tool" => tool, "inputs" => inputs}, takes, asked) do
            heard(about, version, asked, fn -> version.plugin.call(tool, about["inputs"], call) end)
          end
      end
    end)
  end

  # A plugin is added for users and groups. A sharing link's holder is neither, and calls only a tool that says
  # they may: one that keeps to what the link reaches.
  defp open_to(tool, name, {by, _for}) do
    if by != nil and User.link?(by) and tool[:by_link] != true,
      do: {:error, "#{name}_#{tool.name} isn't a tool that a sharing link's holder calls"},
      else: :ok
  end

  # What a call is about once the plugins that change its inputs have, if the ones that decide it agree to it.
  defp hooked(about, takes, asked) do
    with {:ok, changes, by} <- Hooks.change("inputs", about, asked),
         about = Map.put(%{about | "inputs" => Map.merge(about["inputs"], changes)}, "changed", by),
         :ok <- if(changes == %{}, do: :ok, else: changed(takes, about)),
         :ok <- Hooks.decide("call", about, asked) do
      {:ok, about}
    else
      {:refuse, why} -> {:error, why}
      {:error, why} -> {:error, why}
    end
  end

  # A tool is only ever called with what it takes, whoever changed its inputs.
  defp changed(takes, %{"inputs" => inputs, "changed" => by}) do
    with {:error, wrong} <- given(takes, inputs) do
      changed = Enum.map_join(Enum.sort(by), ", ", fn {input, plugin} -> "#{input} by #{plugin}" end)
      {:error, "Its inputs were changed (#{changed}), and it doesn't take them now:\n#{wrong}"}
    end
  end

  # Runs a call, and tells the plugins that hear calls how it went. One passed on is heard where it ran.
  defp heard(about, version, asked, call) do
    began = System.monotonic_time(:microsecond)
    took = fn error -> Map.merge(about, %{"took" => System.monotonic_time(:microsecond) - began, "error" => error}) end

    try do
      call.()
    catch
      :throw, {Call, _outcome} = passed ->
        throw(passed)

      kind, reason ->
        error = Version.own_names(Exception.format_banner(kind, reason, __STACKTRACE__), version)
        Hooks.heard("called", took.(error), asked)
        :erlang.raise(kind, reason, __STACKTRACE__)
    else
      answer ->
        Hooks.heard("called", took.(nil), asked)
        {:ok, answer}
    end
  end

  # Whether `inputs` are what a tool says it takes, its JSON Schema, so a tool is only ever called with them. What's
  # wrong is said to whoever called, a person or an agent, a sentence each.
  defp given(takes, inputs) do
    with {:error, wrong} <- ExJsonSchema.Validator.validate(takes, inputs, error_formatter: false) do
      {:error, Enum.map_join(wrong, "\n", &wrong(&1.error, input(&1.path), takes.schema))}
    end
  end

  # An input as it's named to whoever called, by its path in the inputs: `expenses/0`. All of them have no name.
  defp input("#"), do: nil
  defp input("#/" <> at), do: at

  defp wrong(%Error.Required{missing: missing}, at, _takes) do
    needs = if match?([_], missing), do: "is needed", else: "are needed"
    "#{Enum.map_join(missing, ", ", &Enum.join(List.wrap(at) ++ [&1], "/"))} #{needs}."
  end

  defp wrong(%Error.Type{expected: expected, actual: actual}, at, _takes),
    do: "#{at || "What's given"} is #{Enum.map_join(expected, " or ", &kind/1)}, not #{kind(actual)}."

  defp wrong(%Error.Enum{}, at, takes) do
    case described(takes, at) do
      %{"enum" => one_of} -> "#{at} is one of #{Enum.join(one_of, ", ")}."
      _ -> "#{at} isn't one of what it can be."
    end
  end

  defp wrong(%Error.AdditionalProperties{}, at, _takes), do: "There's no input named #{at}."
  defp wrong(%Error.Minimum{expected: least, exclusive?: true}, at, _takes), do: "#{at} is more than #{least}."
  defp wrong(%Error.Minimum{expected: least}, at, _takes), do: "#{at} is #{least} or more."
  defp wrong(%Error.Maximum{expected: most, exclusive?: true}, at, _takes), do: "#{at} is less than #{most}."
  defp wrong(%Error.Maximum{expected: most}, at, _takes), do: "#{at} is #{most} or less."
  defp wrong(%Error.MinLength{expected: 1}, at, _takes), do: "#{at} can't be empty."
  defp wrong(%Error.MinLength{expected: least}, at, _takes), do: "#{at} is #{least} characters or more."
  defp wrong(%Error.MaxLength{expected: most}, at, _takes), do: "#{at} is #{most} characters or fewer."
  defp wrong(%Error.MinItems{expected: 1}, at, _takes), do: "#{at} can't be empty."
  defp wrong(%Error.MinItems{expected: least}, at, _takes), do: "#{at} has #{least} or more."
  defp wrong(%Error.MaxItems{expected: most}, at, _takes), do: "#{at} has #{most} or fewer."
  # Anything rarer is said as the checker says it, which names the rule that was broken.
  defp wrong(other, at, _takes), do: Enum.join(List.wrap(at) ++ [to_string(other)], ": ")

  # A kind of value, as people call it.
  defp kind("string"), do: "text"
  defp kind("integer"), do: "a whole number"
  defp kind("number"), do: "a number"
  defp kind("boolean"), do: "true or false"
  defp kind("array"), do: "a list"
  defp kind("object"), do: "a set of named values"
  defp kind("null"), do: "nothing"

  # What a tool says of one of its inputs, by the input's path.
  defp described(takes, at) do
    Enum.reduce(String.split(at || "", "/", trim: true), takes, fn part, takes ->
      if is_map(takes), do: takes["properties"][part] || takes["items"]
    end)
  end

  @doc """
  Calls a function of the plugin that `name` is for `at`, for a service of its own, with a call for `asked` and
  then `arguments`. Gives what `call/5` does.
  """
  def function(at, name, function, arguments, asked, passed_on \\ false) do
    asked = asked(asked)

    run([at], name, fn at, versions ->
      plugin = versions[name].plugin

      case plugin && exported(plugin, function, length(arguments) + 1) do
        nil ->
          {:error, "#{name} has no function #{function} that takes a call and #{length(arguments)} more"}

        function ->
          ask = {"plugins/call", %{added: at, plugin: name, function: function, arguments: arguments}}
          {:ok, apply(plugin, function, [called(at, name, versions, ask, asked, passed_on) | arguments])}
      end
    end)
  end

  @doc """
  Calls the `hook/3` of the plugin that `name` is for `at`, at a hook it hooks into, for `asked`. Gives
  `{:ok, answer}` with its answer as `OneCore.Hooks.answer/2` gives it, and otherwise what `call/5` does. A plugin
  that no longer hooks into it has nothing to say. A hook gives no access of its own: a plugin that hears of an
  artifact changing is told only if it could already reach it, and isn't told what it changed itself.
  """
  def hook(at, name, hook, about, asked, passed_on \\ false) do
    asked = asked(asked)

    run([at], name, fn at, versions ->
      version = versions[name]

      call =
        called(
          at,
          name,
          versions,
          {"hooks/call", %{added: at, plugin: name, hook: hook, about: about}},
          asked,
          passed_on
        )

      if hook in version.hooks and hears?(call, about),
        do: {:ok, Hooks.answer(hook, version.plugin.hook(hook, about, call))},
        else: {:ok, Hooks.answer(hook, Hooks.nothing(hook))}
    end)
  end

  defp hears?(call, %{"artifact" => artifact, "by" => by}),
    do: call.who != by and Call.refused(call, artifact, "viewer") == nil

  defp hears?(_call, _about), do: true

  @doc """
  Has the scheduled jobs that are due called, of the plugin that `name` is for `at`: the plugin it uses that
  keeps schedules is given its jobs, as the plugin, and a function that makes one's call here. That plugin has one
  server make each call, once. The call runs for whoever the plugin was added for, asked by nobody, and is never
  passed on: it's made where the schedule is kept.
  """
  def schedule(at, name, passed_on \\ false) do
    clock = {nil, at}

    run([at], name, fn at, versions ->
      asked = called(at, name, versions, {"plugins/schedule", %{added: at, plugin: name}}, clock, passed_on)

      fire = fn job ->
        with {:error, why} <- call(name, job.tool, job.inputs, clock, true),
             do: Logger.error("#{name}'s scheduled #{job.tool} for #{at} failed: #{why}")

        Server.changed()
      end

      for keeper <- keepers(Map.delete(versions, name)), do: keeper.schedule(asked, versions[name].jobs, fire)
      {:ok, :ok}
    end)
  end

  # The plugins among `versions` that keep other plugins' schedules.
  defp keepers(versions) do
    for {_name, %Version{plugin: plugin}} <- versions,
        plugin != nil,
        function_exported?(plugin, :schedule, 3),
        do: plugin
  end

  # A plugin's scheduled jobs are kept by a plugin it uses.
  defp scheduled(_owner, _name, %Version{jobs: []}), do: :ok

  defp scheduled(owner, name, version) do
    with {:ok, used} <- GenServer.call(__MODULE__, {:used, owner, version.uses}),
         [_ | _] <- keepers(used) do
      :ok
    else
      _ -> {:error, "#{name} has scheduled jobs, so it uses a plugin that keeps its schedule, as database does"}
    end
  end

  @doc "The function of `module` named by the text `name` that takes `arity` arguments, or `nil` if it has none."
  def exported(module, name, arity) do
    Enum.find_value(module.__info__(:functions), fn {function, takes} ->
      if takes == arity and Atom.to_string(function) == name, do: function
    end)
  end

  @doc """
  Runs `fun` with who a plugin was added for and the newest version of it and of each plugin it uses, by their
  names. The plugin is the first that `name` is for one of `owners`. Those versions stay loaded until `fun`
  returns. Gives what `fun` does, or `{:error, why}` with what it raised, as its caller is told it.
  """
  def run(owners, name, fun) do
    with {:ok, held, at, versions} <- begin(owners, name),
         do: held(held, Map.values(versions), fn -> fun.(at, versions) end)
  end

  @doc "Runs `fun` with the newest version of the plugin `artifact`, as `run/3` does. `:absent` if it isn't running here."
  def run(artifact, fun) do
    with {:ok, held, version} <- hold(artifact), do: held(held, [version], fn -> fun.(version) end)
  end

  defp held(held, versions, fun) do
    fun.()
  catch
    :throw, {Call, outcome} ->
      outcome

    kind, reason ->
      Logger.error(Exception.format(kind, reason, __STACKTRACE__))
      {blamed, _} = Exception.blame(kind, reason, __STACKTRACE__)
      why = if kind == :error, do: Exception.message(blamed), else: Exception.format_banner(kind, blamed)
      {:error, Enum.reduce(versions, why, &Version.own_names(&2, &1))}
  after
    release(held)
  end

  @doc """
  Holds the newest version of the plugin `artifact` loaded: `{:ok, held, version}`, or `:absent` if it isn't
  running here. It stays loaded until `release(held)`, or until the process that asked ends.
  """
  def hold(artifact), do: GenServer.call(__MODULE__, {:hold, artifact})

  @doc "Lets go of the versions that were held, which are unloaded once nothing runs or holds them."
  def release(held), do: GenServer.call(__MODULE__, {:end, held})

  @doc "The plugins something that runs for `who` finds, by name: each one's `Version`, which holds its tools, its guide and its apps."
  def running(who), do: versions(chain(who))

  # The plugins added for `owners`, by name, the first's found first. One this server hasn't looked at is first.
  defp versions(owners) do
    with :unknown <- GenServer.call(__MODULE__, {:running, owners}) do
      Enum.each(owners, &look/1)
      GenServer.call(__MODULE__, {:running, owners})
    end
  end

  @doc "The plugin that `name` is for `owner`, as its artifact's name, or `nil` if none is added for them by that name."
  def plugin(owner, name) do
    case versions([owner]) do
      %{^name => version} -> version.artifact
      _ -> nil
    end
  end

  @doc "Who a plugin was added for and its name there, as something that runs for `who` finds the plugin `artifact`, or `nil`."
  def added(who, artifact) do
    Enum.find_value(chain(who), fn at ->
      Enum.find_value(versions([at]), fn {name, version} -> if version.artifact == artifact, do: {at, name} end)
    end)
  end

  @doc "The version running here of the plugin `artifact`, or `nil`."
  def version(artifact), do: GenServer.call(__MODULE__, {:version, artifact})

  @doc "Every plugin this server has looked at that has scheduled jobs: who it was added for, its name there, and the jobs."
  def scheduled, do: GenServer.call(__MODULE__, :scheduled)

  @doc "One file of the set of the plugin `artifact`'s files named `code`, which needn't be the set running now."
  def file(artifact, code, path) do
    with {:ok, text, _tag} <- Bucket.read("#{artifact}/code/#{code}/#{path}"), do: {:ok, text}
  end

  @doc """
  Looks in the bucket at what's added for everyone this server has looked at before, and runs each plugin's newest
  version. Gives whether anything changed.
  """
  def look, do: Enum.any?(Enum.map(GenServer.call(__MODULE__, :owners), &look/1))

  @doc "Looks in the bucket at what's added for `owner`, and runs each plugin's newest version. Gives whether anything changed."
  def look(owner), do: looked(owner, [])

  # `loaded` are versions already loaded here, each as `newer/2` gives one. A plugin whose newest version took
  # over has its processes follow it, and its services installed here that weren't running start.
  defp looked(owner, loaded) do
    {added, _tag} = added(owner)
    known = for {artifact, _number, _loaded} <- loaded, do: artifact
    newer = newer(Enum.uniq(Map.values(added)) -- known, GenServer.call(__MODULE__, :newest))
    {changed, any} = GenServer.call(__MODULE__, {:looked, owner, added, loaded ++ newer})

    for artifact <- changed do
      # A process whose own code brought the newer version in is the one that can't wait for itself.
      for process <- __MODULE__.Process.all(artifact), process != self(), do: __MODULE__.Process.follow(process)
      Services.start(artifact)
    end

    if any, do: Jobs.look()
    any
  end

  # What a function of a plugin is called with: who asked, who it runs for, and what was asked, to pass on. The
  # plugin's own artifact is the one for whoever it was added for.
  defp called(at, name, versions, ask, {by, who}, passed_on) do
    own = "#{at}/own/#{name}"

    %Call{
      own: %Artifact{prefix: own <> "/"},
      folder: Path.join(Server.home(), own),
      server: Server.key(),
      for: who,
      by: by,
      plugin: name,
      who: versions[name].artifact,
      ask: ask,
      uses: for({used, version} <- versions, used != name, into: %{}, do: {used, {version.artifact, version.plugin}}),
      passed_on: passed_on
    }
  end

  # Who asked and who for: one who is both.
  defp asked({_by, _for} = asked), do: asked
  defp asked(who) when is_binary(who), do: {who, who}

  # A call holds the newest version of its plugin and of each plugin that one uses, however far that goes. A
  # plugin this server doesn't run is looked for first.
  defp begin(owners, name) do
    with {:absent, _missing} <- GenServer.call(__MODULE__, {:begin, owners, name}),
         _ = Enum.each(Enum.uniq(owners ++ [@network]), &look/1),
         {:absent, missing} <- GenServer.call(__MODULE__, {:begin, owners, name}) do
      if missing == name,
        do: {:error, "There's no plugin named #{name}"},
        else: {:error, "#{name} needs #{missing}, which isn't added for #{Enum.join(owners, " or ")} or the network"}
    end
  end

  # The plugins a plugin uses are added before it is: the ones missing come from what the server ships with, and
  # are added for the network.
  defp bring(owner, name, uses) do
    case Enum.reject(uses, &(is_map_key(running(owner), &1) or match?({:ok, _, _}, deploy(@network, &1, shipped(&1))))) do
      [] ->
        :ok

      missing ->
        {:error,
         "#{name} uses #{Enum.join(missing, ", ")}, which isn't added for #{owner} or the network. Deploy it first."}
    end
  end

  # The files of a plugin this server ships with, or none if it ships with no such plugin.
  defp shipped(name) do
    with %{^name => folder} <- Application.get_env(:one_core, :shipped, %{}) do
      files = Enum.filter(Path.wildcard(Path.join(folder, "**")), &File.regular?/1)
      Map.new(files, &{Path.relative_to(&1, folder), File.read!(&1)})
    else
      _ -> %{}
    end
  end

  defp check(name, files) do
    cond do
      not name?(name) ->
        {:error, "A plugin's name is lowercase letters, digits and hyphens, starting with a letter"}

      not (is_map(files) and map_size(files) > 0 and
               Enum.all?(files, fn {path, text} -> path?(path) and is_binary(text) end)) ->
        {:error, "A plugin's files are text by their paths, with no path part starting with a dot"}

      true ->
        :ok
    end
  end

  defp name?(name), do: is_binary(name) and name =~ ~r/^[a-z][a-z0-9-]*$/

  defp path?(path), do: is_binary(path) and Enum.all?(String.split(path, "/"), &(&1 =~ ~r/^[^.]/))

  # What's added for an owner, as the bucket has it: each plugin's name there with the plugin, and the blob's tag,
  # which is none if there's no blob.
  defp added(owner) do
    case Bucket.read("#{owner}/added") do
      {:ok, added, tag} -> {JSON.decode!(added), tag}
      :absent -> {%{}, nil}
    end
  end

  # Changes what's added for an owner, as `change` says of what's there. Two changes at once both land.
  defp note(owner, change) do
    {added, tag} = added(owner)

    with {:ok, next} <- change.(added) do
      blob = "#{owner}/added"

      cond do
        next == added -> :ok
        tag == nil and Bucket.write(blob, JSON.encode!(next)) == :ok -> :ok
        tag != nil and Bucket.replace(blob, JSON.encode!(next), tag) == :ok -> :ok
        true -> note(owner, change)
      end
    end
  end

  # A plugin's newest version as the bucket has it, its number and the set of files it runs, and the blob's tag.
  # `nil` if there's no such plugin.
  defp newest(artifact) do
    with {:ok, newest, tag} <- Bucket.read("#{artifact}/newest"),
         %{"number" => number, "code" => code} = JSON.decode!(newest) do
      {number, code, tag}
    else
      :absent -> nil
    end
  end

  # Writes a set of files, and makes it the plugin's next version. The version being replaced goes to the
  # plugin's history first, so every version but the newest is there.
  defp keep(artifact, name, files) do
    code = Version.code(files)
    for {path, text} <- files, do: Bucket.write("#{artifact}/code/#{code}/#{path}", text)
    was = newest(artifact)
    number = if was, do: elem(was, 0) + 1, else: 1
    next = JSON.encode!(%{number: number, code: code})

    wrote =
      with {before, code, tag} <- was do
        Bucket.write("#{artifact}/versions/" <> String.pad_leading("#{before}", 8, "0"), code)
        Bucket.replace("#{artifact}/newest", next, tag)
      else
        nil -> Bucket.write("#{artifact}/newest", next)
      end

    # Another deploy of this plugin got there first, and whoever asked for this one hasn't seen it.
    if wrote == :ok,
      do: {:ok, number},
      else: {:error, "#{name} was deployed by someone else just now. Look at it, and do this again."}
  end

  # The versions in the bucket of the plugins `artifacts` that are newer than the ones in `newest`: each one's
  # plugin, its number, and the version loaded, which is `:error` for one this server can't compile. Read at once:
  # a bucket at an address is asked for each over the internet.
  defp newer(artifacts, newest) do
    artifacts
    |> Task.async_stream(&{&1, newest(&1)}, max_concurrency: 16, timeout: :infinity)
    |> Enum.flat_map(fn {:ok, {artifact, in_bucket}} ->
      with {number, code, _tag} <- in_bucket,
           true <- number > Map.get(newest, artifact, 0),
           do: [{artifact, number, loaded(artifact, code)}],
           else: (_ -> [])
    end)
  end

  defp loaded(artifact, code) do
    paths = Bucket.list("#{artifact}/code/#{code}/")

    files =
      paths
      |> Task.async_stream(&Bucket.read/1, max_concurrency: 16, timeout: :infinity)
      |> Enum.zip_with(paths, fn {:ok, {:ok, text, _tag}}, path ->
        {String.replace_prefix(path, "#{artifact}/code/#{code}/", ""), text}
      end)
      |> Map.new()

    case Version.load(files) do
      {:ok, version, _warnings} ->
        {:ok, %{version | artifact: artifact}}

      {:error, why} ->
        Logger.error("#{artifact} isn't updated here, because its newest version doesn't compile:\n#{why}")
        :error
    end
  end

  # A server starts with the newest version of every plugin added for the network. `added` is what's added for
  # each owner it has looked at, `running` and `newest` are by plugin, and `calls` are the versions each call
  # holds.
  @impl true
  def init(nil) do
    :ets.new(Hooks, [:named_table, read_concurrency: true])
    {added, _tag} = added(@network)
    state = %{added: %{}, running: %{}, newest: %{}, calls: %{}}
    {state, _changed, _any} = took_over(state, @network, added, newer(Enum.uniq(Map.values(added)), %{}))
    {:ok, state}
  end

  @impl true
  def handle_call({:looked, owner, added, loaded}, _from, state) do
    {state, changed, any} = took_over(state, owner, added, loaded)
    {:reply, {changed, any}, state}
  end

  def handle_call(:newest, _from, state), do: {:reply, state.newest, state}
  def handle_call(:owners, _from, state), do: {:reply, Map.keys(state.added), state}
  def handle_call({:version, artifact}, _from, state), do: {:reply, state.running[artifact], state}

  def handle_call({:running, owners}, _from, state) do
    if Enum.all?(owners, &is_map_key(state.added, &1)) do
      # Their own are found before the network's.
      {:reply, running(state, Enum.reduce(Enum.reverse(owners), %{}, &Map.merge(&2, state.added[&1]))), state}
    else
      {:reply, :unknown, state}
    end
  end

  def handle_call(:scheduled, _from, state) do
    scheduled =
      for {owner, added} <- state.added,
          {name, artifact} <- added,
          %Version{jobs: [_ | _] = jobs} <- [state.running[artifact]],
          do: {owner, name, jobs}

    {:reply, scheduled, state}
  end

  def handle_call({:used, owner, names}, _from, state), do: {:reply, used(state, [owner, @network], names, %{}), state}

  # The caller is watched, so a call that dies has ended too.
  def handle_call({:begin, owners, name}, {caller, _}, state) do
    with {at, _artifact} <- found(state, owners, name),
         {:ok, versions} <- used(state, [at, @network], [name], %{}) do
      call = Process.monitor(caller)
      {:reply, {:ok, call, at, versions}, put_in(state.calls[call], Map.values(versions))}
    else
      absent -> {:reply, absent, state}
    end
  end

  def handle_call({:hold, artifact}, {caller, _}, state) do
    case state.running do
      %{^artifact => version} ->
        call = Process.monitor(caller)
        {:reply, {:ok, call, version}, put_in(state.calls[call], [version])}

      _ ->
        {:reply, :absent, state}
    end
  end

  def handle_call({:end, call}, _from, state) do
    Process.demonitor(call, [:flush])
    {:reply, :ok, ended(state, call)}
  end

  @impl true
  def handle_info({:DOWN, call, :process, _, _}, state), do: {:noreply, ended(state, call)}

  # What's added for `owner` is `added` now, and each version in `loaded` takes over from its plugin's if it's
  # newer: deploys that finish out of order still leave the one with the highest number, as the bucket has it. A
  # version that doesn't compile here leaves the one before it running. A plugin that's added for nobody this
  # server has looked at no longer runs here. Gives the plugins whose version changed, and whether anything did.
  defp took_over(state, owner, added, loaded) do
    {state, changed, stale} =
      Enum.reduce(loaded, {state, [], []}, fn {artifact, number, loaded}, {state, changed, stale} ->
        version = with {:ok, version} <- loaded, do: version

        cond do
          number <= Map.get(state.newest, artifact, 0) ->
            {state, changed, [version | stale]}

          version == :error ->
            {put_in(state.newest[artifact], number), changed, stale}

          true ->
            state = %{state | newest: Map.put(state.newest, artifact, number)}
            {put_in(state.running[artifact], version), [artifact | changed], [state.running[artifact] | stale]}
        end
      end)

    any = changed != [] or state.added[owner] != added
    state = put_in(state.added[owner], added)
    kept = for {_owner, added} <- state.added, {_name, artifact} <- added, into: MapSet.new(), do: artifact
    {running, dropped} = Map.split_with(state.running, fn {artifact, _version} -> artifact in kept end)

    state = %{
      state
      | running: running,
        newest: Map.filter(state.newest, fn {artifact, _number} -> artifact in kept end)
    }

    changed = Enum.uniq(changed ++ Map.keys(dropped))

    # Which of each owner's plugins hooks into what is kept for every call to find without asking.
    for {at, added} <- state.added, at == owner or Enum.any?(added, fn {_name, artifact} -> artifact in changed end) do
      hooking =
        for {name, artifact} <- Enum.sort(added),
            %Version{hooks: hooks} <- [running[artifact]],
            hook <- hooks,
            reduce: %{} do
          hooking -> Map.update(hooking, hook, [name], &(&1 ++ [name]))
        end

      :ets.insert(Hooks, {at, hooking})
    end

    {unload_unused(state, stale ++ Map.values(dropped)), changed, any}
  end

  # The versions running of the plugins `added`, by their names there.
  defp running(state, added) do
    for {name, artifact} <- added, version = state.running[artifact], into: %{}, do: {name, version}
  end

  # Who the plugin `name` was added for and the plugin, as the first of `owners` that has one by that name finds
  # it. One of them this server hasn't looked at yet is looked at first.
  defp found(_state, [], name), do: {:absent, name}

  defp found(state, [owner | owners], name) do
    case state.added do
      %{^owner => %{^name => artifact}} -> {owner, artifact}
      %{^owner => _added} -> found(state, owners, name)
      _ -> {:absent, name}
    end
  end

  # The versions a call to `names` holds, by their plugins' names as `owners` find them, or the first plugin that
  # isn't running.
  defp used(_state, _owners, [], found), do: {:ok, found}
  defp used(state, owners, [name | names], found) when is_map_key(found, name), do: used(state, owners, names, found)

  defp used(state, owners, [name | names], found) do
    with {_at, artifact} <- found(state, owners, name),
         %{^artifact => version} <- state.running do
      used(state, owners, version.uses ++ names, Map.put(found, name, version))
    else
      _ -> {:absent, name}
    end
  end

  defp ended(state, call) do
    {versions, calls} = Map.pop(state.calls, call, [])
    unload_unused(%{state | calls: calls}, versions)
  end

  defp unload_unused(state, versions) do
    held = Map.values(state.running) ++ Enum.concat(Map.values(state.calls))
    for %Version{} = version <- versions, version not in held, do: Version.unload(version)
    state
  end
end
