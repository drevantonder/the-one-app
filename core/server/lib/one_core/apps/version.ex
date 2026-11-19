defmodule OneCore.Apps.Version do
  @moduledoc """
  One version of an app's code, loaded or described: its modules sit under a name of its own while it's loaded
  (`OneCore.Apps.LoadedVersion`), which compiles it (`OneCore.Apps.Version.Compile`), checks what it says of
  itself (`OneCore.Apps.Version.Checks`), or describes it without its code (`OneCore.Apps.Version.Description`).

  Its code is the Elixir in its folder `server/`, and what of it runs in a client is its folder `client/`.
  `files/1` are what a page loads from there, by their paths: its pages, scripts, stylesheets, pictures and fonts,
  which a client is listed and keeps. The rest of `client/`, a sourcemap, is read by name. `code` names the whole
  set, so a client can tell one version's files from the next's. `hashes/1` names each file that isn't Elixir or a
  skill by what's in it, so a service can tell whether a program it brings changed. `skills/1` are its skills, as
  `OneCore.Skills.found/2` gives them. `own/2` reads one of its other files, outside `client/`, for its own code: a
  template, say.

  Those grow with an app's files, and every call is handed the versions it runs on, from another process.
  So they're kept apart from the struct, where reading them copies nothing (`OneCore.Apps.LoadedVersion.kept/1`),
  and a call costs the same however many files an app has.

  `ui` is the code of the UI library its app was built against, or `nil` for an app with no entrypoint (`OneCore.Entrypoint`).

  `title` is its name where a person reads it, which its skill's metadata says (`title/1`), and `icon` its icon as a `data:`
  address, the file it names (`OneCore.App`). `entrypoints` lists its entrypoints: each one's `name`, its
  `page`, its `icon` as a `data:` address, its own or else the app's,
  its `category`, its `subject`, its `badge` and its `runtime_version`, which are the app's to say and nil where it says none. `widgets` lists its widgets: each
  one's `name`, the `tool` that answers its layout, and the `sizes` it comes in.

  `services` lists its services: each one's `name`, which is its module's as the app wrote it, what it's `called`
  where a person reads it, what it `needs` of a machine, and the `os` it runs on, as npm's `os` names them, or `nil` for any. `hooks` names the hooks it hooks into, `offers` lists the
  hooks it offers, each one's `name`, `kind` and `description`, and `jobs` lists its scheduled jobs: each one's `tool`,
  the `inputs` it's called with, and `every` how many seconds. `requires` names the other apps it needs, each with
  why, and `core` the lowest version of the platform it runs on, as an Elixir requirement (`>= 0.15.0`), or `nil` for
  any, both from its `requires/0` (`OneCore.App`). `own` says whose its own artifact is (`OneCore.Apps.Calls.own/3`):
  `:subject`, whoever each call runs for, or `:installed`, whoever it was installed for. `exports` are the functions its module provides other apps, as name/arity, which run inside
  each app that requires it. `artifact` is the app it's a version of, which whatever loaded it says.

  A version runs only on a server on its `core` or newer (`platform/1`): one older is refused where it's deployed, and
  where it's loaded runs nowhere here, as one that doesn't compile, until the server is updated.

  A server that doesn't run an app for whoever it's installed for never loads its code: it knows the version only
  by what it says of itself (`OneCore.Apps.Version.Description`).
  """
  alias OneCore.Builds
  alias OneCore.Apps.LoadedVersion

  @enforce_keys [:under, :app, :tools, :entrypoints, :widgets, :services, :hooks, :offers, :jobs, :code]
  defstruct [
    :under,
    :app,
    :tools,
    :entrypoints,
    :widgets,
    :services,
    :hooks,
    :offers,
    :jobs,
    :code,
    :ui,
    :artifact,
    :title,
    :icon,
    :core,
    requires: %{},
    exports: [],
    own: :subject,
    trusted: true
  ]

  @doc "The name of a set of files: the same files have the same name, on any server."
  def code(files),
    do: Base.encode16(:crypto.hash(:sha256, :erlang.term_to_binary(files, [:deterministic])), case: :lower)

  @doc "What a page of a version.s entrypoints loads, by its paths in `client/`."
  def files(%__MODULE__{under: under}), do: LoadedVersion.kept(under).files

  @doc "Each of a version's files that isn't Elixir or a skill, by its path, as the hash of what's in it."
  def hashes(%__MODULE__{under: under}), do: LoadedVersion.kept(under).hashes

  @doc """
  One of the files of the version that `module` is a module of, by its path: one that isn't Elixir, a skill or in
  `client/`, as it was deployed. `nil` if it has no such file.
  """
  def own(module, path), do: LoadedVersion.kept(under(module)).own[path]

  @doc "A version's skills, as `OneCore.Skills.found/2` gives them."
  def skills(%__MODULE__{under: under}), do: LoadedVersion.kept(under).skills

  @doc """
  What a person reads of a version where it's listed (the app directory, a marketplace): its `title`, `icon`,
  `entrypoints` (each one's `name`, `icon` and `category`) and `widgets` (each one's `name` and `sizes`), and from its
  skill's `metadata` (`OneCore.App`) its `subtitle`, `about` and `category`, each `nil` where it has none.
  """
  def listing(version), do: listing(version, skills(version))

  @doc """
  What a version of the app `artifact` described (`OneCore.Apps.Version.Description.description/1`) lists, as
  `listing/1` with what the App Store's Marketplace says besides: its skill's `description`, its `tools`' names and its
  `requires`. `{:ok, listing}`, or `:error` where that isn't a description. Nothing is loaded.
  """
  def listed(artifact, description) do
    with {:ok, version, kept} <- OneCore.Apps.Version.Description.described(artifact, description) do
      skill = Enum.find(kept.skills, &(&1.at == "")) || %{frontmatter: %{}}

      {:ok,
       Map.merge(listing(version, kept.skills), %{
         description: skill.frontmatter["description"] || "",
         tools: Enum.map(version.tools, & &1.name),
         requires: version.requires || %{}
       })}
    end
  end

  @doc """
  An app's name where a person reads it, from its skills as `OneCore.Skills.found/2` gives them: its skill's
  `metadata` `title` (`OneCore.App`), or nil where it has none.
  """
  def title(skills), do: text(metadata(skills)["title"])

  defp metadata(skills) do
    case Enum.find(skills, &(&1.at == "")) do
      %{frontmatter: %{"metadata" => %{} = metadata}} -> metadata
      _ -> %{}
    end
  end

  defp listing(version, skills) do
    metadata = metadata(skills)

    text = for key <- ~w(subtitle about category), into: %{}, do: {String.to_atom(key), text(metadata[key])}

    Map.merge(text, %{
      title: version.title,
      icon: version.icon,
      entrypoints: for(entrypoint <- version.entrypoints, do: Map.take(entrypoint, [:name, :icon, :category])),
      widgets: for(widget <- version.widgets, do: Map.take(widget, [:name, :sizes]))
    })
  end

  defp text(text) when is_binary(text), do: text
  defp text(_), do: nil

  @doc """
  Whether a version runs on this server, from its `core` and the server's build, worked out each time it's asked,
  never kept: `:runs` where the server's build is its `core` or newer, or it says none, and `:waiting_for_server`
  where it needs a newer platform, until the servers are updated. `core` is a minimum, as Android's `minSdkVersion`
  is: a newer platform runs it, so a breaking release strands no app.
  """
  def platform(%__MODULE__{core: nil}), do: :runs

  def platform(%__MODULE__{core: core}) do
    if Elixir.Version.compare(Builds.version(Builds.build()), lowest(core)) == :lt,
      do: :waiting_for_server,
      else: :runs
  end

  @doc "Why the version `version` of the app `name` doesn't run on this server (`platform/1`), or `nil` where it does."
  def waiting(name, version) do
    case platform(version) do
      :runs ->
        nil

      :waiting_for_server ->
        "#{name} waits for the servers' update: it requires core #{version.core}, and this server is on #{Builds.build()}"
    end
  end

  # The lowest version a requirement allows, the first it names: `>= 0.16.0`'s 0.16.0, and a ceiling an app said
  # before `core` was a minimum, `~> 0.16.0`, is read as that too.
  defp lowest(core) do
    [named] = Regex.run(~r/\d+\.\d+(?:\.\d+)?/, core)
    Elixir.Version.parse!(if length(String.split(named, ".")) == 2, do: named <> ".0", else: named)
  end

  @doc "Whether a version is only described here, not loaded (`described/2`)."
  def described?(%__MODULE__{under: under}), do: is_tuple(under)

  @doc """
  Where a version runs, as this server holds it: `:here`, loaded, its module running each call; `:sandbox`, an
  untrusted one, only described here and run in its sandbox (`OneCore.Sandbox`); or `:elsewhere`, a trusted one this
  server only knows by its description, which a server that runs its owner's apps loads.
  """
  def runs(%__MODULE__{} = version) do
    cond do
      not described?(version) -> :here
      version.trusted -> :elsewhere
      true -> :sandbox
    end
  end

  @doc "A version's module that its app wrote as `name`, or `nil` if it has none by that name."
  def module(%__MODULE__{under: {:described, _artifact, _code}}, _name), do: nil

  def module(%__MODULE__{under: under}, name) do
    module = Module.concat(under, name)
    if :code.is_loaded(module), do: module
  end

  @doc "The name a version's modules sit under, from one of them."
  def under(module), do: module |> Module.split() |> Enum.take(3) |> Module.concat()

  @doc "The name of a version's module as its app wrote it: what follows the name the version's modules sit under."
  def own_name(module), do: module |> Module.split() |> Enum.drop(3) |> Enum.join(".")

  @doc "`text` with a version's module names as its app wrote them."
  def own_names(text, %__MODULE__{under: under}), do: own_names(text, under)
  def own_names(text, under), do: String.replace(text, inspect(under) <> ".", "")

  @doc "The behaviours a module says it follows, one `behaviour` attribute each."
  def behaviours(module), do: List.flatten(Keyword.get_values(module.module_info(:attributes), :behaviour))

  @doc "Whether `path` is of what runs in a client: the folder `client/`."
  def client?(path), do: String.starts_with?(path, "client/")

  @doc "Whether `path` is of what runs on a server, its Elixir among it: the folder `server/`."
  def server?(path), do: String.starts_with?(path, "server/")

  @doc """
  Whether a page loads `path`: a client may load nothing it isn't served (`clients/app`'s `APP_MAY_LOAD`), and is
  served pages, scripts, stylesheets, the style tables a native host draws a page with (`styles.json`, the UI
  library's `tokens.json`), pictures, sounds, videos and fonts. What's listed is kept with its version, so a client
  that opens from what it kept has every file it loads.
  """
  def loaded?(path) do
    client?(path) and
      (Path.extname(path) in ~w(.html .js .css .json) or
         String.starts_with?(MIME.from_path(path), ~w(image/ audio/ video/ font/)))
  end

  @doc "What kind of file `path` is, as a web view is told it."
  def kind(path) do
    case Path.extname(path) do
      # What MCP Apps calls a page of an app.
      ".html" -> "text/html;profile=mcp-app"
      ".map" -> "application/json"
      _ -> MIME.from_path(path)
    end
  end

  @doc """
  A tool with what it takes and what it gives read once, as the JSON Schemas its inputs and its answers are checked
  against on every call: `takes` from its `inputs`, and `gives` from its `outputs`, where it says them.
  """
  def schemas(tool) do
    resolve = &ExJsonSchema.Schema.resolve(OneCore.JSON.normalize(&1))
    tool = Map.put(tool, :takes, resolve.(tool.inputs))
    if tool[:outputs], do: Map.put(tool, :gives, resolve.(tool.outputs)), else: tool
  end
end
