defmodule OneCore.App do
  @text """
  An app is a folder of files, and nothing is needed to write one but a text editor. It has the same three parts
  as One Core: its Elixir in `server/`, what runs in a client in `client/`, and its edge function, if it has one, in
  `edge/`. Its skill is at its top. Deploying it (the `deploy` tool) hands the server those files: it compiles its
  Elixir, says what's wrong if it doesn't compile, and otherwise runs it at once, with no restart. The app is
  whoever's deployed it, and is installed for them: a user, or the group or the network they asked for. Someone else
  has it too once it's installed for them (the `install` tool), and what's installed for the network is everyone's. An app is
  updated by deploying it again, which everyone who has it runs at once: an app has one version. A trusted app
  installed from a git repo also takes the repo's newer commits by itself, and an untrusted one offers them as its
  update (`apps`).

  Its server code runs with all of its machine's access, so it runs only on servers of whoever it's installed for:
  the network's servers for the network, and for a user or a group a server of their own, set up in Devices to act
  for them alone (or the network's, for the network's owners). So deploying or installing one for yourself takes a
  server of your own, and for the network its owners or an `installer` of it (`permissions`); anyone else asks them
  by the network's launcher, whose skill says how. While none of the servers it runs on is on, a call to it says so. Calls already running finish on the version they started on, and the next call
  gets the new one. An app a program ships with can have native code that the clients are built with, in
  `client/native/`, which is never deployed.

  An app can be untrusted instead (`deploy` with `trusted: false`), and an untrusted app runs in a sandbox: its
  server code runs in an Erlang VM of its own, for each user, group or network it's installed for, behind its
  machine's sandbox, never in the server's.
  Any of the network's servers runs it for anyone, so it needs no server of yours, and whatever its code tries, it
  reaches only what's offered it, as whoever it runs for. What's offered is this interface, each function a message
  to the server: `tools/0`, `call/3`, `entrypoints/0`, `icon/0`, `widgets/0`, `jobs/0` and
  `requires/0`; `OneCore.Artifact`'s functions but `link`;
  `OneCore.Call`'s `tool` (with `on_behalf_of`), `make`, `reach`, `reaches?`, `member?`, `owns?`, `apps`, `deployed`, `process`,
  `forward_to`, `changed`, `stage` and `ask`; `OneCore.App.file/2`; `OneCore.Nif`; `Logger`; and hooks it observes, and collects that say it may (`untrusted: true`). The apps
  it requires whose code runs inside it run in its VM too, as it, `database` among them, where their code can run
  there. It has no services, no hooks it mutates or validates, no other apps' functions but those (it calls the
  tools of the apps it requires), no programs, no network and no website, and no files but its `folder`, which goes
  when the VM stops, a minute after its last call. Deploying an untrusted one that uses what isn't offered says each,
  with its file and line, and what to do instead.

  One module in the app, in a file of `server/` such as `server/hello.ex`, says `@behaviour OneCore.App`, and
  gives the app's tools:

      defmodule Hello do
        @behaviour OneCore.App

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
              },
              outputs: %{
                type: "object",
                properties: %{
                  greeting: %{type: "string", description: "What to say to them"},
                  greeted: %{type: "integer", description: "How many it has greeted, them among them"}
                },
                required: ["greeting", "greeted"]
              }
            }
          ]
        end

        @impl true
        def call("greet", %{"name" => name}, %OneCore.Call{own: own}) do
          OneCore.Artifact.write(own, "greeted/" <> name, "")
          %{greeting: "Hello, \#{name}!", greeted: length(OneCore.Artifact.list(own, "greeted/"))}
        end
      end

  - `tools/0` lists each tool's `name` (lowercase letters, digits and `_`), its `description`, its `inputs` as a
    JSON Schema object, and its `outputs`: a JSON Schema of the object it answers, which is MCP's `outputSchema`. A tool of the app `hello` named `greet` is the MCP tool `hello_greet` and the command
    `one hello greet`, whose inputs are flags as its `type` says: `--name Sam`, `--count 3`, `--loud` for true, a
    list's items one after another (`--tags a b`), and `--no-name` for `null` where the type takes it
    (`["string", "null"]`). `one hello greet --help` says what each takes.
  - `call/3` runs a tool with its inputs, a map with string keys. They're checked against the tool's `inputs`
    first, and a caller that left out what's required, or gave the wrong kind of thing, is told so. It returns the
    answer, a map JSON can hold, which is checked against its `outputs`: one that isn't what they say fails the
    call, saying where. To refuse, for what it was given or a device it needs being off, raise `OneCore.Call.Refused`,
    and the message is what the caller is told: its first paragraph in plain words, for the person whose screen shows
    it, then after a blank line what an agent needs, such as which tool to call next
    (`raise OneCore.Call.Refused, "You have no note called \#{name}.\\n\\nThe tool list lists them."`). A refusal isn't
    an error in Telemetry. To fail saying why, as a service it asked failing, `raise "..."` in the same words.
    Anything else raised is a fault, which a person is told only went wrong.
  - Every tool says its `outputs`, and deploying one that doesn't is refused: an agent writing a script reads them as
    the type its call returns, so it uses the answer without calling first to see it. Say what's there as it is: each
    field's `type`, with `"null"` among them where it can be nil (`["string", "null"]`), a list's `items`, and a
    `description` wherever the name alone doesn't say what it is, its unit or its format (`"When it was sent, in
    milliseconds since 1970"`). Put in `required` what's always there. An answer that's a list is a field of the
    object (`%{threads: [...]}`), and one that's a sentence is the facts it says (`%{sent: true, id: "..."}`), not
    the sentence. Where an answer is another service's, give the fields the tool passes on, not every one it might.
  - A tool answers with a file, such as a photo, as `{:blob, "image/jpeg", data}`. An agent sees the picture, an
    app is given its `data:` address, and the command writes the file out (`one hello photo > photo.jpg`). Its
    `outputs` are what's said of the file, `%{mimeType: ..., size: ...}`, its kind and its bytes:
    `%{type: "object", properties: %{mimeType: %{type: "string"}, size: %{type: "integer"}}, required: ["mimeType"]}`.
  - A tool takes a file as base64 text, in an input that says so: `%{type: "string", contentEncoding: "base64"}`.
    The command reads one into it (`one hello keep --photo-from photo.jpg`).
  - A file that's large, such as a photo from a phone, doesn't go inside a call or its answer. A tool takes one in
    an input that says it's an upload, `%{type: "string", format: "upload"}`: the caller sent the file straight to
    the bucket (an app with the UI library's `upload`, an agent with the `upload` tool, the command with
    `--photo-from photo.jpg`), and the input is its name among the app's own blobs, `uploads/...`.
    `OneCore.Artifact.size(call.own, name)` says whether it arrived and how big it is. A tool answers with one of
    its blobs as `{:file, "image/jpeg", call.own, name}`: the client is handed a short-lived link, fetches the
    file from the bucket and keeps it, so a blob answered this way is one that's never replaced. What's said of it
    is its `mimeType` alone.
  - A `SKILL.md` at the root of the folder, if the app has one, is its skill: text for agents on when and how
    to use its tools, which an agent reads when it needs it, at `skill://hello/hello/SKILL.md`, and `one hello`
    shows with the app's tools. Its address starts with the name the app is installed by, which is where an
    agent finds it: installed as `greeter`, it's `skill://greeter/hello/SKILL.md`, which the `skills` tool reads as
    `{ app: "greeter", name: "hello" }`, and `search` gives it with the tools of it that match. It's an Agent
    Skill (agentskills.io): it starts with the skill's `name`, which is the app's, and its `description`, one line
    saying when an agent should read it, as YAML between two lines of `---`, whose words `search` finds the app's
    tools by too. Its `metadata`, which Agent Skills leaves to whoever reads
    it, is what a person reads where the app's listed (the App Store's Marketplace, the app directory): its `title`, its
    name wherever a person reads it (the App Library, the App Store, a notification it sends), as an App Store app's
    name is, never its `name`; its `subtitle`, a few words under that; and `about`, a few plain sentences on its
    page, as Google Play's "About this app" is. Both say what it does for a person, with no word of an agent's. Its `category`
    says what the app is for, and the App Store and the App Library group apps by it; it's one of these, and a
    deploy without one is refused:

    | `category` | For |
    | --- | --- |
    | `Connectors` | Your accounts at outside services, for you and your agents: email, messages, calendars, contacts, documents and files |
    | `Privacy & Security` | Signing in, keeping secrets, encryption and who may reach what |
    | `Data` | Keeping data, backing it up and getting it back |
    | `AI` | Models, what agents remember and what they're told |
    | `Agent tools` | What an agent uses to act: browsers, screens, phones |
    | `Devices` | The machines themselves: their load, power, shells and containers |
    | `Developer tools` | Building, debugging and running code |
    | `Utilities` | Anything else |
    | `Admin` | What other apps stand on, which only the network's owners manage: databases, encryption, sandboxes |

    Each is one line, as the directory reads it:

        ---
        name: hello
        description: Read when someone arrives, to greet them.
        metadata:
          title: Hello
          subtitle: Hello to whoever arrives
          about: Says hello to each person who arrives, by name, and counts how many it has greeted.
          category: Utilities
        ---

        Use `greet` when someone arrives, once for each of them.
  - The third argument to `call/3` is an `OneCore.Call`. Its `own` is the app's own artifact for whoever the call
    runs for, which only this app reaches: installed for the network, it keeps each person's apart, as an Android app
    installed for every user of a device does, and one installed for a user keeps that user's. An app whose data is
    the network's as a whole, such as its permissions or its backups, or that keeps everyone's together and apart
    itself, says so with `def own, do: :installed`: its `own` is then whoever it was installed for's, one for everyone.
    It keeps what it needs there as blobs through `OneCore.Artifact`: `read/2` gives `{:ok,
    data, tag}` or `:absent`, `write/3` writes only if absent and gives `:ok` or `:exists`, `replace/4` takes that
    tag, replaces only if unchanged and gives `:ok` or `:changed`, `update/3` changes a record in one step (it's
    given what's there, and given it again if someone else wrote it meanwhile), `list/2` gives the names that start
    with what it's given, and `remove/2` removes one for good, so never to write it again. Its `folder` is a folder of the app's own on the machine running
    the call, for files that needn't outlive that machine.
  - `OneCore.Call.make(call, id)` makes an artifact for whoever the call runs for, `call.subject`, under an id the
    app chooses, and the app keeps blobs in it as it does in its own. It's theirs, not the app's, and its
    name (`OneCore.Artifact.name/1`, such as `users/alice/made/hello/7`) is what they share it by. When they
    delete one, it goes to Recently Deleted, which restores it for 30 days: `OneCore.Call.tool(call, "soft-delete", "delete",
    %{"artifact" => name})`, whose skill says more.
  - `OneCore.Call.reach(call, name)` gives an artifact by its name, to a call that may reach it: as a `viewer`,
    to read it, or with `"editor"` or `"owner"` as a third argument, to change it or to say who else may. It
    raises with the reason if it may not, which is then what the caller is told, and `OneCore.Call.reaches?/3`
    asks without raising. What the app made for whoever it runs for is reached without asking. Anything else
    is reached only if both the app and whoever asked have that relation to it, which the apps that
    decide `reach` say. `permissions` is one, which a network starts with, and its skill says how something is
    shared. With none, nothing is reached but by the network's first owner.
  - A tool runs for whoever asked, `call.subject`. Whoever asks
    can ask for a group they're a member of, or for the network: `call.subject` is then the group, what the tool
    makes is the group's, and `call.actor` is who asked. They call a tool that only reads (`read_only: true`) if
    they may see what the app made for the group (`groups/family/made/<app>`), and any other if they may change it,
    and it reaches only what they may themselves. A tool that the holder of a
    sharing link may call too says `by_link: true`: it then runs for someone who is no user, so it keeps to what
    `reach` gives it.
  - `OneCore.Call.owns?(call)` says whether whoever asked owns whoever the call runs for: themselves, or the
    group or the network as its `owner`. A tool that changes what's a group's as a whole asks it.
  - `OneCore.Call.apps(call)` lists the apps whoever the call runs for finds, theirs and the network's,
    each with who it was installed for, its entrypoints and its widgets. A Home Screen page is drawn from it.
  - `OneCore.Call.deployed(call)` lists every app anyone in the network has deployed, as it's listed (its icon,
    subtitle, category, apps and widgets), whether or not it's installed for anyone now: an app store is drawn from it.
  - Make a tool an MCP task ([Tasks](https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/tasks))
    with `task_support: :optional` when it can take more than a few seconds, works through stages, or waits on a
    person. Callers can then start it, carry on, check back, and cancel it. `Call.stage` sets the task's
    `statusMessage`; `Call.ask` moves it to `input_required` and asks the person through MCP elicitation. Its code
    doesn't change otherwise: a caller that doesn't ask for a task waits on it as on any other, and is told its stages
    as progress. `:required` is for one that can't run unless it's a task, as one that always asks. Cancelling a call
    raises at its next `Call.stage` or `Call.ask`, so `try … after` cleans up, and stops its process outright if it
    reaches neither within a few seconds, with what it started linked to it.

        %{name: "copy", task_support: :optional, description: "Copies every photo to the archive", ...}

        def call("copy", _inputs, call) do
          photos = OneCore.Artifact.list(call.own, "photos/")

          for {photo, done} <- Enum.with_index(photos, 1) do
            OneCore.Call.stage(call, "Copying \#{done} of \#{length(photos)}")
            archive(call, photo)
          end

          %{copied: length(photos)}
        end

    `OneCore.Call.ask(call, "Replace the archive's copies?")` gives `%{"action" => "accept"}` when they say yes, and
    takes a `requestedSchema` for more (`%{message: ..., requestedSchema: %{type: "object", properties: ...}}`).
  - A tool that only reads says `read_only: true` beside its name. A client keeps its answers, shows them at
    once, and asks again behind them. After any other tool is called, every client is told that something changed.
  - An app's module is what other apps call, so its public functions besides these are what it provides
    them, and its code runs inside every app that requires it. One whose module has none runs only as itself,
    and is called by its tools. One that only provides functions leaves out `tools/0` and `call/3`.
  - Those functions run as the app that calls them, with its own artifact and database. What has to be kept in a
    app's own, such as an inbox, is one of its tools, which any app calls as whoever its call runs for:
    `OneCore.Call.tool(call, "notifications", "send", %{"title" => "Lunch is here"})` gives the answer, or raises
    with why. An app that provides such a tool can give a function that calls it, as `Notifications.send/2` does.
  - A tool another app called is told which, as `call.caller`, the app's artifact's name, which One Core
    sets and nothing asked can say: `nil` when a person, an app or an agent called it. A tool that does something
    for whichever app called it calls others in turn on behalf of it, as OAuth's on-behalf-of flow does:
    `OneCore.Call.tool(call, "vault", "fetch", request, on_behalf_of: call.caller)`. That's how an app uses an
    account without ever being handed its secret (`vault`'s skill).
  - Who someone is, an app asks `sign-in`, the app that signs people in, by its tool:
    `OneCore.Call.tool(call, "sign-in", "people", %{"users" => ["users/alice"]})["people"]` gives each one's
    `called`, the name they chose, and their `email` where how the network signs in has one, either `nil` where
    there's none.
    Left out, `users` is whoever asked. It answers about others only to a member of the network, as the apps
    that validate `reach` say (`OneCore.Call.member?/2`), so an app learns no more than its person may.
  - `requires/0`, if the app has one, names the other apps it needs, which are its requirements, each with
    why, as a person reads it after "Gmail uses Google Account":
    `def requires, do: %{"google-account" => "to reach your mail"}`. A requirement is there for whoever it's installed
    for where it's installed for them or for the network (`OneCore.Apps.Requires`), and installing an app
    installs its requirements. `deploy`, `install` and `apps` say them, and `apps` says the other way, what
    each app is required by. An untrusted app calls the tools of only the apps it requires.
    It names the platform too, as `"core"`, where it uses what's new in it: the lowest version it runs on, as an
    Elixir requirement, `%{"core" => ">= 0.16.0", ...}`, as Android's `minSdkVersion` is. It runs on that and every
    version after: deploying it to older servers is refused, saying so, and is done again once they're updated
    (`waiting_for_server`), which `one apps` and `one builds` say.
  - A requirement whose module provides functions (below) runs inside the app that requires it:
    `OneCore.Call.app(call, "database")` gives its module, whose functions take the call and run as this app,
    for whoever it runs for. Any other is called by its tools. An app that keeps more than a few blobs requires
    `database`, whose skill says how. What whoever runs the bucket mustn't read it keeps with `encryption`, and an
    account at an outside service it uses through `vault`: their skills say how, and which of a tool's inputs a
    device encrypts before it's sent.
  - `OneCore.Call.fetch(call, %{url: "https://api.example.com/x", method: "POST", headers: %{...}, body: "..."})`
    makes a web request from the server running the call, and gives the answer's `status`, `headers` and `body`. An
    untrusted app reaches no website.
  - `OneCore.Call.process(call, Hello.Counter, key, argument)` gives a process that keeps running between calls on
    the server running this one: a GenServer or a `:gen_statem` of the app's, one for each `key`, started with
    `argument` when first asked for. It's for what is slow to open and belongs to no machine in particular, as a
    database is. What must run on particular machines is a service. Either carries on through the app's
    updates as a service does (below).
  - A process that goes through stages, such as starting, connecting, waiting for a person or holding something
    for a while, is a `:gen_statem`, written as OTP says, with each stage a named state and the moves between
    them in its `handle_event/4`. What a stage can't handle yet is `:postpone`d until the next, how long a stage
    may last is its `:state_timeout`, and nothing waits with `Process.sleep`, so the process still hears what
    comes meanwhile. A tool says the state by name, and the app shows each one, never an error while it's still
    starting. A GenServer is for a process that handles each message the same way whatever came before.
  - Every module is a `defmodule` at the top of a file in `server/`, and an app can have as many files and modules
    as it likes. The code is free to use the machine it runs on.
  - An app's files can be anything: a template, a picture, a font. Its code reads one of its own by its path,
    as this version was deployed with it: `OneCore.App.file(__MODULE__, "server/form.xlsx")` gives what's
    in it. `deploy` takes a file that isn't text as `{"base64": "..."}` in place of its text, as `code` gives it
    back, and `one deploy --files-from <folder>` sends each file as it is.

  - An app brings native code as a NIF, which its Elixir calls: its source in a folder of `server/nif/`, say
    `server/nif/sqlite/`, which is never deployed, and beside it the library built from it for each kind of machine
    a server runs on, `linux-x86_64.so`, `linux-aarch64.so`, `macos-aarch64.so` and `windows-x86_64.so` (a DLL),
    which are. Each build is a module of its own, named for what it's built from, which its `build.rs` gives Rustler
    as the database app's does (`server/nif/sqlite/build.rs`), and `tooling/nifs` builds. Its Elixir is
    `server/nif/sqlite.ex`, one module named in one word, which says `use OneCore.Nif` and has each function
    `:erlang.nif_error(:nif_not_loaded)`. A changed NIF deploys as any change does, with no restart: its build loads
    beside the one before, this version's code calls its own build by that name, and another app's code the
    newest. A deploy whose NIF doesn't load on the server fails, saying why, and the version before runs on. What a
    build opens, a database or keys, works only with that build, so it's kept with the module that opened it, and a
    process that keeps one into a new version opens it again in `code_change/3`, as `Database.Lease` does.
    The app `database` brings `Sqlite`.
  - An app brings a program the same way, from its source in a folder of `server/bin/`, built for each kind of
    machine as `<kind>` beside it, which its code reads with `file/2` and writes out to run: the app `sandbox`
    brings its wall on Windows, `server/bin/wall/windows-x86_64`.
  - A git repo keeps a NIF's or program's source, not its builds: those are published as files at URLs, as Elixir's
    rustler_precompiled publishes a library's (a GitHub Release's files, say), and `server/checksums.json` names
    each by its path with its `url` and `sha256`, so the commit an app is installed or updated from pins exactly
    which builds are trusted. A deploy that lacks a build named there, as one from git does, downloads it and checks
    it, and fails saying which if it's missing or isn't that build, and the version before runs on. A build a deploy
    brings, from a checkout, is deployed as it is. This repo's apps publish theirs with `tooling/publish-nifs`.

  A scheduled job is a tool call made by the clock. `jobs/0`, if the app has one, lists them:

      @impl true
      def requires, do: %{"database" => "to keep when each job last ran"}

      @impl true
      def jobs, do: [%{tool: "sweep", every: 3600}]

  - A job names one of the app's `tool`s, the `inputs` to call it with if it takes any, and `every`: how many
    seconds apart. It's due at each whole multiple of that since 1970 in UTC, so `every: 3600` is on the hour and
    `every: 86_400` is at midnight UTC, and first at the next one after it's deployed.
  - The call is made on one server, for whoever the app was installed for, when a server is up: once for each of
    them, with `call.subject` the user, the group or the network, and `call.actor` nobody. One installed for the
    network that keeps each person's data apart is called for each person who keeps something with it too, with
    their own `own`. Times that
    passed with none up are made up for with one call when one is back.
  - Nobody is told a job ran, as they are a person's call: one that changes what a screen shows calls
    `OneCore.Call.changed/1` once it has, and one that changed nothing tells nobody.
  - An app with jobs requires `database`, which keeps when each last ran in the app's database, and has the
    server writing that database make the call. Its skill says what that promises a job's tool.
  - `schedule/3` is only for an app that keeps other apps' schedules, as `database` does. It's called as
    the app whose jobs they are, on every server, each time one of them is due, with the jobs and a function
    that makes a job's call on the server it's called on: `fire.(job)`. It's what has one server call it once.

  What an app does shows in Telemetry, where it's installed, in Settings and to agents through its tools. Every tool
  call is a span there already, with whose it was, how long it took and how it ended, and so is every warning, error
  and crash said to `Logger`: Logger's info and debug aren't kept, so they can say more. An app records more by
  calling Telemetry' tool `record`, whose skill says how. Whoever may deploy it looks into it as it runs on a server
  with `server-debug`, where it's added: its Logger output at every level, its processes and a trace of a function,
  of their own calls. `sensitive/0`, if an app gives `true`, says what it's given and keeps are secrets, as
  Vault's are, and nothing looks into it.

  An app hooks into a moment that isn't one of its own tools being called, to observe it, to mutate it or to
  validate it. `hooks/0` names the hooks it hooks into, and `hook/3` is called at each:

      @impl true
      def hooks, do: ["call"]

      @impl true
      def hook("call", %{"app" => "hello", "inputs" => %{"name" => "nobody"}}, _call),
        do: {:refuse, "Nobody is there to greet."}

      def hook("call", _about, _call), do: :ok

  - `hook/3` is called with the hook's name, what the moment is about, a map with string keys, and an
    `OneCore.Call`: it runs as this app, for whoever the moment is about, with the app's own artifact. A
    hook gives an app nothing it couldn't already reach.
  - An app hooks into the moments of whoever it was installed for: one installed for a user mutates and validates only
    what runs for that user, and one installed for the network does for everyone.
  #{OneCore.App.Manifest.listed()}

  An app that knows which app it needs calls it (`OneCore.Call.tool/4`, `OneCore.Call.app/2`). One that can't
  know who will answer offers a hook instead, and whoever wants to hooks into it. `offers/0` lists them, each with a
  `name`, a `kind` (`:observe`, `:mutate`, `:validate` or `:collect`) and a `description`, and for a collect whose
  parts it only reads as data, `untrusted: true`, which an untrusted app may answer too (no other hook asks one but
  to observe). `OneCore.Call.run_hook/3` runs one,
  as One Core runs its own of that kind, for whoever the call runs for. Another app hooks into it by its full name,
  the app's name and the hook's, in its `hooks/0`, and before the offering app is installed it's never run:

      def offers, do: [%{name: "moved", kind: :collect, description: "Something made for someone was deleted"}]
      OneCore.Call.run_hook(call, "moved", %{"artifact" => name, "to" => "deleted"})   # in soft-delete
      def hooks, do: ["soft-delete.moved"]                                               # in git-repos

  - `soft-delete.moved` says each move of something an app made through soft delete (deleted, purging, active
    again, purged), and soft delete waits for every app's answer. On `purging`, before anything of it is erased,
    it's at its name again, so the app that made it reads it once more as it always does, as one does to revoke a
    token it holds; `readable` is false only where something new was made at its name since. An app that raises
    or doesn't answer within ten seconds holds that purge back a day, once: the next day it's purged whatever it
    answers, so no app keeps something someone deleted for ever.

  What of an app runs in a client is its folder `client/`, as the server built it, which reaches each client from
  the server. Its other files that aren't Elixir or skills stay on servers: the source of a program a service brings,
  and its edge function. An app's icon is its own, as an Android package's is, and deploying one without it is
  refused: `icon/0` names its file, an entrypoint's icon or one of its own, which goes wherever the app itself is shown (the App Store, Settings). `entrypoints/0`, if the app has any, names its app entrypoints: each an icon
  it puts in the launcher, with its page, a page of `client/`, and an icon of its own or else the app's. It's called
  its app's `title`, and has a `name` of its own only where it's something else than its app, and so is a widget.
  People just see apps:

      @impl true
      def icon, do: "icon.png"

      @impl true
      def entrypoints, do: [%{page: "client/index.html", runtime_version: 5}]

  - An entrypoint may say its `category`, one of the app's categories above, where it's for something else than its
    app (`category: "Devices"`); One Core passes it to the launcher as it is, which otherwise places it by its app's. One
    Core knows one more, `launcher`: the app every client opens first, which draws the rest. A network has exactly one, installed
    for the network, so installing a second is refused, and so is uninstalling it; deploying another as its new
    version replaces it. A client knows two more: `app-store`, the App Store, which installs and removes apps, which it
    opens itself where there's no launcher yet; and `settings`, where apps' settings are, which a link to an app with
    no page of its own opens at that app.
  - An entrypoint may say its `subject`, who it's opened for, which One Core passes to the launcher as it is too:
    `subject: "user"` is an entrypoint about whoever opens it, as Settings and Notifications are, which is opened for them wherever
    it's installed. Without one, it's opened for whoever it's installed for, or whose page it's on.
  - An entrypoint may say its `badge`, one of the app's tools that only reads, whose answer's `count` is the red count
    on its icon while it's more than 0, as an iPhone's app sets its own: what's waiting for whoever's looking, as
    Settings counts what waits in its sections, such as devices waiting to sign in as them (`badge: "badge"`). A Home Screen asks it again whenever
    something changed, so it only reads what the app keeps.
  - An entrypoint says its `runtime_version`, the runtime version its page needs of a native host
    (`RUNTIME_VERSION` in `core/ui/src/drawing-primitives/renderer.ts`, 5 now): its page is written with the UI library
    alone (the UI library's skill), so a computer, an iPhone or Android that has that version draws it itself, a browser
    draws it with the same renderer on its WebGPU, and so does an assistant showing it as an MCP App, in a frame of
    the web's (`core/edge/mcp/mcp.ts`). A computer or a phone doesn't open one
    needing a newer version than it has, and says why in its window. Deploying type checks it against
    the UI library (`OneCore.Entrypoint.lint/3`) and refuses it with the checker's errors where it isn't, and gives its warnings with the deploy's: a raw
    colour or length in a style, which should be a token.
  - An app's settings are in Settings, where it adds a section, or a group in another's page, by the hook
    `settings.sections`, with the component that draws it, as Settings' skill says.
  - An app that keeps something in its people's Vaults (an account at an outside service, a Chrome profile, a key)
    registers its item type there, as the Vault's skill says: `vault.types`, with the section it's listed under, how
    one is added, and the component that draws one, if it has one. An account is such an item, and isn't an app:
    having them makes no entrypoint. An app has an entrypoint only for a screen a person opens for something else.
  - Its item is in one of three states, which the app says (`vault_state`) and the Vault shows: `ready`, usable,
    which shows nothing; `busy`, with a message ("Saving…"); or `needs_action`, with why and the one action that
    fixes it ("Signed out on your phone", Sign In). It keeps its own detailed states and says only which of these it's
    in. Deleting, restoring and purging an item are soft delete's, which the Vault doesn't add to.
  - Two things are the app's to choose, deliberately, and nothing chooses for it. Whether its person is also
    notified when its item needs them (`notifications`): WhatsApp and Messenger notify, since messages stop, and
    Chrome Profiles doesn't, since its person is in the update that failed. And what it does when soft delete's hook
    says its item was deleted, restored or purged: sign out and revoke at once, so no token waits in Recently Deleted,
    or keep the token so a restore just works and revoke it only once it's purged.
  - A tool with a page of its own, which shows what it does to a person, says it as `ui: "client/link.html"`. It's
    listed as an MCP App's tool names its UI (`_meta.ui.resourceUri`), so a host shows it, and that page needn't be
    an app. One that isn't is named for what it does (`client/sign-in.html`), not `client/index.html`, an app's. It
    says its `runtime_version` beside it, as an entrypoint does, for a native host to draw it.
  - An app is drawn on a home screen under its `name` and `icon`, natively on a computer, an iPhone and Android, and
    by the same renderer in a browser. It's written in Solid 2.0 with the UI library, and its source is the folder `client/`:
    each `.html` there is a page, whose script is the app's, and the rest is whatever the app's made of. Its look is
    the UI library's components and `styles()` beside what uses them, never a stylesheet. The UI library gives every
    page its look, so a page is no more than this:

        client/index.html     <!doctype html><title>Hello</title><script type="module" src="./app.tsx"></script>
        client/app.tsx        import { render, Screen, Text } from "ui"
                              render(() => <Screen heading="Hello"><Text>Hello</Text></Screen>)
        client/tsconfig.json  {"extends": "<ui>/tsconfig.app.json", "include": ["**/*"]}, for its editor: a deploy
                              checks it against its server's own copy of the UI library
        client/package.json   packages of its own, if it needs any, under "dependencies"

  - An app's client may export modules for another app's page to import, as a package does, in
    `client/package.json`'s `exports` (`{"exports": {"./profile": "./profile.tsx"}}`), each built as `client/<name>.js`
    with what it exports: a component another app draws, as the Vault draws an item of a type an app registers.
    Solid and `ui` are the one copy every page and every module it imports shares, whichever app
    built it, so signals and context work across apps; and every page is served with an import map (`OneCore.Mcp`)
    that names them and `<app>/`, the client of its own app and of each app it requires, for what it imports by
    name as it runs.

  - The source is what's deployed, and the server builds it (`OneCore.Entrypoint`): what a client is served at `client/` is
    what it built. A client keeps what a page loads, and reads a sourcemap only when something asks for it. The
    `code` tool gives the source back, to change and deploy again. A build that fails fails the deploy, which
    answers with the compiler's words, a problem to a line: `app.tsx:3:30: Unexpected token`, then the lines
    around it with a `^` under the place.
  - Solid and the UI library are in every client already, so an app leaves them out and is small; an app that needs
    nothing more installs nothing. Packages of its own are installed as it's built. It's built with the UI library's own
    build, as the server it's deployed on ships it, and built again once a server's build brings another.
  - Read Solid's skills before writing one: `solid-2.0`, then `solid-2.0-cheatsheet`
    (skill://ui/skills/solid-2.0-cheatsheet/SKILL.md), and `reactivity-diagnostics` for a problem Solid names
    by its code.
    They're Solid's own, from the version apps are built with. Solid 2.0 isn't 1.x, and isn't React. A page draws with
    `render` from `"ui"`, never Solid's own from `@solidjs/web`, which doesn't tell its client it drew; `@solidjs/web`
    gives a page only `dynamic` and `Dynamic`, for a component chosen as it runs.
  - The UI library, One Core's own, is what an app's UI is written with, so every app looks and moves the same. Its
    skill, `ui`, has every component with an example. It speaks to the client for the app: its `call` and `changed` are
    how an app asks and hears, done.
  - How an app looks, reads and behaves follows `design` (skill://ui/skills/design/SKILL.md), the network's own
    rules. Beside it are Emil Kowalski's: `gestures` for drags, swipes and sheets, `apple-design` for fluid motion,
    `mobile-native` for what makes it feel at home on a phone, and `review-animations` to review its motion. Where one of
    them says otherwise (a motion library's springs, say), `design` and the UI library win: they're what every app shares.
  - Its screens are TypeScript (`.tsx`): `typescript-best-practices` says how to write it. Beside it are pstack's
    principles: `principle-experience-first` for what to build, `principle-model-the-domain` for the state it keeps,
    `principle-make-operations-idempotent` for tools and jobs that may run twice, `principle-fix-root-causes` when
    something's wrong, and `principle-prove-it-works` before calling it done.
  - The UI library's components are its own and work on every host, the DOM and a native one. For something it lacks,
    compose its primitives (and, later, the element API).
  - An app reaches nothing but its client, and speaks to it as an MCP App does: JSON-RPC messages posted to
    `window.parent`, which answers with a message to the page. It sends `ui/initialize` first and then
    `ui/notifications/initialized`, calls a tool with `tools/call` (`{name: "hello_greet", arguments: {...}}`),
    and sends `ui/notifications/request-teardown` when its user goes back out of it.
  - The client sends an app `notifications/one/changed` when what it was answered has changed. The app asks
    again and draws what it's answered.

  A widget is what the app shows on a Home Screen page at a glance: a tool call, drawn. `widgets/0`, if the app
  has one, lists them. This is a whole app with one: a folder with `server/glance.ex`, its `icon.svg`, and a
  `SKILL.md` as above, named `glance`, in the category `Utilities`:

      defmodule Glance do
        @behaviour OneCore.App

        @impl true
        def tools do
          [
            %{
              name: "glance",
              description: "One figure, to draw on a Home Screen page",
              read_only: true,
              inputs: %{type: "object", properties: %{size: %{type: "string"}}},
              outputs: %{type: "object", properties: %{part: %{type: "string"}}, required: ["part"]}
            }
          ]
        end

        @impl true
        def call("glance", %{"size" => _size}, _call), do: %{part: "stat", value: 42, label: "answers"}

        @impl true
        def icon, do: "icon.svg"

        @impl true
        def widgets, do: [%{name: "Glance", tool: "glance", sizes: ["2x2"]}]
      end

  - A widget names one of the app's `tool`s that only reads, its `name` on the card, and the `sizes` it comes
    in, a phone's own: `2x2`, `4x2` and `4x4`, columns by rows of a Home Screen page's grid. The smallest is one fact,
    the wide one a few rows, and the large one those rows with a detail each, or a chart.
  - The tool is called with the `size` to draw, for whoever's Home Screen page it's on, and answers a layout: one
    part, which holds the rest. A part is a map that says which `part` it is, from this list, and nothing in a
    layout is ever markup. Its `outputs` say so: `%{type: "object", properties: %{part: %{type: "string"}},
    required: ["part"]}`. Every client draws the same layout: the UI library's `Widget` on a Home Screen page, and a phone's
    own home screen once it has widgets. A client asks again when something changed, and when the page is
    returned to. A widget that shows what only time changes, such as a load, says `refresh` in its layout's root part:
    how many seconds until it's asked again, while it's in view.
  - `column` and `row` hold `parts`, with a `gap` between them in points if not the usual. Any part can say
    `grow: true` to take the room that's left, and a `width` in points. `align: "center"` centres what a column
    or row holds, and `dim: true` fades it, for what's off. A column or row with `from: "sm"` or `"md"` is
    dropped when the widget is narrower than 15 or 18 rem, so what doesn't fit goes whole, never cut off.
  - `text` says `text`, in a `size` (`xl`, `lg`, `sm`, `xs`) and `weight` (`medium`, `bold`) if not the usual,
    cut to `lines`. `time` says when `at` was, in milliseconds since 1970, as `style`: `relative` (the
    usual), `clock` or `date`.
  - `stat` is one figure: its `value`, a `unit` beside it and a `label` under it. `gauge` is a ring and
    `progress` a bar, each filled to `value`, from 0 to 1, with a `label`. `spark` is a small line of `values`
    up to `max`, and `chart` is bars of `values` over their `labels`, or with `kind: "line"` a line.
  - `item` is a row of a list: its `title`, a grey `subtitle`, and what it ends with, `trailing` text or a time
    `at`. `divider` is a line between parts.
  - `icon` is drawn from an SVG `path` in a box of 24, with a `badge`, another path, at its corner: a brand's
    real logo, never a generic glyph. `image` shows a picture by its `url`, a `data:` address, `width` by
    `height`, `round` for a face.
  - `button` says its `label` and calls the app's `tool` with `inputs` when it's pressed. `switch` says its
    `label` and whether it's `on`, and calls `tool` with `inputs` and the new `on`. The widget is asked for again
    after.
  - Colour is for what needs someone: `tone: "warn"` or `"bad"` on a part, and `"muted"` for what matters
    less. Fine is plain.
  - The first part can say `title: false` when it names itself, and the card leaves its title out. A widget
    with nothing to show yet answers one muted `text`.

  A service is a part of the app that runs only on the machines that have what it needs, and keeps running
  between calls: to watch for something, hold a connection open or keep a program going. It's a module of the
  app that is a GenServer, or a `:gen_statem` when it has stages, and says what it's called where a person
  reads it, and what it needs of a machine:

      defmodule Hello.Door do
        use GenServer
        @behaviour OneCore.Service

        @impl OneCore.Service
        def called, do: "Hello's doorbell"

        @impl OneCore.Service
        def needs, do: "A doorbell plugged into the machine, which writes a line to /dev/ttyUSB0 when it's pressed"

        def rings(service), do: GenServer.call(service, :rings)

        @impl GenServer
        def init(_), do: {:ok, %{bell: Port.open({:spawn, "cat /dev/ttyUSB0"}, [:binary, line: 80]), rings: 0}}

        @impl GenServer
        def handle_call(:rings, _from, state), do: {:reply, state.rings, state}

        @impl GenServer
        def handle_info({bell, {:data, _line}}, %{bell: bell} = state) do
          for who <- OneCore.Service.acts_for(), do: OneCore.Service.app(self(), who, :rang, [state.rings + 1])
          {:noreply, %{state | rings: state.rings + 1}}
        end
      end

  - A service that runs on some systems only says which in `os/0`, as npm's `os` field in package.json does:
    `def os, do: ["darwin"]`. Settings shows it only on servers of those systems, and a server of another refuses it.
  - A service runs on each server whose owner installed it there: the `services` tool lists every service with
    what it needs, and `install_service` runs one on a server. The server starts it, starts it again whenever it
    stops, and stops it when the app is removed. `init/1` is given nothing, and raises if the machine lacks
    what the service needs.
  - Its public functions are what the rest of the app calls, and nothing else can. In a tool,
    `OneCore.Service.call(call, Hello.Door, :rings)` runs `Hello.Door.rings(service)` on a machine that runs the
    service and gives its answer, where `service` is the service's process there. More arguments follow it:
    `OneCore.Service.call(call, Hello.Door, :mute, [30])` runs `mute(service, 30)`. Arguments and answers are what
    JSON can hold. The function runs on that machine for each call, apart from the others, so only what needs
    the service's state has to go through its process.
  - The app decides which machine it calls: `OneCore.Service.servers(call, Hello.Door)` gives the keys of the
    servers that run the service, and `OneCore.Service.call/5` takes one of them last. Left out, it's the server
    running the call if the service runs there, and otherwise the first that's up. `OneCore.Service.server()`
    gives a service the key of its own, and `OneCore.Service.name(server)` what a server's machine is called,
    for a person to pick it by: its machine's own name unless it was started as another.
  - An app that requires this one calls its functions with its own call, and they reach this app's services as
    they would from its own tools.
  - A service calls the app's functions with `OneCore.Service.app(self(), who, :rang, [4])`, which runs
    `Hello.rang(call, 4)` for `who`: one of `OneCore.Service.acts_for()`, which are who the app was installed for that
    this server acts for, or one the app passed it, as `call.installed` or `call.subject`. That function has the call, so
    it's where a service keeps anything in the app's artifact or its database, and says so with
    `OneCore.Call.changed/1` where that changes what a screen shows: nobody else is told it ran. In a function of the service,
    `service` goes where `self()` does. Whoever its server is for comes first in `acts_for()`, so a service
    that's for one person at a time, whoever's machine it's on, takes the first.
  - What a service keeps on its machine goes in `OneCore.Service.folder(self())`, a folder of its own there,
    which stays through its app's updates and goes when it's removed from that server. It runs as whoever
    runs its server there, with their files and their programs.
  - A service removed from a server, or whose app is uninstalled, stops with `{:shutdown, :removed}`: its
    `terminate/2` removes what it put on the machine outside its folder, such as a launchd agent, systemd unit or
    scheduled task. Any other reason, as when its server stops, leaves them.
  - When the app is updated, the service's process carries on in the new code with what it holds, and
    whatever it has open stays open. `code_change/3` is given the state, or a `:gen_statem`'s `code_change/4`
    its state and data, if the new code keeps them in another shape. Keep them in a struct with a default for
    each field, the safe one: `struct(__MODULE__, Map.from_struct(data))` in `code_change` gives the new code's
    struct, with a field the old code didn't have at its default, which a running process otherwise lacks and
    crashes on. A service that can't carry on starts afresh in the new code: one whose `code_change` refuses,
    one that takes over 5 seconds to get to it, one that has become a `:gen_statem` or stopped being one, and
    one whose state holds a function, so keep only data there.
  - A service can bring a program in another language, as source in a folder of the app's files.
    `OneCore.Service.folder(self(), "server/meter")` gives a folder on the machine holding the files under
    `server/meter/`, which is the service's to build the program in, with what the machine has, and to start it
    from as a `Port`. The folder is a new one only when those files change, so a service that kept it can tell in
    `code_change/3` whether to build and start the program again. `needs/0` says what building and running it
    takes. The program stops when its input closes, which is when the service's process ends, and a program
    that stops by itself should stop the service, so that both are started again.

  An app's edge function is what of it has to be reached over the web: a page that opens in a
  browser, or a call from a service outside. It's an ordinary Cloudflare Worker in the app's folder `edge/`,
  with a `wrangler.jsonc`, and a `package.json` that names the first part of each path it answers:

      "paths": ["hello"]

  - It answers every request at the address whose path starts with one of those (`/hello/...`), for the whole
    network. It isn't deployed with the app: whoever deploys the network's edge deploys it with that
    (`core/edge/edge.js`), so it's for the apps a network is set up with.
  - It keeps what it needs in storage of its own, which its `wrangler.jsonc` binds, and is given three bindings
    besides. `env.OWN.write(name, data)` writes a blob of the app's own artifact for the network, only if
    there's none, `env.OWN.update(name, change)` changes one in one step as `OneCore.Artifact.update/3` does, and
    `env.OWN.remove(name)` removes one: that's where the app's server-side code reads what this
    part has to tell it, as `call.own` of the app installed for the network. `env.NETWORK.servers(of)` gives the
    keys of the servers acting for a user, a group or the network, by their prefix (`users/alice/`).
    `env.RELAY.forward(server, message)` passes one JSON-RPC message to a server and gives its answer, or `null` if
    that server isn't up, and `env.RELAY.notify(message)` tells every server a notification.
  - The app's server-side code asks it over the web, signing what it asks as its server does at the edge
    (`OneCore.Edge.signed/2`). `env.NETWORK.verifiedKey(url, headers, body)` gives the key that signed it, or `null`.
  - Its `package.json`'s packages are installed and its `build` script run where the edge is deployed from. A
    page it serves is built there as an app's is, against the UI library (`bun ../../../core/ui/dist/build/app.js <source> <out>`, which `bun edge.js deploy` builds first), which
    the edge serves at `/ui/` as every client has it. `sign-in` does all of this.

  Undo follows what people ordinarily expect of an app. An app that wants more keeps its own history. What a tool
  did outside the network isn't ours to undo.

  Tools to check an app before it's deployed are recommended, not needed.

  Check your app before the people it's for get it:

  - Deploy it to a test install first: for yourself alone, under a name of its own (`tracker-test`), not for the
    group or the network it's for.
  - Try it as the people who'll use it: do what they'll do, on the devices they'll use, a phone first.
  - Take real pictures of its screens (the `chrome`, `iphone-simulator`, `android-emulator` and `computer-use`
    apps' skills say how), look at each, and show them to the person.
  - Try it with worst-case data: long names, nothing at all, huge counts, other scripts. The skill `break-ui`
    (skill://ui/skills/break-ui/SKILL.md) says how.
  """
  @moduledoc "The interface an app's server-side code is written against.\n\n" <> @text

  @doc """
  One of an app's own files, by its path among the files it was deployed with, from any module of the app:
  `OneCore.App.file(__MODULE__, "server/claim.xlsx")`. It's the file as the version running that module was
  deployed with, and one that isn't Elixir, a skill or in `client/`. Raises if the app has no such file.
  """
  def file(module, path) do
    OneCore.Apps.Version.own(module, path) ||
      raise "#{OneCore.Apps.Version.own_name(module)}'s app has no file #{path} outside client/"
  end

  @doc """
  One Core's own skill, `building-apps`, as its `SKILL.md`: how to write an app, which is this module's, and the
  hooks offered by the apps `versions`, by their names.
  """
  def skill(versions) do
    offered =
      for {name, version} <- Enum.sort(versions),
          %{name: hook, kind: kind, description: about} = offer <- version.offers,
          do: {"#{name}.#{hook}", kind, about <> if(offer[:untrusted], do: " An untrusted app may answer it.", else: "")}

    """
    ---
    name: building-apps
    description: Read before making or changing an app. It says how one is written, deployed and updated.
    ---

    """ <>
      @text <>
      if(offered == [],
        do: "",
        else: "\nThe hooks the apps you have offer:\n\n" <> OneCore.App.Manifest.listed(offered) <> "\n"
      )
  end

  require OneCore.App.Manifest
  OneCore.App.Manifest.callbacks()
end
