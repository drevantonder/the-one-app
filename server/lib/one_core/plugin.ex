defmodule OneCore.Plugin do
  @guide """
  A plugin is a folder of Elixir files, and nothing is needed to write one but a text editor. Deploying it (the
  `deploy` tool) hands the server those files: it compiles them, says what's wrong if they don't compile, and
  otherwise runs them at once, with no restart. A plugin is updated by deploying it again. Calls already running
  finish on the version they started on, and the next call gets the new one.

  One module in the plugin says `@behaviour OneCore.Plugin`, and gives the plugin's tools:

      defmodule Hello do
        @behaviour OneCore.Plugin

        @impl true
        def tools do
          [
            %{
              name: "greet",
              description: "Greets someone, and says how many it has greeted",
              inputs: %{
                type: "object",
                properties: %{name: %{type: "string", description: "Who to greet"}},
                required: ["name"]
              }
            }
          ]
        end

        @impl true
        def guide, do: "Use greet when someone arrives."

        @impl true
        def call("greet", %{"name" => name}, %OneCore.Call{own: own}) do
          OneCore.Artifact.write(own, "greeted/" <> name, "")
          "Hello, \#{name}! That's \#{length(OneCore.Artifact.list(own, "greeted/"))} greeted."
        end
      end

  - `tools/0` lists each tool's `name` (lowercase letters, digits and `_`), its `description`, and its `inputs` as a
    JSON Schema object. A tool of the plugin `hello` named `greet` is the MCP tool `hello_greet` and the command
    `one hello greet`.
  - `call/3` runs a tool with its inputs, a map with string keys. They're checked against the tool's `inputs`
    first, and a caller that left out what's required, or gave the wrong kind of thing, is told so. It returns the
    answer: text, or anything JSON can hold. To refuse, raise, and the message is what the caller is told.
  - A tool answers with a file, such as a photo, as `{:blob, "image/jpeg", data}`. An agent sees the picture, an
    app is given its `data:` address, and the command writes the file out (`one hello photo > photo.jpg`).
  - A tool takes a file as base64 text, in an input that says so: `%{type: "string", contentEncoding: "base64"}`.
    The command reads one into it (`one hello keep --photo-from photo.jpg`).
  - `guide/0`, if the plugin has one, is text for agents on when and how to use its tools.
  - The third argument to `call/3` is an `OneCore.Call`. Its `own` is the plugin's own artifact, where it keeps
    what it needs as blobs through `OneCore.Artifact`: `read/2` gives `{:ok, data, tag}` or `:absent`, `write/3`
    writes only if absent and gives `:ok` or `:exists`, `replace/4` takes that tag, replaces only if unchanged
    and gives `:ok` or `:changed`, `list/2` gives the names that start with what it's given, and `remove/2`
    removes one for good. Its `folder` is a folder of the plugin's own on the machine running the call, for files
    that needn't outlive that machine.
  - `OneCore.Call.make(call, id)` makes an artifact for whoever the call runs for, `call.for`, under an id the
    plugin chooses, and the plugin keeps blobs in it as it does in its own. It's theirs, not the plugin's, and its
    name (`OneCore.Artifact.name/1`, such as `users/alice/made/hello/7`) is what they share it by.
  - `OneCore.Call.reach(call, name)` gives an artifact by its name, to a call that may reach it: as a `viewer`,
    to read it, or with `"editor"` or `"owner"` as a third argument, to change it or to say who else may. It
    raises with the reason if it may not, which is then what the caller is told, and `OneCore.Call.reaches?/3`
    asks without raising. What the plugin made for whoever it runs for is reached without asking. Anything else
    is reached only if both the plugin and whoever it runs for have that relation to it, which the plugins that
    decide `reach` say. `relations` is the one The One App ships with, and its guide says how something is
    shared. With none, everything in a network is shared.
  - A tool runs for whoever asked, and every plugin is added for the network's users. A tool that the holder of
    a sharing link may call too says `by_link: true`: it then runs for someone who is no user, so it keeps to
    what `reach` gives it.
  - A tool that only reads says `read_only: true` beside its name. A client keeps its answers, shows them at
    once, and asks again behind them. After any other tool is called, every client is told that something changed.
  - A tool whose answer to the same inputs is never different, such as a photo by its id, says `fixed: true` too.
    A client that has kept its answer never asks again.
  - `uses/0`, if the plugin has one, names the other plugins it uses. Deploying it brings along the ones the
    server ships with, and `OneCore.Call.plugin(call, "database")` gives one of them: its module, whose functions
    take the call and run as this plugin, for whoever it runs for. A plugin that keeps more than a few blobs uses
    `database`, whose guide says how.
  - A plugin's module is what other plugins call, so its public functions besides these are what it provides
    them. One that only provides functions leaves out `tools/0` and `call/3`.
  - `OneCore.Call.process(call, Hello.Counter, key, argument)` gives a process that keeps running between calls on
    the server running this one: a GenServer of the plugin's, one for each `key`, started with `argument` when
    first asked for. It's for what is slow to open and belongs to no machine in particular, as a database is. What
    must run on particular machines is a service.
  - Every module is a `defmodule` at the top of a file, and a plugin can have as many files and modules as it
    likes. The code is free to use the machine it runs on.

  A plugin hooks into a moment that isn't one of its own tools being called, to hear of it, to change it or to
  decide it. `hooks/0` names the hooks it hooks into, and `hook/3` is called at each:

      @impl true
      def hooks, do: ["call"]

      @impl true
      def hook("call", %{"plugin" => "hello", "inputs" => %{"name" => "nobody"}}, _call),
        do: {:refuse, "Nobody is there to greet."}

      def hook("call", _about, _call), do: :ok

  - `hook/3` is called with the hook's name, what the moment is about, a map with string keys, and an
    `OneCore.Call`: it runs as this plugin, for whoever the moment is about, with the plugin's own artifact. A
    hook gives a plugin nothing it couldn't already reach.
  #{OneCore.Hooks.guide()}

  A plugin's files that aren't Elixir reach each client from the server: they are its parts that run in a client,
  and the source of any program a service of its brings. `apps/0`, if the plugin has one, names its apps:

      @impl true
      def apps, do: [%{name: "Hello", page: "app.html", icon: "icon.svg"}]

  - An app is a page of HTML, drawn in the client's web view on a home screen under its `name` and `icon`. It
    loads the plugin's other files by their paths beside it (`app.js`), and another plugin's by that plugin's
    name (`../ui/ui.js`). It needs no build step.
  - The kit, the plugin `ui`, is what an app's UI is written with, so every app looks and moves the same. Its
    guide lists its elements, and it speaks to the client for the app: its `call` and `changed` are the next two
    lines, done.
  - An app reaches nothing but its client, and speaks to it as an MCP App does: JSON-RPC messages posted to
    `window.parent`, which answers with a message to the page. It sends `ui/initialize` first and then
    `ui/notifications/initialized`, calls a tool with `tools/call` (`{name: "hello_greet", arguments: {...}}`),
    and sends `ui/notifications/request-teardown` when its user goes back out of it.
  - The client sends an app `notifications/one/changed` when what it was answered has changed. The app asks
    again and draws what it's answered.

  A service is a part of the plugin that runs only on the machines that have what it needs, and keeps running
  between calls: to watch for something, hold a connection open or keep a program going. It's a module of the
  plugin that is a GenServer, and says what it needs of a machine:

      defmodule Hello.Door do
        use GenServer
        @behaviour OneCore.Service

        @impl OneCore.Service
        def needs, do: "A doorbell plugged into the machine, which writes a line to /dev/ttyUSB0 when it's pressed"

        def rings(service), do: GenServer.call(service, :rings)

        @impl GenServer
        def init(_), do: {:ok, %{bell: Port.open({:spawn, "cat /dev/ttyUSB0"}, [:binary, line: 80]), rings: 0}}

        @impl GenServer
        def handle_call(:rings, _from, state), do: {:reply, state.rings, state}

        @impl GenServer
        def handle_info({bell, {:data, _line}}, %{bell: bell} = state) do
          for who <- OneCore.Service.acts_for(), do: OneCore.Service.plugin(self(), who, :rang, [state.rings + 1])
          {:noreply, %{state | rings: state.rings + 1}}
        end
      end

  - A service runs on each server whose owner installed it there: the `services` tool lists every service with
    what it needs, and `install_service` runs one on a server. The server starts it, starts it again whenever it
    stops, and stops it when the plugin is removed. `init/1` is given nothing, and raises if the machine lacks
    what the service needs.
  - Its public functions are what the rest of the plugin calls, and nothing else can. In a tool,
    `OneCore.Service.call(call, Hello.Door, :rings)` runs `Hello.Door.rings(service)` on a machine that runs the
    service and gives its answer, where `service` is the service's process there. More arguments follow it:
    `OneCore.Service.call(call, Hello.Door, :mute, [30])` runs `mute(service, 30)`. Arguments and answers are what
    JSON can hold. The function runs on that machine for each call, apart from the others, so only what needs
    the service's state has to go through its process.
  - The plugin decides which machine it calls: `OneCore.Service.servers(call, Hello.Door)` gives the keys of the
    servers that run the service, and `OneCore.Service.call/5` takes one of them last. Left out, it's the server
    running the call if the service runs there, and otherwise the first that's up. `OneCore.Service.server()`
    gives a service the key of its own.
  - A service calls the plugin's functions with `OneCore.Service.plugin(self(), who, :rang, [4])`, which runs
    `Hello.rang(call, 4)` for the user or group `who`: one of `OneCore.Service.acts_for()`, or one the plugin
    passed it as `call.for`. That function has the call, so it's where a service keeps anything in the plugin's
    artifact or its database. In a function of the service, `service` goes where `self()` does.
  - When the plugin is updated, the service's process carries on in the new code with what it holds, and
    whatever it has open stays open. `code_change/3` is given the state, if the new code keeps it in another
    shape. A service that can't carry on starts afresh in the new code: one whose `code_change/3` refuses, one
    that takes over 5 seconds to get to it, and one whose state holds a function, so keep only data there.
  - A service can bring a program in another language, as source in a folder of the plugin's files.
    `OneCore.Service.folder(self(), "meter")` gives a folder on the machine holding the files under `meter/`,
    which is the service's to build the program in, with what the machine has, and to start it from as a
    `Port`. The folder is a new one only when those files change, so a service that kept it can tell in
    `code_change/3` whether to build and start the program again. `needs/0` says what building and running it
    takes. The program stops when its input closes, which is when the service's process ends, and a program
    that stops by itself should stop the service, so that both are started again. The plugin `load` does all of
    this.
  """
  @moduledoc "The interface a plugin's server part is written against.\n\n" <> @guide

  @doc "How to write a plugin, for whoever is about to."
  def guide, do: @guide

  @callback tools() :: [%{name: String.t(), description: String.t(), inputs: map}]
  @callback call(tool :: String.t(), inputs :: map, OneCore.Call.t()) :: term
  @callback guide() :: String.t()
  @callback apps() :: [%{name: String.t(), page: String.t(), icon: String.t()}]
  @callback uses() :: [String.t()]
  @callback hooks() :: [String.t()]
  @callback hook(hook :: String.t(), about :: map, OneCore.Call.t()) :: term
  @optional_callbacks tools: 0, call: 3, guide: 0, apps: 0, uses: 0, hooks: 0, hook: 3
end
