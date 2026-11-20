defmodule OneCore.Apps.Calls do
  @moduledoc """
  Calling an app: one of its tools, a function of it for a service of its own, its `hook/3` at a hook, or having its
  scheduled jobs called. A call runs on the version of the app that was latest when it began, and those of the
  apps it requires whose code runs inside it, which stay loaded until it ends (`OneCore.Apps.run/3`). It runs on a
  server that runs the apps of whoever the app was installed for (`OneCore.Apps.runs?/1`): this one, or one
  it's passed on to.
  """
  require Logger
  alias ExJsonSchema.Validator.Error
  alias OneCore.{Artifact, Call, Hooks, Apps, Sandbox, Server, Tasks, User}
  alias OpenTelemetryAPIExperimental.Histogram
  require OpenTelemetry.Tracer, as: Tracer
  require Histogram
  alias OneCore.Call.State
  alias OneCore.Apps.{Installed, Version}

  @doc """
  Calls an app's tool for `asked`: who asked and who it's for, `{by, for}`, or one who that is both. Gives
  `{:ok, answer}`, or `{:error, why}` with what the caller is told. A call another server ran instead gives
  `{:forwarded, result}`, that server's result. `forwarded` says another server passed this call on to this one,
  and `:forwarding_loop` that this one would pass it on in turn. `caller` is the app that called it, if one did
  (`OneCore.Call.tool/5`).
  """
  def call(name, tool, inputs, principal, forwarded \\ false, caller \\ nil) do
    principal = principal(principal)

    State.run(fn ->
      case run(name, tool, inputs, principal, forwarded, caller) do
        {:ran, ended} ->
          ended

        # A tool that isn't there is observed as a call refused, as one whose inputs are wrong is.
        {:error, why} ->
          observed(%{"app" => name, "tool" => tool, "inputs" => inputs}, principal, fn -> {:refused, why} end)

        forwarded ->
          forwarded
      end
    end)
  end

  # `{:ran, ended}` where the app and the tool are there, `{:error, why}` where they aren't, and what `call/5`
  # gives where another server runs it.
  defp run(name, tool, inputs, {_actor, subject} = principal, forwarded, caller) do
    Apps.run(Apps.chain(subject), name, fn at, versions ->
      version = versions[name]

      case Enum.find(version.tools, &(&1.name == tool)) do
        nil ->
          {:error, "#{name} has no tool named #{tool}. It has: #{Enum.map_join(version.tools, ", ", & &1.name)}"}

        found ->
          # What's passed on is what was asked, so the server it's passed to hooks into the same thing, and is told
          # which app called it.
          ask = {"tools/call", %{name: "#{name}_#{tool}", arguments: inputs, _meta: %{caller: caller}}}
          about = %{"app" => name, "tool" => tool, "inputs" => inputs}
          # What an app answered a hook that's kept, it answered from what its tools change. So after one of those
          # that doesn't only read, it's invalidated: here, and by the servers this one tells.
          cached = if found[:read_only] == true, do: [], else: Enum.filter(Hooks.cached(), &(&1 in version.hooks))

          if why = member_refused(found, name, principal, caller) do
            {:ran, refuse(about, principal, -32000, why)}
          else
            try do
              call = %{called(at, name, versions, ask, principal, forwarded) | caller: caller, state: State.id()}
              result = tool(found, about, principal, Map.values(versions), &invoked(versions, name, {:tool, tool, &1}, call))
              if cached != [], do: Server.notify_servers(Enum.uniq([at | Apps.chain(subject)]), "invalidate", cached)
              {:ran, result}
            after
              Hooks.invalidate(cached)
            end
          end
      end
    end)
  end

  # A member asking for a group or the network runs an app's tool only as far as they may themselves: one that only
  # reads as a viewer of what the app made for them (`<them>/made/<app>`), any other as an editor, as the apps that
  # validate `reach` say. It's asked here, before the tool runs, since the app reaches what it makes for whoever it
  # runs for without asking (`OneCore.Call.make/2`) and one database holds it for every call of theirs. An app
  # calling another's tool answers for what it asks itself.
  defp member_refused(tool, name, {actor, subject}, caller) do
    if actor != nil and actor != subject and caller == nil do
      relation = if tool[:read_only] == true, do: "viewer", else: "editor"
      reach = %{"who" => actor, "relation" => relation, "artifact" => "#{subject}/made/#{name}"}

      case Hooks.validate("reach", reach, {actor, subject}) do
        :ok -> nil
        {:refuse, why} -> why
      end
    end
  end

  @doc """
  The tool `name` is for `who`: One Core's own, one of `own` (as `OneCore.Tools.Definitions` says them, or with their
  schemas read, as `OneCore.Tools` lists them), or an app's by its name and the app's, or `nil`.
  """
  def find(name, who, own) do
    Enum.find(own, &(&1.name == name)) ||
      with [app, tool] <- String.split(name, "_", parts: 2),
           %{^app => version} <- Apps.running(who) do
        Enum.find(version.tools, &(&1.name == tool))
      else
        _ -> nil
      end
  end

  @doc "Whether calling the tool `name` for `who`, as `find/3` finds it, only reads: it says so."
  def reads?(name, who), do: find(name, who, OneCore.Tools.Definitions.tools())[:read_only] == true

  @doc """
  Calls a tool as every tool is called, an app's and One Core's own alike. `tool` is what it says of itself,
  `about` is what the call is about, and `fun` is the tool: it's given the inputs, and raises to refuse. The
  inputs are what the tool takes, the apps that mutate a call's inputs are asked and the ones that validate it,
  and the ones that observe are told how it went. Gives `{:ok, answer}`, or `{:error, why}` with what the caller is
  told, or what `call/5` does where another server ran it. What it raises is told in the names `versions` wrote,
  the versions of apps its code is of (`OneCore.Apps.held/2`): none for One Core's own.
  """
  def tool(tool, about, principal, versions, fun) do
    about = Map.put(about, "task", tool[:task_support] != nil)

    with :ok <- open_to(tool, about, principal),
         :ok <- refused(given(tool.takes, about["inputs"]), -32602),
         {:ok, about} <- hooked(about, tool.takes, principal) do
      # It's accepted: a caller that asked for a task is answered it now, and the tool runs.
      State.accepted(State.id(), &Tasks.made/1)

      observed(about, principal, fn ->
        Apps.held(versions, fn ->
          try do
            answered(tool, about, fun.(about["inputs"]))
          rescue
            refused in Call.Refused -> {:refused, Exception.message(refused)}
          end
        end)
      end)
    else
      # A call refused is observed as refused, with why: what it was given is wrong, or who may or a hook said no.
      {:error, why, code} -> refuse(about, principal, code, why)
      {:error, why} -> refuse(about, principal, -32000, why)
    end
  end

  defp refuse(about, principal, code, why) do
    State.refuse(State.id(), code, why)
    observed(about, principal, fn -> {:refused, why} end)
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
        name = Enum.join(List.wrap(about["app"]) ++ [tool.name], "_")
        wrong = Enum.map_join(wrong, "\n", &wrong(&1.error, input(&1.path), gives.schema))
        why = "#{name} answered what its outputs don't say, which is its app's to fix:\n#{wrong}"
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

  # One Core's own answer with MCP content after it, as code mode's `call` gives what its calls showed (`OneCore.CodeMode`).
  def structured({:content, answer, _parts}), do: structured(answer)
  def structured({:file, kind, %Artifact{}, _name}), do: %{"mimeType" => kind}
  def structured(answer), do: OneCore.JSON.normalize(answer)

  # An app is installed for users and groups. A sharing link's holder is neither, and calls only a tool that says
  # they may, of the apps installed for whoever the link leads to: one that keeps to what the link reaches.
  defp open_to(tool, about, {by, _for}) do
    name = Enum.join(List.wrap(about["app"]) ++ [tool.name], "_")

    if by != nil and User.link?(by) and tool[:by_link] != true,
      do: {:error, "#{name} isn't a tool that a sharing link's holder calls"},
      else: :ok
  end

  # What a call is about once the apps that mutate its inputs have, if the ones that validate it agree to it.
  defp hooked(about, takes, principal) do
    with {:ok, changes, by} <- Hooks.mutate("inputs", about, principal),
         about = Map.put(%{about | "inputs" => Map.merge(about["inputs"], changes)}, "changed", by),
         :ok <- if(changes == %{}, do: :ok, else: changed(takes, about)),
         :ok <- Hooks.validate("call", about, principal) do
      {:ok, about}
    else
      {:refuse, why} -> {:error, why}
      {:error, why} -> {:error, why}
    end
  end

  # A tool is only ever called with what it takes, whoever changed its inputs.
  defp changed(takes, %{"inputs" => inputs, "changed" => by}) do
    with {:error, wrong} <- given(takes, inputs) do
      changed = Enum.map_join(Enum.sort(by), ", ", fn {input, app} -> "#{input} by #{app}" end)
      {:error, "Its inputs were changed (#{changed}), and it doesn't take them now:\n#{wrong}"}
    end
  end

  # Runs a call as a span of its trace, OpenTelemetry's, which the SDK exports (`OneCore.Otlp`), with how long it took
  # as `one.tool.duration`, unless another server ran it, which measures it there. Its span says the app, the tool, whose call it was as an owner's
  # prefix (`users/alice`), and how it ended: never what it was given or answered. One refused before it ran (`call`
  # gives `{:refused, why}`) is the caller's mistake, not the tool's, as OpenTelemetry has a server's span for an RPC
  # refused for its arguments or who asked: its outcome is `refused`, and its span isn't an error.
  defp observed(about, {_actor, subject}, call) do
    app = about["app"] || "one"
    attributes = %{"one.app" => app, "one.tool" => about["tool"]}
    attributes = if about["task"] == true, do: Map.put(attributes, "one.task", true), else: attributes
    began = System.monotonic_time(:microsecond)

    Tracer.with_span "#{app} #{about["tool"]}", %{kind: :server, attributes: Map.put(attributes, "one.for", User.owner(subject) || "link")} do
      ended = call.()
      took = System.monotonic_time(:microsecond) - began

      {error, forwarded} =
        case ended do
          {:ok, _result} -> {nil, false}
          {:refused, why} -> {{:refused, why}, false}
          {:refused, why, _details} -> {{:refused, why}, false}
          {:error, why} -> {why, false}
          {:forwarded, result} -> {passed(result), true}
          # It would have been passed on again, so the server that passed it here observes it.
          :forwarding_loop -> {nil, nil}
        end

      outcome =
        case error do
          nil -> "ok"
          {:refused, _why} -> "refused"
          why ->
            Tracer.set_status(:error, String.slice(why, 0, 1000))
            "error"
        end

      if forwarded != nil do
        if forwarded, do: Tracer.set_attribute("one.forwarded", true)
        if outcome == "refused", do: Tracer.set_attribute("one.outcome", outcome)
        if not forwarded, do: Histogram.record(:"one.tool.duration", took / 1000, Map.put(attributes, "one.outcome", outcome))
      end

      case ended do
        {:refused, why} -> {:error, why}
        {:refused, why, details} -> {:error, why, details}
        other -> other
      end
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
  Calls a function of the app that `name` is for `at`, for a service of its own, with a call for `asked` and
  then `arguments`. Gives what `call/5` does.
  """
  def function(at, name, function, arguments, principal, forwarded \\ false) do
    principal = principal(principal)

    Apps.run([at], name, fn at, versions ->
      ask = {"apps/call", %{installed: at, app: name, function: function, arguments: arguments}}
      call = called(at, name, versions, ask, principal, forwarded)
      app = versions[name].app

      case app && exported(app, function, length(arguments) + 1) do
        nil -> {:error, "#{name} has no function #{function} that takes a call and #{length(arguments)} more"}
        function -> {:ok, running(call, fn -> apply(app, function, [call | arguments]) end)}
      end
    end)
  end

  @doc """
  Calls the `hook/3` of the app that `name` is for `at`, at `hook`, a hook of `kind`, for `asked`. Gives
  `{:ok, answer}` with its answer as `OneCore.Hooks.answer/3` gives it, and otherwise what `call/5` does. An app
  that no longer hooks into it has nothing to say. A hook gives no access of its own: an app that observes an
  artifact changing is told only if it could already reach it, and isn't told what it changed itself.
  """
  def hook(at, name, kind, hook, about, principal, forwarded \\ false) do
    principal = principal(principal)

    Apps.run([at], name, fn at, versions ->
      version = versions[name]
      ask = {"hooks/call", %{installed: at, app: name, kind: kind, hook: hook, about: about}}
      call = called(at, name, versions, ask, principal, forwarded)

      # An untrusted app observes, and answers the collects whose app says it may (`Hooks.untrusted?/2`). It
      # mutates and validates nothing, which would change or refuse what others do, and answers no other collect,
      # whose app may act on what it's given (Google's scopes a person grants, a screen agents type into).
      {_actor, subject} = principal

      if hook in version.hooks and observes?(call, about) and
           (version.trusted or kind == :observe or (kind == :collect and Hooks.untrusted?(hook, Apps.chain(subject)))),
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
  Tells the app that `name` is for `at` what it observed, each a hook, what it's about and who asked, in turn and
  under one hold of its version, as `hook/6` would. It's told here, where it happened: an app that needs a server
  acting for whoever it was installed for passes it there itself (`OneCore.Call.forward/2`), and one that keeps what
  it observed on this server for a while needn't. One that only runs elsewhere is told there. It has ten seconds for
  each, however many waited (`OneCore.Hooks.within/3`), so a server that comes back from sleep with minutes of them
  waiting tells each; what it answers or raises is ignored.
  """
  def observe(at, name, observed) do
    Apps.run([at], name, fn at, versions ->
      version = versions[name]

      for {hook, about, principal} <- observed, hook in version.hooks do
        ask = {"hooks/call", %{installed: at, app: name, kind: :observe, hook: hook, about: about}}

        call = called(at, name, versions, ask, principal, false)

        # Each is told apart from the others: what one raised is logged, and one it passed on ended there.
        Apps.held(Map.values(versions), fn ->
          Hooks.within(name, hook, fn ->
            if observes?(call, about), do: invoked(versions, name, {:hook, hook, about}, call)
          end)
        end)
      end

      {:ok, nil}
    end)
  end

  defp observes?(call, %{"artifact" => artifact, "by" => by}),
    do: call.app_artifact != by and Call.refused(call, artifact, "viewer") == nil

  defp observes?(_call, _about), do: true

  @doc """
  Has the scheduled jobs that are due called, of the app that `name` is for `at`, for `subject`: whoever it was
  installed for, or one who keeps their own data with it (`whose/3`). The app it requires that keeps schedules is
  given its jobs, as the app, and a function that makes one's call here. That app has one server make each call,
  once. The call runs for `subject`, asked by nobody, and is never passed on: it's made where the schedule is kept.

  An untrusted app's schedule is kept the same way, by the keeper's code here, never in its sandbox: only its calls
  run there, each as any call of its tools does. The keeper keeps it in an artifact of its own for the app, not the
  app's own artifact, which the app writes, so what this server reads of it is nothing the app wrote.
  """
  def schedule(at, name, subject, forwarded \\ false) do
    clock = {nil, subject}

    Apps.run([at], name, fn at, versions ->
      version = versions[name]
      scheduled = called(at, name, versions, {"apps/schedule", %{installed: at, app: name}}, clock, forwarded)
      scheduled = if version.trusted, do: scheduled, else: apart(scheduled, name)

      fire = &fire(at, name, subject, &1)
      for keeper <- keepers(Map.delete(versions, name)), do: keeper.schedule(scheduled, version.jobs, fire)

      {:ok, :ok}
    end)
  end

  # Where an untrusted app's schedule is kept: in the sandbox app's own artifact for whoever its own is, a folder for
  # each app, which no untrusted app is given.
  defp apart(%Call{own: own} = call, name) do
    own = "#{User.owner(Artifact.name(own))}/own/sandbox/#{name}"
    %{call | own: %Artifact{prefix: own <> "/"}, folder: Path.join(Server.home(), own)}
  end

  # Makes a scheduled job's call, the `tool` of the app that `name` is for `at` with its `inputs`, as the clock: for
  # `subject`, asked by nobody, and never passed on. What went wrong is logged. Nobody is told it ran: a job that
  # changes what a screen shows says so itself (`OneCore.Call.changed/1`).
  defp fire(at, name, subject, job) do
    with {:error, why} <- call(name, job.tool, job.inputs, {nil, subject}, true),
         do: Logger.error("#{name}'s scheduled #{job.tool} for #{subject} (installed for #{at}) failed: #{why}")
  end

  @doc "The apps among `versions`, by their names, that keep other apps' schedules."
  def keepers(versions) do
    for {_name, %Version{app: app}} <- versions,
        app != nil,
        function_exported?(app, :schedule, 3),
        do: app
  end

  @doc "The function of `module` named by the text `name` that takes `arity` arguments, or `nil` if it has none."
  def exported(module, name, arity) do
    Enum.find_value(module.__info__(:functions), fn {function, takes} ->
      if takes == arity and Atom.to_string(function) == name, do: function
    end)
  end

  # An app's tool or hook, run by its module here or, for an untrusted app, in its sandbox.
  defp invoked(versions, name, what, call) do
    running(call, fn ->
      case {Version.runs(versions[name]), what} do
        {:sandbox, what} -> Sandbox.call(versions, name, what, call)
        {:here, {:tool, tool, inputs}} -> versions[name].app.call(tool, inputs, call)
        {:here, {:hook, hook, about}} -> versions[name].app.hook(hook, about, call)
      end
    end)
  end

  @doc """
  Runs `fun`, an app's code for `call`, with the process saying so in its Logger metadata: the app as a who
  (`who`), whoever it was `installed` for and whoever the call runs for (`subject`). Every call of an app runs in
  it. It's how what a process logs, or is traced doing, is told apart by whose call it was, which only the call
  knows: the app `server-debug` reads that metadata, and traces this function to hear a call begin and end.
  """
  def running(%Call{} = call, fun) do
    was = Logger.metadata()
    Logger.metadata(app_artifact: call.app_artifact, installed: call.installed, subject: call.subject)

    try do
      fun.()
    after
      Logger.reset_metadata(was)
    end
  end

  # What a function of an app is called with: who asked, who it runs for, and what was asked, to pass on. It's run
  # by a server that runs the apps of whoever it was installed for, or by any server acting for them where the
  # app is untrusted: this one, or one that it's passed on to.
  defp called(at, name, versions, ask, principal, forwarded) do
    call = new_call(at, name, versions, ask, principal, forwarded)

    if Apps.runs?(at, versions[name].trusted),
      do: call,
      else: Call.run_elsewhere(call, at)
  end

  # The call as this server would run it. One run for someone who keeps their own data apart from whoever the app was
  # installed for is said to be theirs where it runs, so its jobs and services act for them too.
  defp new_call(at, name, versions, ask, {actor, subject}, forwarded) do
    whose = whose(at, versions[name], subject)

    if whose != at and Apps.runs?(at, versions[name].trusted), do: Installed.subject(at, name, whose)

    own = "#{whose}/own/#{name}"

    %Call{
      own: %Artifact{prefix: own <> "/"},
      installed: at,
      folder: Path.join(Server.home(), own),
      server: Server.key(),
      subject: subject,
      actor: actor,
      app: name,
      app_artifact: versions[name].artifact,
      request: ask,
      loaded:
        for({other, version} <- versions, other != name, into: %{}, do: {other, {version.artifact, version.app}}),
      forwarded: forwarded
    }
  end

  @doc """
  Whose own artifact an app installed for `at` keeps, in a call for `subject`, by what its `version` says (`own/0`):
  whoever the call runs for, a user, a group or the network, so one installed for the network keeps each person's data
  apart, as an Android app installed for every user of a device does; or, for one that says `:installed`, whoever it
  was installed for, everyone's together, as the network's Permissions and Backups are. They differ only for an app
  installed for the network: one installed for a user or a group runs only for them.
  """
  def whose(at, %Version{own: :installed}, _subject), do: at
  def whose(at, %Version{own: :subject}, subject), do: User.owner(subject) || at

  # Who asked and who for: one who is both.
  defp principal({_by, _for} = principal), do: principal
  defp principal(who) when is_binary(who), do: {who, who}
end
