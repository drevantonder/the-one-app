defmodule OneCore.App.Manifest do
  @moduledoc """
  What an app says of itself, its manifest, and what One Core offers it: the functions an app defines
  (`OneCore.App`) with what each gives where an app defines none, the hooks it can hook into, and what an
  untrusted app is offered in its sandbox. Each is said here once, and everything that needs one reads it from here:
  `OneCore.App` and its skill, the checks a version passes, what a server keeps of a version in the bucket, and an
  untrusted app's VM (`OneCore.Sandbox`).

  It depends on nothing of One Core's: both the server and an untrusted app's VM compile this file (`mix.exs`,
  `OneCore.Sandbox`).

  A manifest's JSON form is the one shape it has outside a module: what an untrusted app's VM says, and what a
  server keeps of a version in the bucket. `decode/1` takes it back.
  """

  # What an app says of itself, by the functions that say it, with what each gives where an app has none.
  @said [
    tools: [],
    entrypoints: [],
    icon: nil,
    widgets: [],
    hooks: [],
    offers: [],
    jobs: [],
    requires: %{},
    own: :subject
  ]

  # The keys of each map in a list an app says, as JSON names them.
  @keys %{
    tools: ~w(name description inputs outputs read_only by_link ui runtime_version task_support)a,
    entrypoints: ~w(name page icon category subject badge runtime_version)a,
    widgets: ~w(name tool sizes)a,
    offers: ~w(name kind description untrusted)a,
    jobs: ~w(tool every inputs)a
  }

  # Every function an app may define, each optional, which the compiler holds `callbacks/0` to.
  @callbacks [
    tools: 0,
    call: 3,
    entrypoints: 0,
    widgets: 0,
    hooks: 0,
    hook: 3,
    offers: 0,
    jobs: 0,
    schedule: 3,
    icon: 0,
    requires: 0,
    own: 0,
    sensitive: 0
  ]

  @task_support [:optional, :required]
  @owns [:subject, :installed]
  @hook_kinds [:observe, :mutate, :validate, :collect]

  @doc """
  Defines `OneCore.App`'s callbacks, in the server's `OneCore.App` and in an untrusted app's VM's, which an app is
  compiled against there.
  """
  defmacro callbacks do
    quote do
      @callback tools() :: [%{name: String.t(), description: String.t(), inputs: map, outputs: map}]
      @callback call(tool :: String.t(), inputs :: map, OneCore.Call.t()) :: term
      @callback entrypoints() :: [%{required(:page) => String.t(), optional(atom) => String.t()}]
      @callback widgets() :: [%{name: String.t(), tool: String.t(), sizes: [String.t()]}]
      @callback hooks() :: [String.t()]
      @callback hook(hook :: String.t(), about :: map, OneCore.Call.t()) :: term
      @callback offers() :: [
                  %{name: String.t(), kind: :observe | :mutate | :validate | :collect, description: String.t()}
                ]
      @callback jobs() :: [%{tool: String.t(), every: pos_integer}]
      @callback schedule(OneCore.Call.t(), jobs :: [map], fire :: (map -> term)) :: term
      @callback icon() :: String.t()
      @callback requires() :: %{String.t() => String.t()}
      @callback own() :: :subject | :installed
      @callback sensitive() :: boolean
      @optional_callbacks unquote(@callbacks)
    end
  end

  @doc "Defines `OneCore.Service`'s callbacks, as `callbacks/0` does `OneCore.App`'s."
  defmacro service_callbacks do
    quote do
      @doc "What the service needs of a machine, for its owner to read before installing it."
      @callback needs() :: String.t()

      @doc "What the service is called, in words its owner reads: its app's name, and what it is of it."
      @callback called() :: String.t()
    end
  end

  @doc "The functions an app says itself with, by name, as `said/1` reads them."
  def said, do: Keyword.keys(@said)

  @doc """
  What the app's module `app` says of itself: each function's answer, or what it gives where the app has none, and
  its `exports`. Raises what the app's own functions raise.
  """
  def said(app) do
    for {function, none} <- @said,
        into: %{exports: exports(app)},
        do: {function, if(function_exported?(app, function, 0), do: apply(app, function, []), else: none)}
  end

  # The functions an app's module provides other apps, which run inside each app that requires it
  # (`OneCore.App`): its public ones besides the callbacks, as name/arity.
  defp exports(app) do
    Enum.sort(
      for {name, arity} <- app.__info__(:functions),
          {name, arity} not in @callbacks,
          not String.starts_with?(Atom.to_string(name), "__"),
          do: "#{name}/#{arity}"
    )
  end

  @doc """
  What an app said of itself, from its JSON form, with its names as `said/1` gives them: a map's keys an app writes
  as atoms, and a hook's kind and a tool's `task_support`, which JSON holds as text. What isn't one of those is left as
  it is, for the checks to refuse, and anything else is left out. `exports` is kept as it is.
  """
  def decode(%{} = json) do
    for {function, none} <- [{:exports, []} | @said], into: %{} do
      value = Map.get(json, Atom.to_string(function), none)
      {function, decode(function, value)}
    end
  end

  defp decode(function, list) when is_map_key(@keys, function) and is_list(list) do
    for item <- list do
      if is_map(item),
        do: atoms(for(key <- @keys[function], is_map_key(item, "#{key}"), into: %{}, do: {key, item["#{key}"]})),
        else: item
    end
  end

  defp decode(:own, own) when is_binary(own), do: atom(own, @owns)
  defp decode(_function, value), do: value

  defp atoms(%{kind: kind} = offer) when is_binary(kind), do: %{offer | kind: atom(kind, @hook_kinds)}

  defp atoms(%{task_support: support} = tool) when is_binary(support),
    do: %{tool | task_support: atom(support, @task_support)}

  defp atoms(map), do: map

  defp atom(text, atoms), do: Enum.find(atoms, text, &(Atom.to_string(&1) == text))

  @kinds [
    observe: """
    is told after it happened, on the server where it happened, apart from what happened and from every other
    app that observes it, in the order it happened. What it answers is ignored, and so is its raising. One that
    needs a server acting for whoever it was installed for passes it there (`OneCore.Call.forward/2`).
    """,
    mutate: """
    is handed it as it was asked, as every other app that mutates it is, and answers only what it changes, by
    name: `%{}` changes nothing. Two apps that change the same thing to something different refuse it, and say
    so. Which app changed what is said to the apps that validate and observe it.
    """,
    validate: """
    answers `:ok` or `{:refuse, why}`. Every app that validates it has to answer `:ok`: one refusal refuses it,
    and whoever asked is told why. An app that raises, or answers anything else, refuses it too.
    """,
    collect: """
    is asked, as every other app that collects it is, and answers its own part of what's collected, which is
    kept apart from every other app's, by its name: `nil` is no part. One that can't answer is left out, and
    whoever ran the hook is told which, and why: an app that breaks takes away its own part and nothing else.
    """
  ]

  @hooks [
    {"inputs", :mutate,
     """
     A tool is about to be called. It's about the tool's `app`, which is `nil` for one of One Core's own tools, its
     `tool` name and the `inputs` it was asked with. The answer is the inputs to set, by name. The tool is called only
     if they are still what it takes.
     """},
    {"call", :validate,
     """
     A tool is about to be called. It's about the tool's `app`, which is `nil` for one of One Core's own tools, its
     `tool` name, the `inputs` it will be called with, and `changed`: the app that set each input that was changed,
     by the input's name.
     """},
    {"reach", :validate,
     """
     An app is about to reach an artifact that isn't its own. It's about `who` would reach it, the `relation`
     they need to it (`viewer`, `editor` or `owner`) and the `artifact`'s name. It's asked about whoever the
     app runs for, and about the app, by its artifact's name, where the app didn't make the artifact.
     An `:ok` is kept for a few seconds, and until a tool of the app is called that doesn't only read: so
     what it validates from is what its own tools change.
     It's also asked whether whoever asks is a `member` of the group or the network they ask for: then `who` is
     them, the `relation` is `member` and the `artifact` is the group, or `network`.
     """},
    {"device", :mutate,
     """
     A device asks, and One Core is about to say whose it is. It's about the `device`, by its key, and `who`, which
     is nobody: `nil`. The answer is `who`: the user whose device it is, as `users/<name>`. With no app
     answering, the device is nobody's, and nobody is refused everything. It's asked of the apps installed for the
     network, since nobody is asking yet, and what's answered is kept until a tool of the app is called that
     doesn't only read.
     """},
    {"owner", :validate,
     """
     The network is about to have its first owner. It's about `who` they are. An app that keeps who may do what
     writes that down before it answers, and refuses if it can't: nothing else tells it.
     """},
    {"changed", :observe,
     """
     A blob of an artifact that an app made or reached was written or removed. It's about the `artifact`'s name, the
     `blob`'s name in it, and `by`, the app that did it, as a who. An app is told only if it could already reach
     the artifact for whoever changed it, and isn't told what it changed itself.
     """},
    {"measured", :observe,
     """
     A client said what it measured of itself. It's observed by the apps installed for the network, and it's about the
     `device`'s key and `who` it's of, as its request proved, and what its OpenTelemetry SDK exported, `otlp`: each a
     `signal` (`traces`, `metrics` or `logs`) and its export request in OTLP's protobuf, in base64. What went wrong
     or was slow there, kept on the device until a server could be told. A client from before 0.23.48 says
     `records` instead, each as Telemetry's tool `record` takes one, with the `time` it was made, in milliseconds
     since 1970.
     """},
    {"exported", :observe,
     """
     This server's OpenTelemetry SDK exported what it gathered (`OneCore.Otlp`), as it does once a minute: its
     `signal`, `traces`, `metrics` or `logs`, and `otlp`, the export request in OTLP's protobuf, in base64. It's
     observed by the apps installed for the network, on a server that runs them.
     """},
    {"uninstalled", :observe,
     """
     An app was uninstalled for someone. It's about who it was installed `for`, the `name` it was installed by
     there and the `app` it is, by its artifact's name, `with`, each app installed for them that required it
     and went with it, by its `name` and `app`, and `artifacts`, the names of what they kept and made that goes with
     them: what the same app installed for everyone reads for them stays. An app that keeps what's deleted takes
     those along, as one.
     """},
    {"superseded", :observe,
     """
     An app has one copy, and this one was superseded by the broader copy of the same app, installed or deployed for
     a group or the network that covers everyone who had it: each of its installs moved to that one. It's about the
     `app` superseded, by its artifact's name, installed for nobody now, and `by`, the copy that stays. An app that
     keeps what's deleted takes it along, so it comes back as it was if it's restored.
     """},
    {"joined", :observe,
     """
     A server was let act for someone. It's about the `server`'s key, who it's `for`, and `by`, the key of the
     server that installed it.
     """},
    {"wake", :collect,
     """
     A server is about to ask a device's app to do something on the device (`OneCore.Server.ask_device/3`), and its
     app isn't running. It's asked of the apps installed for the network, and it's about the `device`, by its key.
     An app that can wake it, as one that holds its push token, does, and answers `%{"woken" => true}`: One Core
     then waits for its app to connect. `nil` is no part.
     """},
    {"edge", :observe,
     """
     The network's edge is on an older build than its servers, and this server, on theirs, is to deploy its build's
     edge (`core/edge/updater/updater.ts`). It's observed by the apps installed for the network, and it's about the
     `build`. An app that deploys the edge deploys `OneCore.Release.edge/0`, this server's build of it, with
     `deploy.js` beside it, and says how that went with `OneCore.Release.edge_deployed/2`. A server says it deploys the
     edge (`deploys_edge`) while an app hooks into this.
     """},
    {"started", :observe,
     """
     This server started: its apps are running and it answers. It's observed by the apps installed for the network,
     and it's about the `server`'s key. An app that watches every server from its start, as Telemetry hears its
     warnings, begins here.
     """}
  ]

  @doc "Every one of One Core's hooks' names, with its kind: `:observe`, `:mutate`, `:validate` or `:collect`."
  def kinds, do: Map.new(@hooks, fn {hook, kind, _about} -> {hook, kind} end)

  @doc """
  Whether `hook` is a name an app can hook into: one of One Core's hooks, or an app's, `<app>.<hook>`, which
  may not be installed yet: then it's never run.
  """
  def hook?(hook),
    do: is_map_key(kinds(), hook) or (is_binary(hook) and hook =~ ~r/^[a-z][a-z0-9-]*\.[a-z][a-z0-9_]*$/)

  @doc "What an app's author reads about hooks: each kind, and each of One Core's hooks with its kind and what it's about."
  def listed do
    kinds = for {kind, how} <- @kinds, do: "- An app that hooks into a hook of kind `#{inspect(kind)}` #{flat(how)}"
    Enum.join(kinds ++ ["", "The hooks:", "", listed(@hooks)], "\n")
  end

  @doc "Hooks, each `{name, kind, about}`, as an app's author reads them."
  def listed(hooks), do: Enum.map_join(hooks, "\n", fn {hook, kind, about} -> "- `#{hook}` #{kind}. #{flat(about)}" end)

  defp flat(text), do: text |> String.split() |> Enum.join(" ")

  @doc """
  What an untrusted app is offered of the app interface, in its sandbox, by its module's name: each function's name.
  Each is a message to the server there (`OneCore.Sandbox`).
  """
  def offered do
    %{
      "OneCore.Call" =>
        ~w(app tool make reach reaches? member? owns? apps deployed process forward_to changed stage ask __struct__)a,
      "OneCore.Artifact" => ~w(name read read_all write replace remove size list __struct__)a,
      "OneCore.App" => ~w(file behaviour_info)a
    }
  end
end
