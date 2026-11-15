defmodule OneCore.Plugin do
  @text """
  A plugin is a folder of Elixir files, and nothing is needed to write one but a text editor. Deploying it (the
  `deploy` tool) hands the server those files: it compiles them, says what's wrong if they don't compile, and
  otherwise runs them at once, with no restart. The plugin is whoever's deployed it, and is added for them: a user, or
  the group or the network they asked for. Someone else has it too once it's added for them (the `add` tool), and
  what's added for the network is everyone's. A plugin is updated by deploying it again. Calls already running finish
  on the version they started on, and the next call gets the new one.

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
  - A file that's large, such as a photo from a phone, doesn't go inside a call or its answer. A tool takes one in
    an input that says it's an upload, `%{type: "string", format: "upload"}`: the caller sent the file straight to
    the bucket (an app with the kit's `upload`, an agent with the `upload` tool, the command with
    `--photo-from photo.jpg`), and the input is its name among the plugin's own blobs, `uploads/...`.
    `OneCore.Artifact.size(call.own, name)` says whether it arrived and how big it is. A tool answers with one of
    its blobs as `{:file, "image/jpeg", call.own, name}`: the client is handed a short-lived link, fetches the
    file from the bucket and keeps it, so a blob answered this way is one that's never replaced.
  - A `SKILL.md` at the root of the folder, if the plugin has one, is its skill: text for agents on when and how
    to use its tools, which an agent reads when it needs it, at `skill://hello/SKILL.md`, and `one hello` shows
    with the plugin's tools. It's an Agent Skill (agentskills.io): it starts with the skill's `name`, which is the
    plugin's, and its `description`, one line saying when an agent should read it, as YAML between two lines of
    `---`:

        ---
        name: hello
        description: Read when someone arrives, to greet them.
        ---

        Use `greet` when someone arrives, once for each of them.
  - The third argument to `call/3` is an `OneCore.Call`. Its `own` is the plugin's own artifact for whoever it was
    added for, which only this plugin reaches: one added for a user keeps that user's, and one added for the network
    keeps everyone's together. It keeps what it needs there as blobs through `OneCore.Artifact`: `read/2` gives `{:ok,
    data, tag}` or `:absent`, `write/3` writes only if absent and gives `:ok` or `:exists`, `replace/4` takes that
    tag, replaces only if unchanged and gives `:ok` or `:changed`, `list/2` gives the names that start with what it's
    given, and `remove/2` removes one for good. Its `folder` is a folder of the plugin's own on the machine running
    the call, for files that needn't outlive that machine.
  - `OneCore.Call.make(call, id)` makes an artifact for whoever the call runs for, `call.for`, under an id the
    plugin chooses, and the plugin keeps blobs in it as it does in its own. It's theirs, not the plugin's, and its
    name (`OneCore.Artifact.name/1`, such as `users/alice/made/hello/7`) is what they share it by.
  - `OneCore.Call.reach(call, name)` gives an artifact by its name, to a call that may reach it: as a `viewer`,
    to read it, or with `"editor"` or `"owner"` as a third argument, to change it or to say who else may. It
    raises with the reason if it may not, which is then what the caller is told, and `OneCore.Call.reaches?/3`
    asks without raising. What the plugin made for whoever it runs for is reached without asking. Anything else
    is reached only if both the plugin and whoever it runs for have that relation to it, which the plugins that
    decide `reach` say. `relations` is the one The One App ships with, and its skill says how something is
    shared. With none, nothing is reached but by the network's first owner.
  - A tool runs for whoever asked, `call.for`. Whoever asks
    can ask for a group they're a member of, or for the network: `call.for` is then the group, what the tool
    makes is the group's, it reaches what the group may, and `call.by` is who asked. A tool that the holder of a
    sharing link may call too says `by_link: true`: it then runs for someone who is no user, so it keeps to what
    `reach` gives it.
  - A tool that only reads says `read_only: true` beside its name. A client keeps its answers, shows them at
    once, and asks again behind them. After any other tool is called, every client is told that something changed.
  - `uses/0`, if the plugin has one, names the other plugins it uses, as they're added for whoever it's added for, or
    for the network. Deploying it brings along the ones the server ships with, and `OneCore.Call.plugin(call,
    "database")` gives one of them: its module, whose functions take the call and run as this plugin, for whoever it
    runs for. A plugin that keeps more than a few blobs uses `database`, whose skill says how.
  - A plugin's module is what other plugins call, so its public functions besides these are what it provides
    them. One that only provides functions leaves out `tools/0` and `call/3`.
  - `OneCore.Call.process(call, Hello.Counter, key, argument)` gives a process that keeps running between calls on
    the server running this one: a GenServer of the plugin's, one for each `key`, started with `argument` when
    first asked for. It's for what is slow to open and belongs to no machine in particular, as a database is. What
    must run on particular machines is a service.
  - Every module is a `defmodule` at the top of a file, and a plugin can have as many files and modules as it
    likes. The code is free to use the machine it runs on.

  A scheduled job is a tool call made by the clock. `jobs/0`, if the plugin has one, lists them:

      @impl true
      def uses, do: ["database"]

      @impl true
      def jobs, do: [%{tool: "sweep", every: 3600}]

  - A job names one of the plugin's `tool`s, the `inputs` to call it with if it takes any, and `every`: how many
    seconds apart. It's due at each whole multiple of that since 1970 in UTC, so `every: 3600` is on the hour and
    `every: 86_400` is at midnight UTC, and first at the next one after it's deployed.
  - The call is made on one server, for whoever the plugin was added for, when a server is up: once for each of
    them, with `call.for` the user, the group or the network, and `call.by` nobody. Times that
    passed with none up are made up for with one call when one is back.
  - A plugin with jobs uses `database`, which keeps when each last ran in the plugin's database, and has the
    server writing that database make the call. Its skill says what that promises a job's tool.
  - `schedule/3` is only for a plugin that keeps other plugins' schedules, as `database` does. It's called as
    the plugin whose jobs they are, on every server, each time one of them is due, with the jobs and a function
    that makes a job's call on the server it's called on: `fire.(job)`. It's what has one server call it once.

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
  - A plugin hooks into the moments of whoever it was added for: one added for a user decides only what runs for
    that user, and one added for the network decides for everyone.
  #{OneCore.Hooks.listed()}

  What of a plugin runs in a client is its folder `app/`, as the server built it, which reaches each client from the
  server. Its other files that aren't Elixir or skills stay on servers: the source of a program a service brings,
  and its part at the network's address. `apps/0`, if the plugin has one, names its apps:

      @impl true
      def apps, do: [%{name: "Hello", page: "app/index.html", icon: "icon.svg"}]

  - An app is drawn in the client's web view on a home screen under its `name` and `icon`. It's written in Solid
    2.0 with the kit, and its source is the folder `app/`: each `.html` there is a page, whose script is the
    app's, and the rest is whatever the app's made of. The kit gives every page its look, so a page is no more
    than this:

        app/index.html    <!doctype html><title>Hello</title><script type="module" src="./app.jsx"></script>
        app/app.jsx       import { render } from "@solidjs/web"; import { Screen } from "ui"; ...
        app/app.css       the app's own look, if it has any, imported by app.jsx
        app/package.json  packages of its own, if it needs any, under "dependencies"

  - The source is what's deployed, and the server builds it (`OneCore.App`): what a client is served at `app/` is
    what it built. A client keeps what a page loads, and reads a sourcemap only when something asks for it. The
    `code` tool gives the source back, to change and deploy again. A build that fails fails the deploy, which
    answers with the compiler's words, a problem to a line: `app.jsx:3:30: Unexpected token`, then the lines
    around it with a `^` under the place.
  - Solid and the kit are on the server and every client already, so an app leaves them out and is small; an app
    that needs nothing more installs nothing. Packages of its own are installed as it's built.
  - Read Solid's skills before writing one: `solid-2.0`, then `solid-2.0-cheatsheet`
    (skill://solid-2.0-cheatsheet/SKILL.md), and `reactivity-diagnostics` for a problem Solid names by its code.
    They're Solid's own, from the version apps are built with. Solid 2.0 isn't 1.x, and isn't React. What it renders
    with is `@solidjs/web`, not `solid-js/web`, and `Dynamic` is `dynamic` there.
  - The kit, the plugin `ui`, is what an app's UI is written with, so every app looks and moves the same. Its skill,
    `ui`, has every component with an example. It speaks to the client for the app: its `call` and `changed` are
    how an app asks and hears, done.
  - The kit's menu, select, date picker, number input and tabs are Ark UI's, with Ark's parts and props, over Zag.
    For something the kit lacks, Zag's docs for Solid are https://v2.zagjs.com/llms-solid.txt and Ark's are
    https://ark-ui.com/llms-solid.txt: an app adds `@zag-js/<component>` and `@zag-js/solid` to its package.json
    and draws the component as Zag says, styling it with the kit's custom properties. Zag's docs import from
    `solid-js/web`, which is `@solidjs/web`; and Zag's props set `id`, so one written before them is replaced.
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
    `Hello.rang(call, 4)` for `who`: one of `OneCore.Service.acts_for()`, which are who the plugin was added for that
    this server acts for, or one the plugin passed it, as `call.added` or `call.for`. That function has the call, so
    it's where a service keeps anything in the plugin's artifact or its database. In a function of the service,
    `service` goes where `self()` does.
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

  A plugin's part at the network's address is what of it has to be reached over the web: a page that opens in a
  browser, or a call from a service outside. It's an ordinary Cloudflare Worker in the plugin's folder `address`,
  with a `wrangler.jsonc`, and a `package.json` that names the first part of each path it answers:

      "paths": ["hello"]

  - It answers every request at the address whose path starts with one of those (`/hello/...`), for the whole
    network. It isn't deployed with the plugin: whoever deploys the network's address deploys it with that
    (`address/address.js`), so it's for the plugins a network is set up with.
  - It keeps what it needs in storage of its own, which its `wrangler.jsonc` binds, and is given three bindings
    besides. `env.OWN.write(name, data)` writes a blob of the plugin's own artifact for the network, only if
    there's none, and `env.OWN.remove(name)` removes one: that's where the plugin's server part reads what this
    part has to tell it, as `call.own` of the plugin added for the network. `env.NETWORK.servers(of)` gives the
    keys of the servers acting for a user, a group or the network, by their prefix (`users/alice/`).
    `env.RELAY.pass(server, message)` passes one JSON-RPC message to a server and gives its answer, or `null` if
    that server isn't up, and `env.RELAY.tell(message)` tells every server a notification.
  - The plugin's server part asks it over the web, signing what it asks as its server does at the address
    (`OneCore.Address.signed/2`). `env.NETWORK.asker(url, headers, body)` gives the key that signed it, or `null`.
  - Its `package.json`'s packages are installed and its `build` script run where the address is deployed from. A
    page it serves is built there as an app's is, against the kit (`bun ../../../ui/app.js <source> <out>`), which
    the address serves at `/ui/` as every client has it. `sign-in` does all of this.
  """
  @moduledoc "The interface a plugin's server part is written against.\n\n" <> @text

  @doc "One Core's own skill, `building-plugins`, as its `SKILL.md`: how to write a plugin, which is this module's."
  def skill do
    """
    ---
    name: building-plugins
    description: Read before making or changing a plugin. It says how one is written, deployed and updated.
    ---

    """ <> @text
  end

  @callback tools() :: [%{name: String.t(), description: String.t(), inputs: map}]
  @callback call(tool :: String.t(), inputs :: map, OneCore.Call.t()) :: term
  @callback apps() :: [%{name: String.t(), page: String.t(), icon: String.t()}]
  @callback uses() :: [String.t()]
  @callback hooks() :: [String.t()]
  @callback hook(hook :: String.t(), about :: map, OneCore.Call.t()) :: term
  @callback jobs() :: [%{tool: String.t(), every: pos_integer}]
  @callback schedule(OneCore.Call.t(), jobs :: [map], fire :: (map -> term)) :: term
  @optional_callbacks tools: 0, call: 3, apps: 0, uses: 0, hooks: 0, hook: 3, jobs: 0, schedule: 3
end
