defmodule OneCore.Plugins.Calls do
  @moduledoc """
  Calling a plugin: one of its tools, a function of it for a service of its own, its `hook/3` at a hook, or having
  its scheduled jobs called. A call runs on the version of the plugin that was newest when it began, and the ones
  it uses, which stay loaded until it ends (`OneCore.Plugins.run/3`). It runs on a server acting for whoever the
  plugin was installed for: this one, or one it's passed on to.
  """
  require Logger
  alias ExJsonSchema.Validator.Error
  alias OneCore.{Artifact, Call, Hooks, Plugins, Server, Servers, Trace, User}
  alias OneCore.Plugins.Version

  @doc """
  Calls a plugin's tool for `asked`: who asked and who it's for, `{by, for}`, or one who that is both. Gives
  `{:ok, answer}`, or `{:error, why}` with what the caller is told. A call another server ran instead gives
  `{:forwarded, result}`, that server's result. `forwarded` says another server passed this call on to this one,
  and `:forwarding_loop` that this one would pass it on in turn.
  """
  def call(name, tool, inputs, asked, forwarded \\ false) do
    asked = asked(asked)

    case run(name, tool, inputs, asked, forwarded) do
      {:ran, ended} ->
        ended

      # A tool that isn't there is heard as a call that failed, as one refused is.
      {:error, why} ->
        heard(%{"plugin" => name, "tool" => tool, "inputs" => inputs}, asked, fn -> {:error, why} end)

      forwarded ->
        forwarded
    end
  end

  # `{:ran, ended}` where the plugin and the tool are there, `{:error, why}` where they aren't, and what `call/5`
  # gives where another server runs it.
  defp run(name, tool, inputs, {_by, who} = asked, forwarded) do
    Plugins.run(Plugins.chain(who), name, fn at, versions ->
      version = versions[name]

      case Enum.find(version.tools, &(&1.name == tool)) do
        nil ->
          {:error, "#{name} has no tool named #{tool}. It has: #{Enum.map_join(version.tools, ", ", & &1.name)}"}

        found ->
          # What's passed on is what was asked, so the server it's passed to hooks into the same thing.
          ask = {"tools/call", %{name: "#{name}_#{tool}", arguments: inputs}}
          about = %{"plugin" => name, "tool" => tool, "inputs" => inputs}
          # What a plugin answered a hook that's kept, it answered from what its tools change. So after one of those
          # that doesn't only read, it's forgotten: here, and by the servers this one tells.
          kept = if found[:read_only] == true, do: [], else: Enum.filter(Hooks.cached(), &(&1 in version.hooks))

          try do
            call = called(at, name, versions, ask, asked, forwarded)
            answered = tool(found, about, asked, version, &version.plugin.call(tool, &1, call))
            if kept != [], do: Server.notify_servers(Enum.uniq([at | Plugins.chain(who)]), "forget", kept)
            {:ran, answered}
          after
            Hooks.forget(kept)
          end
      end
    end)
  end

  @doc """
  Calls a tool as every tool is called, a plugin's and One Core's own alike. `tool` is what it says of itself,
  `about` is what the call is about, and `fun` is the tool: it's given the inputs, and raises to refuse. The
  inputs are what the tool takes, the plugins that change a call's inputs are asked and the ones that decide it,
  and the ones that hear are told how it went. Gives `{:ok, answer}`, or `{:error, why}` with what the caller is
  told, or what `call/5` does where another server ran it. `apart` is the plugin the call removes or deploys over, as who it was installed for and its name there, which
  isn't asked.
  """
  def tool(tool, about, asked, version \\ nil, fun, apart \\ nil) do
    with :ok <- open_to(tool, about, asked),
         :ok <- given(tool.takes, about["inputs"]),
         {:ok, about} <- hooked(about, tool.takes, asked, apart) do
      heard(about, asked, fn -> Plugins.held(nil, List.wrap(version), fn -> {:ok, fun.(about["inputs"])} end) end)
    else
      # A call refused is heard as one that failed, with why.
      {:error, why} -> heard(about, asked, fn -> {:error, why} end)
    end
  end

  # A plugin is installed for users and groups. A sharing link's holder is neither, and calls only a tool that says
  # they may, of the plugins installed for whoever the link leads to: one that keeps to what the link reaches.
  defp open_to(tool, about, {by, _for}) do
    name = Enum.join(List.wrap(about["plugin"]) ++ [tool.name], "_")

    if by != nil and User.link?(by) and tool[:by_link] != true,
      do: {:error, "#{name} isn't a tool that a sharing link's holder calls"},
      else: :ok
  end

  # What a call is about once the plugins that change its inputs have, if the ones that decide it agree to it.
  defp hooked(about, takes, asked, apart) do
    with {:ok, changes, by} <- Hooks.change("inputs", about, asked, apart),
         about = Map.put(%{about | "inputs" => Map.merge(about["inputs"], changes)}, "changed", by),
         :ok <- if(changes == %{}, do: :ok, else: changed(takes, about)),
         :ok <- Hooks.decide("call", about, asked, apart) do
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

  # Runs a call as a span of its trace, and tells the plugins that hear calls how it went: whether it answered, and
  # whether another server ran it. With none hearing, it's only run.
  defp heard(about, {_by, who} = asked, call) do
    if Hooks.hooking("called", who) == [] do
      call.()
    else
      began = System.monotonic_time(:microsecond)

      Trace.span(fn trace ->
        ended = call.()

        took = System.monotonic_time(:microsecond) - began
        about = Map.merge(about, %{"trace" => trace, "took" => took})

        case ended do
          {:ok, _answer} ->
            Hooks.heard("called", Map.merge(about, %{"error" => nil, "forwarded" => false}), asked)

          {:error, why} ->
            Hooks.heard("called", Map.merge(about, %{"error" => why, "forwarded" => false}), asked)

          {:forwarded, result} ->
            Hooks.heard("called", Map.merge(about, %{"error" => passed(result), "forwarded" => true}), asked)

          # It would have been passed on again, so the server that passed it here hears it.
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
  def function(at, name, function, arguments, asked, forwarded \\ false) do
    asked = asked(asked)

    Plugins.run([at], name, fn at, versions ->
      plugin = versions[name].plugin

      case plugin && exported(plugin, function, length(arguments) + 1) do
        nil ->
          {:error, "#{name} has no function #{function} that takes a call and #{length(arguments)} more"}

        function ->
          ask = {"plugins/call", %{installed: at, plugin: name, function: function, arguments: arguments}}
          {:ok, apply(plugin, function, [called(at, name, versions, ask, asked, forwarded) | arguments])}
      end
    end)
  end

  @doc """
  Calls the `hook/3` of the plugin that `name` is for `at`, at a hook it hooks into, for `asked`. Gives
  `{:ok, answer}` with its answer as `OneCore.Hooks.answer/2` gives it, and otherwise what `call/5` does. A plugin
  that no longer hooks into it has nothing to say. A hook gives no access of its own: a plugin that hears of an
  artifact changing is told only if it could already reach it, and isn't told what it changed itself.
  """
  def hook(at, name, hook, about, asked, forwarded \\ false) do
    asked = asked(asked)

    Plugins.run([at], name, fn at, versions ->
      version = versions[name]

      call =
        called(
          at,
          name,
          versions,
          {"hooks/call", %{installed: at, plugin: name, hook: hook, about: about}},
          asked,
          forwarded
        )

      if hook in version.hooks and hears?(call, about),
        do: {:ok, Hooks.answer(hook, Hooks.within(name, hook, fn -> version.plugin.hook(hook, about, call) end))},
        else: {:ok, Hooks.answer(hook, Hooks.nothing(hook))}
    end)
  end

  @doc """
  Tells the plugin that `name` is for `at` what it heard, each a hook, what it's about and who asked, in turn and
  under one hold of its version, as `hook/6` would. It's told here, where it happened: a plugin that needs a server
  acting for whoever it was installed for passes it there itself (`OneCore.Call.forward/2`), and one that keeps what
  it heard on this server for a while needn't. It has ten seconds for all of it; what it answers or raises is
  ignored.
  """
  def hear(at, name, heard) do
    Plugins.run([at], name, fn at, versions ->
      version = versions[name]

      Hooks.within(name, "what it heard", fn ->
        for {hook, about, asked} <- heard, hook in version.hooks do
          ask = {"hooks/call", %{installed: at, plugin: name, hook: hook, about: about}}
          call = call(at, name, versions, ask, asked, false)

          try do
            if hears?(call, about), do: version.plugin.hook(hook, about, call)
          catch
            :throw, {Call, _forwarded} -> :ok
            kind, reason -> Logger.error(Exception.format(kind, reason, __STACKTRACE__))
          end
        end
      end)

      {:ok, nil}
    end)
  end

  defp hears?(call, %{"artifact" => artifact, "by" => by}),
    do: call.who != by and Call.refused(call, artifact, "viewer") == nil

  defp hears?(_call, _about), do: true

  @doc """
  Has the scheduled jobs that are due called, of the plugin that `name` is for `at`: the plugin it uses that
  keeps schedules is given its jobs, as the plugin, and a function that makes one's call here. That plugin has one
  server make each call, once. The call runs for whoever the plugin was installed for, asked by nobody, and is never
  passed on: it's made where the schedule is kept.
  """
  def schedule(at, name, forwarded \\ false) do
    clock = {nil, at}

    Plugins.run([at], name, fn at, versions ->
      asked = called(at, name, versions, {"plugins/schedule", %{installed: at, plugin: name}}, clock, forwarded)

      fire = fn job ->
        with {:error, why} <- call(name, job.tool, job.inputs, clock, true),
             do: Logger.error("#{name}'s scheduled #{job.tool} for #{at} failed: #{why}")

        Server.changed()
      end

      for keeper <- keepers(Map.delete(versions, name)), do: keeper.schedule(asked, versions[name].jobs, fire)
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

  # What a function of a plugin is called with: who asked, who it runs for, and what was asked, to pass on. The
  # plugin's own artifact is the one for whoever it was installed for, so the call is run by a server acting for them:
  # this one, or one that it's passed on to.
  defp called(at, name, versions, ask, asked, forwarded) do
    call = call(at, name, versions, ask, asked, forwarded)
    if Servers.here?(at), do: call, else: Call.forward(call, at)
  end

  # The call as this server would run it.
  defp call(at, name, versions, ask, {by, who}, forwarded) do
    own = "#{at}/own/#{name}"

    %Call{
      own: %Artifact{prefix: own <> "/"},
      installed: at,
      folder: Path.join(Server.home(), own),
      server: Server.key(),
      for: who,
      by: by,
      plugin: name,
      who: versions[name].artifact,
      ask: ask,
      uses: for({used, version} <- versions, used != name, into: %{}, do: {used, {version.artifact, version.plugin}}),
      forwarded: forwarded
    }
  end

  # Who asked and who for: one who is both.
  defp asked({_by, _for} = asked), do: asked
  defp asked(who) when is_binary(who), do: {who, who}
end
