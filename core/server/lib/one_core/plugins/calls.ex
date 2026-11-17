defmodule OneCore.Plugins.Calls do
  @moduledoc """
  Calling a plugin: one of its tools, a function of it for a service of its own, its `hook/3` at a hook, or having
  its scheduled jobs called. A call runs on the version of the plugin that was latest when it began, and the ones
  it uses, which stay loaded until it ends (`OneCore.Plugins.run/3`). It runs on a server that runs the plugins of
  whoever the plugin was installed for (`OneCore.Servers.runs?/1`): this one, or one it's passed on to.
  """
  require Logger
  alias ExJsonSchema.Validator.Error
  alias OneCore.{Artifact, Call, Hooks, Plugins, Sandbox, Server, Servers, Tasks, Trace, User}
  alias OneCore.Call.State
  alias OneCore.Plugins.Version

  @doc """
  Calls a plugin's tool for `asked`: who asked and who it's for, `{by, for}`, or one who that is both. Gives
  `{:ok, answer}`, or `{:error, why}` with what the caller is told. A call another server ran instead gives
  `{:forwarded, result}`, that server's result. `forwarded` says another server passed this call on to this one,
  and `:forwarding_loop` that this one would pass it on in turn. `caller` is the plugin that called it, if one did
  (`OneCore.Call.tool/5`).
  """
  def call(name, tool, inputs, principal, forwarded \\ false, caller \\ nil) do
    principal = principal(principal)

    State.run(fn ->
      case run(name, tool, inputs, principal, forwarded, caller) do
        {:ran, ended} ->
          ended

        # A tool that isn't there is observed as a call that failed, as one refused is.
        {:error, why} ->
          observed(%{"plugin" => name, "tool" => tool, "inputs" => inputs}, principal, fn -> {:error, why} end)

        forwarded ->
          forwarded
      end
    end)
  end

  # `{:ran, ended}` where the plugin and the tool are there, `{:error, why}` where they aren't, and what `call/5`
  # gives where another server runs it.
  defp run(name, tool, inputs, {_actor, subject} = principal, forwarded, caller) do
    Plugins.run(Plugins.chain(subject), name, fn at, versions ->
      version = versions[name]

      case Enum.find(version.tools, &(&1.name == tool)) do
        nil ->
          {:error, "#{name} has no tool named #{tool}. It has: #{Enum.map_join(version.tools, ", ", & &1.name)}"}

        found ->
          # What's passed on is what was asked, so the server it's passed to hooks into the same thing, and is told
          # which plugin called it.
          ask = {"tools/call", %{name: "#{name}_#{tool}", arguments: inputs, _meta: %{caller: caller}}}
          about = %{"plugin" => name, "tool" => tool, "inputs" => inputs}
          # What a plugin answered a hook that's kept, it answered from what its tools change. So after one of those
          # that doesn't only read, it's forgotten: here, and by the servers this one tells.
          cached = if found[:read_only] == true, do: [], else: Enum.filter(Hooks.cached(), &(&1 in version.hooks))

          try do
            call = %{called(at, name, versions, ask, principal, forwarded) | caller: caller, state: State.id()}
            result = tool(found, about, principal, version, &invoked(versions, name, {:tool, tool, &1}, call))
            if cached != [], do: Server.notify_servers(Enum.uniq([at | Plugins.chain(subject)]), "forget", cached)
            {:ran, result}
          after
            Hooks.forget(cached)
          end
      end
    end)
  end

  @doc """
  Calls a tool as every tool is called, a plugin's and One Core's own alike. `tool` is what it says of itself,
  `about` is what the call is about, and `fun` is the tool: it's given the inputs, and raises to refuse. The
  inputs are what the tool takes, the plugins that mutate a call's inputs are asked and the ones that validate it,
  and the ones that observe are told how it went. Gives `{:ok, answer}`, or `{:error, why}` with what the caller is
  told, or what `call/5` does where another server ran it. `apart` is the plugin the call removes or deploys over, as who it was installed for and its name there, which
  isn't asked.
  """
  def tool(tool, about, principal, version \\ nil, fun, apart \\ nil) do
    with :ok <- open_to(tool, about, principal),
         :ok <- refused(given(tool.takes, about["inputs"]), -32602),
         {:ok, about} <- hooked(about, tool.takes, principal, apart) do
      # It's accepted: a caller that asked for a task is answered it now, and the tool runs.
      State.accepted(State.id(), &Tasks.made/1)

      observed(about, principal, fn ->
        Plugins.held(nil, List.wrap(version), fn -> answered(tool, about, fun.(about["inputs"])) end)
      end)
    else
      # A call refused is observed as one that failed, with why: what it was given is wrong, or who may or a hook
      # said no.
      {:error, why, code} -> refuse(about, principal, code, why)
      {:error, why} -> refuse(about, principal, -32000, why)
    end
  end

  defp refuse(about, principal, code, why) do
    State.refuse(State.id(), code, why)
    observed(about, principal, fn -> {:error, why} end)
  end

  defp refused({:error, why}, code), do: {:error, why, code}
  defp refused(:ok, _code), do: :ok

  # What a tool answers is what its outputs say, so an answer that isn't fails here, loudly, rather than reaching
  # whoever asked in a shape nothing told them of. A tool of a version from before tools said it isn't checked.
  defp answered(%{gives: gives} = tool, about, answer) do
    case ExJsonSchema.Validator.validate(gives, structured(answer), error_formatter: false) do
      :ok ->
        {:ok, answer}

      {:error, wrong} ->
        name = Enum.join(List.wrap(about["plugin"]) ++ [tool.name], "_")
        wrong = Enum.map_join(wrong, "\n", &wrong(&1.error, input(&1.path), gives.schema))
        why = "#{name} answered what its outputs don't say, which is its plugin's to fix:\n#{wrong}"
        Logger.error(why)
        {:error, why}
    end
  end

  defp answered(_tool, _about, answer), do: {:ok, answer}

  @doc """
  An answer as JSON holds it, MCP's `structuredContent`: what a tool answered, or for a file, what kind it is
  (`mimeType`), and for one inside the answer its `size` in bytes.
  """
  def structured({:blob, kind, data}), do: %{"mimeType" => kind, "size" => byte_size(data)}
  def structured({:file, kind, %Artifact{}, _name}), do: %{"mimeType" => kind}
  def structured(answer), do: JSON.decode!(JSON.encode!(answer))

  # A plugin is installed for users and groups. A sharing link's holder is neither, and calls only a tool that says
  # they may, of the plugins installed for whoever the link leads to: one that keeps to what the link reaches.
  defp open_to(tool, about, {by, _for}) do
    name = Enum.join(List.wrap(about["plugin"]) ++ [tool.name], "_")

    if by != nil and User.link?(by) and tool[:by_link] != true,
      do: {:error, "#{name} isn't a tool that a sharing link's holder calls"},
      else: :ok
  end

  # What a call is about once the plugins that mutate its inputs have, if the ones that validate it agree to it.
  defp hooked(about, takes, principal, apart) do
    with {:ok, changes, by} <- Hooks.mutate("inputs", about, principal, apart),
         about = Map.put(%{about | "inputs" => Map.merge(about["inputs"], changes)}, "changed", by),
         :ok <- if(changes == %{}, do: :ok, else: changed(takes, about)),
         :ok <- Hooks.validate("call", about, principal, apart) do
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

  # Runs a call as a span of its trace, and tells the plugins that observe calls how it went: whether it answered,
  # and whether another server ran it. With none observing, it's only run.
  defp observed(about, {_actor, subject} = principal, call) do
    if Hooks.hooking("called", subject) == [] do
      call.()
    else
      began = System.monotonic_time(:microsecond)

      Trace.span(fn trace ->
        ended = call.()

        took = System.monotonic_time(:microsecond) - began
        about = Map.merge(about, %{"trace" => trace, "took" => took})

        case ended do
          {:ok, _result} ->
            Hooks.observe("called", Map.merge(about, %{"error" => nil, "forwarded" => false}), principal)

          {:error, why} ->
            Hooks.observe("called", Map.merge(about, %{"error" => why, "forwarded" => false}), principal)

          {:forwarded, result} ->
            Hooks.observe("called", Map.merge(about, %{"error" => passed(result), "forwarded" => true}), principal)

          # It would have been passed on again, so the server that passed it here observes it.
          :forwarding_loop ->
            :ok
        end

        ended
      end)
    end
  end

  # Why a call another server ran failed, or `nil` if it answered.
  defp passed(%{"isError" => true, "content" => [%{"text" => why} | _]}), do: why
  defp passed(_result), do: nil

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
  def function(at, name, function, arguments, principal, forwarded \\ false) do
    principal = principal(principal)

    Plugins.run([at], name, fn at, versions ->
      ask = {"plugins/call", %{installed: at, plugin: name, function: function, arguments: arguments}}
      call = called(at, name, versions, ask, principal, forwarded)
      plugin = versions[name].plugin

      case plugin && exported(plugin, function, length(arguments) + 1) do
        nil -> {:error, "#{name} has no function #{function} that takes a call and #{length(arguments)} more"}
        function -> {:ok, running(call, fn -> apply(plugin, function, [call | arguments]) end)}
      end
    end)
  end

  @doc """
  Calls the `hook/3` of the plugin that `name` is for `at`, at `hook`, a hook of `kind`, for `asked`. Gives
  `{:ok, answer}` with its answer as `OneCore.Hooks.answer/3` gives it, and otherwise what `call/5` does. A plugin
  that no longer hooks into it has nothing to say. A hook gives no access of its own: a plugin that observes an
  artifact changing is told only if it could already reach it, and isn't told what it changed itself.
  """
  def hook(at, name, kind, hook, about, principal, forwarded \\ false) do
    principal = principal(principal)

    Plugins.run([at], name, fn at, versions ->
      version = versions[name]
      ask = {"hooks/call", %{installed: at, plugin: name, kind: kind, hook: hook, about: about}}
      call = called(at, name, versions, ask, principal, forwarded)

      # An untrusted plugin only observes: it mutates, validates and collects nothing.
      if hook in version.hooks and observes?(call, about) and (kind == :observe or version.trusted),
        do:
          {:ok,
           Hooks.check_result(
             kind,
             hook,
             Hooks.within(name, hook, fn -> invoked(versions, name, {:hook, hook, about}, call) end)
           )},
        else: {:ok, Hooks.check_result(kind, hook, Hooks.nothing(kind))}
    end)
  end

  @doc """
  Tells the plugin that `name` is for `at` what it observed, each a hook, what it's about and who asked, in turn and
  under one hold of its version, as `hook/6` would. It's told here, where it happened: a plugin that needs a server
  acting for whoever it was installed for passes it there itself (`OneCore.Call.forward/2`), and one that keeps what
  it observed on this server for a while needn't. One that only runs elsewhere is told there. It has ten seconds for all
  of it; what it answers or raises is ignored.
  """
  def observe(at, name, observed) do
    Plugins.run([at], name, fn at, versions ->
      version = versions[name]

      Hooks.within(name, "what it observed", fn ->
        for {hook, about, principal} <- observed, hook in version.hooks do
          ask = {"hooks/call", %{installed: at, plugin: name, kind: :observe, hook: hook, about: about}}

          call =
            if Version.described?(version),
              do: called(at, name, versions, ask, principal, false),
              else: new_call(at, name, versions, ask, principal, false)

          try do
            if observes?(call, about), do: invoked(versions, name, {:hook, hook, about}, call)
          catch
            :throw, {Call, _forwarded} -> :ok
            kind, reason -> Logger.error(OneCore.Redact.format(kind, reason, __STACKTRACE__))
          end
        end
      end)

      {:ok, nil}
    end)
  end

  defp observes?(call, %{"artifact" => artifact, "by" => by}),
    do: call.who != by and Call.refused(call, artifact, "viewer") == nil

  defp observes?(_call, _about), do: true

  @doc """
  Has the scheduled jobs that are due called, of the plugin that `name` is for `at`: the plugin it uses that
  keeps schedules is given its jobs, as the plugin, and a function that makes one's call here. That plugin has one
  server make each call, once. The call runs for whoever the plugin was installed for, asked by nobody, and is never
  passed on: it's made where the schedule is kept.
  """
  def schedule(at, name, forwarded \\ false) do
    clock = {nil, at}

    Plugins.run([at], name, fn at, versions ->
      scheduled = called(at, name, versions, {"plugins/schedule", %{installed: at, plugin: name}}, clock, forwarded)

      fire = fn job ->
        with {:error, why} <- call(name, job.tool, job.inputs, clock, true),
             do: Logger.error("#{name}'s scheduled #{job.tool} for #{at} failed: #{why}")

        Server.changed()
      end

      # An untrusted plugin has its jobs kept in its sandbox, by the plugin it uses there.
      if versions[name].trusted,
        do: for(keeper <- keepers(Map.delete(versions, name)), do: keeper.schedule(scheduled, versions[name].jobs, fire)),
        else: Sandbox.call(versions, name, {:schedule, versions[name].jobs}, scheduled)

      {:ok, :ok}
    end)
  end

  @doc "The plugins among `versions`, by their names, that keep other plugins' schedules."
  def keepers(versions) do
    for {_name, %Version{plugin: plugin}} <- versions,
        plugin != nil,
        function_exported?(plugin, :schedule, 3),
        do: plugin
  end

  @doc "The function of `module` named by the text `name` that takes `arity` arguments, or `nil` if it has none."
  def exported(module, name, arity) do
    Enum.find_value(module.__info__(:functions), fn {function, takes} ->
      if takes == arity and Atom.to_string(function) == name, do: function
    end)
  end

  # A plugin's tool or hook, run by its module here or, for an untrusted plugin, in its sandbox.
  defp invoked(versions, name, what, call) do
    running(call, fn ->
      case {versions[name], what} do
        {%Version{trusted: false}, what} -> Sandbox.call(versions, name, what, call)
        {version, {:tool, tool, inputs}} -> version.plugin.call(tool, inputs, call)
        {version, {:hook, hook, about}} -> version.plugin.hook(hook, about, call)
      end
    end)
  end

  @doc """
  Runs `fun`, a plugin's code for `call`, with the process saying so in its Logger metadata: the plugin as a who
  (`who`), whoever it was `installed` for and whoever the call runs for (`subject`). Every call of a plugin runs in
  it. It's how what a process logs, or is traced doing, is told apart by whose call it was, which only the call
  knows: the plugin `server-debug` reads that metadata, and traces this function to hear a call begin and end.
  """
  def running(%Call{} = call, fun) do
    was = Logger.metadata()
    Logger.metadata(who: call.who, installed: call.installed, subject: call.subject)

    try do
      fun.()
    after
      Logger.reset_metadata(was)
    end
  end

  # What a function of a plugin is called with: who asked, who it runs for, and what was asked, to pass on. It's run
  # by a server that runs the plugins of whoever it was installed for, or by any server acting for them where the
  # plugin is untrusted: this one, or one that it's passed on to.
  defp called(at, name, versions, ask, principal, forwarded) do
    call = new_call(at, name, versions, ask, principal, forwarded)

    if Servers.runs?(at) or (not versions[name].trusted and Servers.here?(at)),
      do: call,
      else: Call.run_elsewhere(call, at)
  end

  # The call as this server would run it.
  defp new_call(at, name, versions, ask, {actor, subject}, forwarded) do
    own = "#{at}/own/#{name}"

    %Call{
      own: %Artifact{prefix: own <> "/"},
      installed: at,
      folder: Path.join(Server.home(), own),
      server: Server.key(),
      subject: subject,
      actor: actor,
      plugin: name,
      who: versions[name].artifact,
      request: ask,
      uses: for({used, version} <- versions, used != name, into: %{}, do: {used, {version.artifact, version.plugin}}),
      forwarded: forwarded
    }
  end

  # Who asked and who for: one who is both.
  defp principal({_by, _for} = principal), do: principal
  defp principal(who) when is_binary(who), do: {who, who}
end
