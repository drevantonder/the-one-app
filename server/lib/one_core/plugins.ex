defmodule OneCore.Plugins do
  @moduledoc """
  Runs a network's plugins on this server, and adds, updates and removes them while it keeps running.

  A plugin is an artifact whose blobs are code. Each set of files deployed is kept at `code/<its hash>/`. Which set
  each plugin runs is one blob for all of a user's plugins, `added`: each one's name with its newest version, which
  is a number and the set it runs, where no set means the plugin was removed. So a server reads one blob to know
  what to run. A deploy replaces that blob only if it's unchanged, so two deploys at once can't both be a plugin's
  next version, and first writes the version it replaces to the plugin's `versions/<number>`, so a plugin keeps
  its history.

  A call runs on the version that was newest when it began. That version stays loaded until its last call ends,
  however many versions follow it. A plugin's processes hold a version the same way, and move to the newest
  (`OneCore.Plugins.Process`).

  A plugin says which other plugins it uses, and a call holds the versions of those too. Adding a plugin brings
  along the ones it uses that the server ships with: its `:shipped` setting, each one's folder by its name.

  A server learns of a version another server was given by looking in the bucket: when it's told to, when it's
  asked for a plugin it doesn't run, and every so often (`OneCore.Server.look/0`).
  """
  use GenServer
  require Logger
  alias OneCore.{Artifact, Bucket, Call, Hooks, Jobs, Server, Services, User}
  alias OneCore.Plugins.Version
  alias ExJsonSchema.Validator.Error

  @added User.anyone() <> "/added"

  # This, and after it what runs plugins' processes and what tells the plugins that hear a hook, which hold
  # versions here.
  def child_spec(_) do
    children = [%{id: __MODULE__, start: {GenServer, :start_link, [__MODULE__, nil, [name: __MODULE__]]}}]
    children = children ++ __MODULE__.Process.children() ++ [{Task.Supervisor, name: Hooks}]
    start = {Supervisor, :start_link, [children, [strategy: :rest_for_one]]}
    %{id: __MODULE__, type: :supervisor, start: start}
  end

  @doc "Adds a plugin, or updates it, from its files by their paths. Gives the compiler's warnings."
  def deploy(name, files) do
    with :ok <- check(name, files),
         {:ok, version, warnings} <- Version.load(files) do
      with :ok <- bring(name, version.uses),
           :ok <- scheduled(name, version),
           {:ok, number} <- keep(name, files) do
        takes_over(name, number, {:ok, version})
        Server.tell_servers()
        {:ok, number, warnings}
      else
        {:error, why} ->
          Version.unload(version)
          {:error, why}
      end
    end
  end

  @doc "Removes a plugin. What it kept stays."
  def remove(name) do
    used_by = for {other, version} <- running(), name in version.uses, do: other

    with [] <- used_by,
         {%{^name => %{"code" => code}}, _tag} when code != "" <- added(),
         {:ok, number} <- keep(name, %{}) do
      takes_over(name, number, {:ok, nil})
      Server.tell_servers()
    else
      [_ | _] -> {:error, "#{name} is used by #{Enum.join(used_by, ", ")}. Remove what uses it first."}
      {:error, why} -> {:error, why}
      _ -> {:error, "There's no plugin named #{name}"}
    end
  end

  @doc """
  Deploys each plugin this server ships with that the network has never had. One the network has is left as it is:
  deploying it again is what updates it. Gives what `deploy/2` gave for each, by its name.
  """
  def ship do
    {added, _tag} = added()

    for {name, folder} <- Enum.sort(Application.get_env(:one_core, :shipped, %{})), not is_map_key(added, name) do
      case shipped(name) do
        files when files == %{} -> {name, {:error, "There are no files in #{folder}"}}
        files -> {name, deploy(name, files)}
      end
    end
  end

  @doc """
  Calls a plugin's tool for `who`. Gives `{:ok, answer}`, or `{:error, why}` with what the caller is told. A call
  another server ran instead gives `{:passed_on, result}`, that server's result. `passed_on` says another server
  passed this call on to this one, and `:passed_back` that this one would pass it on in turn.
  """
  def call(name, tool, inputs, who, passed_on \\ false) do
    run(name, fn versions ->
      version = versions[name]

      case Enum.find(version.tools, &(&1.name == tool)) do
        nil ->
          {:error, "#{name} has no tool named #{tool}. It has: #{Enum.map_join(version.tools, ", ", & &1.name)}"}

        %{takes: takes} = found ->
          # What's passed on is what was asked, so the server it's passed to hooks into the same thing.
          call = called(name, versions, {"tools/call", %{name: "#{name}_#{tool}", arguments: inputs}}, who, passed_on)

          with :ok <- open_to(found, name, who),
               :ok <- given(takes, inputs),
               {:ok, about} <- hooked(%{"plugin" => name, "tool" => tool, "inputs" => inputs}, takes, who) do
            heard(about, version, who, fn -> version.plugin.call(tool, about["inputs"], call) end)
          end
      end
    end)
  end

  # A plugin is added for users. A sharing link's holder is none, and calls only a tool that says they may: one
  # that keeps to what the link reaches.
  defp open_to(tool, name, who) do
    if User.link?(who) and tool[:by_link] != true,
      do: {:error, "#{name}_#{tool.name} isn't a tool that a sharing link's holder calls"},
      else: :ok
  end

  # What a call is about once the plugins that change its inputs have, if the ones that decide it agree to it.
  defp hooked(about, takes, who) do
    with {:ok, changes, by} <- Hooks.change("inputs", about, who),
         about = Map.put(%{about | "inputs" => Map.merge(about["inputs"], changes)}, "changed", by),
         :ok <- if(changes == %{}, do: :ok, else: changed(takes, about)),
         :ok <- Hooks.decide("call", about, who) do
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
  defp heard(about, version, who, call) do
    began = System.monotonic_time(:microsecond)
    took = fn error -> Map.merge(about, %{"took" => System.monotonic_time(:microsecond) - began, "error" => error}) end

    try do
      call.()
    catch
      :throw, {Call, _outcome} = passed ->
        throw(passed)

      kind, reason ->
        error = Version.own_names(Exception.format_banner(kind, reason, __STACKTRACE__), version)
        Hooks.heard("called", took.(error), who)
        :erlang.raise(kind, reason, __STACKTRACE__)
    else
      answer ->
        Hooks.heard("called", took.(nil), who)
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
  Calls one of a plugin's functions for a service of its own, with a call for `who` and then `arguments`. Gives
  what `call/5` does.
  """
  def function(name, function, arguments, who, passed_on \\ false) do
    run(name, fn versions ->
      plugin = versions[name].plugin

      case plugin && exported(plugin, function, length(arguments) + 1) do
        nil ->
          {:error, "#{name} has no function #{function} that takes a call and #{length(arguments)} more"}

        function ->
          ask = {"plugins/call", %{plugin: name, function: function, arguments: arguments}}
          {:ok, apply(plugin, function, [called(name, versions, ask, who, passed_on) | arguments])}
      end
    end)
  end

  @doc """
  Calls a plugin's `hook/3` at a hook it hooks into, for `who`. Gives `{:ok, answer}` with its answer as
  `OneCore.Hooks.answer/2` gives it, and otherwise what `call/5` does. A plugin that no longer hooks into it has
  nothing to say.
  """
  def hook(name, hook, about, who, passed_on \\ false) do
    run(name, fn versions ->
      version = versions[name]
      call = called(name, versions, {"hooks/call", %{plugin: name, hook: hook, about: about}}, who, passed_on)

      if hook in version.hooks,
        do: {:ok, Hooks.answer(hook, version.plugin.hook(hook, about, call))},
        else: {:ok, Hooks.answer(hook, Hooks.nothing(hook))}
    end)
  end

  @doc """
  Has a plugin's scheduled jobs that are due called: the plugin it uses that keeps schedules is given its jobs, as
  the plugin, and a function that makes one's call here. That plugin has one server make each call, once. The call
  runs for the user the plugin was added for, and is never passed on: it's made where the schedule is kept.
  """
  def schedule(name, passed_on \\ false) do
    who = User.anyone()

    run(name, fn versions ->
      asked = called(name, versions, {"plugins/schedule", %{plugin: name}}, who, passed_on)

      fire = fn job ->
        with {:error, why} <- call(name, job.tool, job.inputs, who, true),
             do: Logger.error("#{name}'s scheduled #{job.tool} failed: #{why}")

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
  defp scheduled(_name, %Version{jobs: []}), do: :ok

  defp scheduled(name, version) do
    with {:ok, used} <- used(running(), version.uses, %{}),
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
  Runs `fun` with the newest version of a plugin, and of each plugin it uses, by their names. Those versions stay
  loaded until it returns. Gives what `fun` does, or `{:error, why}` with what it raised, as its caller is told it.
  """
  def run(name, fun) do
    with {:ok, held, versions} <- begin(name) do
      try do
        fun.(versions)
      catch
        :throw, {Call, outcome} ->
          outcome

        kind, reason ->
          Logger.error(Exception.format(kind, reason, __STACKTRACE__))
          {blamed, _} = Exception.blame(kind, reason, __STACKTRACE__)
          why = if kind == :error, do: Exception.message(blamed), else: Exception.format_banner(kind, blamed)
          {:error, Enum.reduce(Map.values(versions), why, &Version.own_names(&2, &1))}
      after
        release(held)
      end
    end
  end

  @doc """
  Holds the newest version of a plugin, and of each plugin it uses, loaded: `{:ok, held, versions}`, with the
  versions by their plugins' names, or `{:absent, name}` for the first that isn't running here. They stay loaded
  until `release(held)`, or until the process that asked ends.
  """
  def hold(name), do: GenServer.call(__MODULE__, {:begin, name})

  @doc "Lets go of the versions that `hold/1` held, which are unloaded once nothing runs or holds them."
  def release(held), do: GenServer.call(__MODULE__, {:end, held})

  @doc "The name of a plugin's own artifact."
  def own(name), do: "#{User.anyone()}/own/#{name}"

  @doc "A plugin as a who, which is its artifact's name: the one holding its code."
  def who(name), do: Artifact.name(artifact(name))

  @doc "The plugins running, by name: each one's `Version`, which holds its tools, its guide and its apps."
  def running, do: GenServer.call(__MODULE__, :running)

  @doc "One file of the set of a plugin's files named `code`, which needn't be the set running now."
  def file(name, code, path) do
    with {:ok, text, _tag} <- Artifact.read(artifact(name), "code/#{code}/#{path}"), do: {:ok, text}
  end

  @doc "Looks in the bucket for plugins' newer versions, and runs them. Gives whether there were any."
  def look do
    newer = newer(GenServer.call(__MODULE__, :newest))
    for {name, number, loaded} <- newer, do: takes_over(name, number, loaded)
    newer != []
  end

  # A plugin's newest version takes over here: one to run, or none once it's removed. Its processes follow it,
  # and its services installed here that weren't running start.
  defp takes_over(name, number, loaded) do
    GenServer.call(__MODULE__, {:newest, name, number, loaded})
    # A process whose own code brought the newer version in is the one that can't wait for itself.
    for process <- __MODULE__.Process.all(name), process != self(), do: __MODULE__.Process.follow(process)
    Services.start(name)
    Jobs.look()
  end

  # What a function of a plugin is called with: who it runs for, and what was asked, to pass on. The plugin's own
  # artifact is the one for whoever it was added for, which is the one user every plugin is added for.
  defp called(name, versions, ask, who, passed_on) do
    own = own(name)

    %Call{
      own: %Artifact{prefix: own <> "/"},
      folder: Path.join(Server.home(), own),
      server: Server.key(),
      for: who,
      plugin: name,
      ask: ask,
      uses: Map.new(Map.delete(versions, name), fn {used, version} -> {used, version.plugin} end),
      passed_on: passed_on
    }
  end

  # A call holds the newest version of its plugin and of each plugin that one uses, however far that goes. A
  # plugin this server doesn't run is looked for first.
  defp begin(name) do
    with {:absent, _missing} <- hold(name),
         _ = look(),
         {:absent, missing} <- hold(name) do
      if missing == name,
        do: {:error, "There's no plugin named #{name}"},
        else: {:error, "#{name} needs #{missing}, which isn't in this network"}
    end
  end

  # The plugins a plugin uses are in the network before it is: the ones missing come from what the server ships
  # with.
  defp bring(name, uses) do
    look()

    case Enum.reject(uses, &(is_map_key(running(), &1) or match?({:ok, _, _}, deploy(&1, shipped(&1))))) do
      [] -> :ok
      missing -> {:error, "#{name} uses #{Enum.join(missing, ", ")}, which isn't in this network. Deploy it first."}
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

  defp artifact(name), do: %Artifact{prefix: "#{User.anyone()}/plugins/#{name}/"}

  # Every plugin's newest version by its name, as the bucket has them, and the blob's tag: none if there's no blob.
  defp added do
    case Bucket.read(@added) do
      {:ok, added, tag} -> {JSON.decode!(added), tag}
      :absent -> {%{}, nil}
    end
  end

  # Writes a set of files, and makes it the plugin's next version. No files is the version that removes it.
  defp keep(name, files) do
    code = if files == %{}, do: "", else: Version.code(files)
    for {path, text} <- files, do: Artifact.write(artifact(name), "code/#{code}/#{path}", text)
    keep(name, code, added())
  end

  # The version being replaced goes to the plugin's history first, so every version but the newest is there.
  defp keep(name, code, {added, tag}) do
    was = added[name]
    number = if was, do: was["number"] + 1, else: 1

    if was,
      do: Artifact.write(artifact(name), "versions/" <> String.pad_leading("#{was["number"]}", 8, "0"), was["code"])

    next = JSON.encode!(Map.put(added, name, %{number: number, code: code}))

    case if(tag, do: Bucket.replace(@added, next, tag), else: Bucket.write(@added, next)) do
      :ok ->
        {:ok, number}

      # Another plugin's deploy got there first, and this one goes after it. Another deploy of this plugin did, and
      # whoever asked for this one hasn't seen it.
      _ ->
        {now, _tag} = read = added()

        if now[name] == was,
          do: keep(name, code, read),
          else: {:error, "#{name} was deployed or removed by someone else just now. Look at it, and do this again."}
    end
  end

  # The versions in the bucket that are newer than the ones in `newest`, by plugins' names: each one's name, its
  # number, and the version loaded. That's `{:ok, nil}` for one that removed its plugin, and `:error` for one this
  # server can't compile.
  defp newer(newest) do
    {added, _tag} = added()

    for {name, %{"number" => number, "code" => code}} <- Enum.sort(added), number > Map.get(newest, name, 0) do
      {name, number, loaded(name, code)}
    end
  end

  defp loaded(_name, ""), do: {:ok, nil}

  defp loaded(name, code) do
    plugin = artifact(name)
    paths = Artifact.list(plugin, "code/#{code}/")

    # Read at once: a bucket at an address is asked for each over the internet.
    files =
      paths
      |> Task.async_stream(&Artifact.read(plugin, &1), max_concurrency: 16, timeout: :infinity)
      |> Enum.zip_with(paths, fn {:ok, {:ok, text, _tag}}, path ->
        {String.replace_prefix(path, "code/#{code}/", ""), text}
      end)
      |> Map.new()

    case Version.load(files) do
      {:ok, version, _warnings} ->
        {:ok, version}

      {:error, why} ->
        Logger.error("#{name} isn't updated here, because its newest version doesn't compile:\n#{why}")
        :error
    end
  end

  # A server starts with the newest version of every plugin in the bucket.
  @impl true
  def init(nil) do
    state = %{running: %{}, newest: %{}, calls: %{}}
    Hooks.running(%{})

    {:ok,
     Enum.reduce(newer(%{}), state, fn {name, number, loaded}, state -> took_over(state, name, number, loaded) end)}
  end

  # A plugin's newest version takes over. Deploys that finish out of order still leave the one with the highest
  # number, as the bucket has it.
  @impl true
  def handle_call({:newest, name, number, loaded}, _from, state) do
    if number > Map.get(state.newest, name, 0) do
      was = state.running[name]
      {:reply, :ok, unload_unused(took_over(state, name, number, loaded), [was])}
    else
      {:reply, :ok, unload_unused(state, [with({:ok, version} <- loaded, do: version)])}
    end
  end

  def handle_call(:newest, _from, state), do: {:reply, state.newest, state}

  # The caller is watched, so a call that dies has ended too.
  def handle_call({:begin, name}, {caller, _}, state) do
    case used(state.running, [name], %{}) do
      {:ok, versions} ->
        call = Process.monitor(caller)
        {:reply, {:ok, call, versions}, put_in(state.calls[call], Map.values(versions))}

      absent ->
        {:reply, absent, state}
    end
  end

  def handle_call({:end, call}, _from, state) do
    Process.demonitor(call, [:flush])
    {:reply, :ok, ended(state, call)}
  end

  def handle_call(:running, _from, state), do: {:reply, state.running, state}

  @impl true
  def handle_info({:DOWN, call, :process, _, _}, state), do: {:noreply, ended(state, call)}

  # A version that doesn't compile here leaves the one before it running.
  defp took_over(state, name, number, loaded) do
    running =
      case loaded do
        {:ok, nil} -> Map.delete(state.running, name)
        {:ok, version} -> Map.put(state.running, name, version)
        :error -> state.running
      end

    Hooks.running(running)
    %{state | running: running, newest: Map.put(state.newest, name, number)}
  end

  # The versions a call to `names` holds, by their plugins' names, or the first plugin that isn't running.
  defp used(_running, [], found), do: {:ok, found}
  defp used(running, [name | names], found) when is_map_key(found, name), do: used(running, names, found)

  defp used(running, [name | names], found) do
    case running do
      %{^name => version} -> used(running, version.uses ++ names, Map.put(found, name, version))
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
