defmodule OneCore.Plugins.Version do
  @moduledoc """
  One version of a plugin's code, compiled and loaded.

  The VM keeps only two versions of a module, so each version loaded gets module names of its own: its modules
  sit under a name no other loaded version has (`OneCore.Plugins.Load7.Notes`), by an alias put before each
  file. A version is unloaded by removing every module under that name, which the next version loaded then takes,
  so a server that's updated for years makes no more names than it ever ran versions at once.

  Its code is the Elixir in its folder `server/`, and what of it runs in a client is its folder `client/`.
  `files/1` are what a page loads from there, by their paths: its pages, scripts, stylesheets, pictures and fonts,
  which a client is listed and keeps. The rest of `client/`, a sourcemap, is read by name. `code` names the whole
  set, so a client can tell one version's files from the next's. `hashes/1` names each file that isn't Elixir or a
  skill by what's in it, so a service can tell whether a program it brings changed. `skills/1` are its skills, as
  `OneCore.Skills.found/2` gives them. `own/2` reads one of its other files, outside `client/`, for its own code: a
  template, say.

  Those grow with a plugin's files, and every call is handed the versions it runs on, from another process.
  So they're kept apart from the struct, where reading them copies nothing (`:persistent_term`), and a call costs
  the same however many files a plugin has.

  `ui` is the code of the UI library its app was built against, or `nil` for a plugin with no app (`OneCore.App`).

  `title` is its name where a person reads it, its first app's unless it says, and `icon` its icon as a `data:`
  address, the file it names (`OneCore.Plugin`), or its first app's. `apps` lists its apps: each one's `name`, its
  `page`, its `icon` as a `data:` address, its own or else the plugin's,
  and its `category`, which is the plugin's to say and nil where it says none. `widgets` lists its widgets: each
  one's `name`, the `tool` that answers its layout, and the `sizes` it comes in.

  `services` lists its services: each one's `name`, which is its module's as the plugin wrote it, what it's
  `called` where a person reads it, and what it `needs` of a machine. `hooks` names the hooks it hooks into, `offers` lists the hooks it offers, each one's `name`, `kind` and
  `description`, and `jobs` lists its scheduled jobs: each one's
  `tool`, the `inputs` it's called with, and `every` how many seconds. `essential` is whether a network can't work
  without it (`OneCore.Plugins.Deploy`). `artifact` is the plugin it's a version of, which whatever loaded it says.

  A server that doesn't run a plugin for whoever it's installed for never loads its code, which runs with all of
  its machine's access: it knows the version by what the version says of itself (`description/1`), which a server
  that loaded it kept in the bucket, and is `described/2` from that. A described version has no modules, so it's
  only listed here, and every call to it is run where it's loaded (`OneCore.Plugins.Calls`).
  """
  alias OneCore.{App, Builds, Hooks, Server, Skills}

  @enforce_keys [:under, :plugin, :tools, :apps, :widgets, :uses, :services, :hooks, :offers, :jobs, :code]
  defstruct [
    :under,
    :plugin,
    :tools,
    :apps,
    :widgets,
    :uses,
    :services,
    :hooks,
    :offers,
    :jobs,
    :code,
    :ui,
    :artifact,
    :title,
    :icon,
    essential: false,
    permissions: [],
    sandbox: false
  ]

  # The sizes a widget comes in, which are a phone's own: columns by rows of a Home Screen page's grid.
  @sizes ~w(2x2 4x2 4x4)

  # The names versions are loaded under: each is free, or taken by the one version loaded under it.
  @names 4096
  @on_load :names

  @doc false
  def names, do: :persistent_term.put(__MODULE__, :atomics.new(@names, signed: false))

  @doc "The name of a set of files: the same files have the same name, on any server."
  def code(files),
    do: Base.encode16(:crypto.hash(:sha256, :erlang.term_to_binary(files, [:deterministic])), case: :lower)

  @doc """
  Compiles and loads the files of the plugin named `name`. Gives the version and the compiler's warnings, or what's
  wrong.
  """
  def load(name, files) do
    names = :persistent_term.get(__MODULE__)
    free = Enum.find(1..@names, &(:atomics.compare_exchange(names, &1, 0, 1) == :ok))
    if free == nil, do: raise("#{@names} versions of plugins are loaded, which is as many as a server runs at once")
    under = Module.concat(OneCore.Plugins, "Load#{free}")
    {sources, rest} = Enum.split_with(files, fn {path, _} -> server?(path) and Path.extname(path) == ".ex" end)
    {nifs, sources} = Enum.split_with(sources, fn {path, _} -> Path.dirname(path) == "server/nif" end)

    {skills, rest} =
      Map.split_with(Map.new(rest), fn {path, _} -> path == "SKILL.md" or String.starts_with?(path, "skills/") end)

    with {:ok, skills} <- Skills.found(name, skills),
         {:ok, nif_warnings} <- nifs(nifs, rest),
         {:ok, modules, warnings} <- compile(under, sources),
         {:ok, version} <- describe(under, modules, rest) do
      warnings = Enum.join(Enum.reject([warnings | nif_warnings], &(&1 == "")), "\n")
      hashes = Map.new(rest, fn {path, text} -> {path, :crypto.hash(:sha256, text)} end)
      served = Enum.sort(for path <- Map.keys(rest), loaded?(path), do: path)
      own = Map.reject(rest, fn {path, _} -> client?(path) end)
      :persistent_term.put({__MODULE__, under}, %{files: served, hashes: hashes, skills: skills, own: own})
      {:ok, %{version | code: code(files), ui: App.ui(files), title: version.title || name}, warnings}
    else
      {:error, why} ->
        unload(under)
        {:error, why}
    end
  end

  @doc "What a page of a version's apps loads, by its paths in `client/`."
  def files(%__MODULE__{under: under}), do: :persistent_term.get({__MODULE__, under}).files

  @doc "Each of a version's files that isn't Elixir or a skill, by its path, as the hash of what's in it."
  def hashes(%__MODULE__{under: under}), do: :persistent_term.get({__MODULE__, under}).hashes

  @doc """
  One of the files of the version that `module` is a module of, by its path: one that isn't Elixir, a skill or in
  `client/`, as it was deployed. `nil` if it has no such file.
  """
  def own(module, path), do: :persistent_term.get({__MODULE__, under(module)}).own[path]

  @doc "A version's skills, as `OneCore.Skills.found/2` gives them."
  def skills(%__MODULE__{under: under}), do: :persistent_term.get({__MODULE__, under}).skills

  @doc """
  What a person reads of a version where it's listed (the App Store, the plugin directory), from its skill's
  `metadata` (`OneCore.Plugin`): its `subtitle` and its `about`, each `nil` where it has none.
  """
  def listing(version) do
    metadata =
      case Enum.find(skills(version), &(&1.at == "")) do
        %{frontmatter: %{"metadata" => %{} = metadata}} -> metadata
        _ -> %{}
      end

    for key <- [:subtitle, :about], into: %{} do
      {key, with(text when is_binary(text) <- metadata[Atom.to_string(key)], do: text, else: (_ -> nil))}
    end
  end

  @doc """
  What a loaded version says of itself, as the bucket keeps it: what it provides, and its files and skills, but
  none of its code.
  """
  def description(%__MODULE__{under: under} = version) do
    cached = :persistent_term.get({__MODULE__, under})

    description =
      Map.take(version, [
        :tools,
        :apps,
        :widgets,
        :uses,
        :services,
        :hooks,
        :offers,
        :jobs,
        :code,
        :ui,
        :essential,
        :title,
        :icon,
        :permissions
      ])

    # It holds only names any server knows: a tool's inputs as JSON has them, not the atoms its plugin wrote them
    # with, which a server that never loaded the plugin doesn't have, and won't take from the bucket.
    tools =
      for tool <- description.tools do
        tool
        |> Map.take([:name, :description, :inputs, :outputs, :read_only, :by_link, :ui])
        |> Map.update!(:inputs, &JSON.decode!(JSON.encode!(&1)))
        |> Map.replace_lazy(:outputs, &JSON.decode!(JSON.encode!(&1)))
      end

    :erlang.term_to_binary(%{description | tools: tools} |> Map.merge(%{files: cached.files, skills: cached.skills}))
  end

  @doc """
  A version of the plugin `artifact` from what it said of itself (`description/1`): `{:ok, version}`, or `:error`
  where that isn't one. It was written by a server that runs the plugin, whoever's that is, so only what's
  listed of it is taken, and what each tool takes and gives is read from its inputs and outputs again.
  """
  def described(artifact, description) do
    # Its skills are as `OneCore.Skills` says them, whose names a server that has just started may not know yet.
    Code.ensure_loaded!(Skills)
    description = :erlang.binary_to_term(description, [:safe])
    under = {:described, artifact, description.code}

    tools = Enum.map(description.tools, &schemas/1)

    :persistent_term.put({__MODULE__, under}, %{files: description.files, hashes: %{}, skills: description.skills, own: %{}})

    fields =
      Map.take(description, [
        :apps,
        :widgets,
        :uses,
        :services,
        :hooks,
        :offers,
        :jobs,
        :code,
        :ui,
        :essential,
        :title,
        :icon,
        :permissions
      ])

    {:ok, struct!(__MODULE__, Map.merge(fields, %{under: under, plugin: nil, tools: tools, artifact: artifact}))}
  rescue
    _ -> :error
  end

  @doc """
  A version of the plugin `artifact` in sandbox mode, from its files as a server keeps them and what its code said of
  itself in its own VM (`OneCore.Sandbox`): `{:ok, version}`, or `{:error, why}`. Its code never loads
  here, so it's described as a version a server doesn't run is, and is checked as one that loads is.
  """
  def sandboxed(artifact, files, description) do
    name = Path.basename(artifact)
    code = code(files)
    under = {:described, artifact, code}
    rest = Map.reject(files, fn {path, _} -> server?(path) and Path.extname(path) == ".ex" end)

    {skills, rest} =
      Map.split_with(rest, fn {path, _} -> path == "SKILL.md" or String.starts_with?(path, "skills/") end)

    taken = fn list, keys ->
      for item <- List.wrap(list),
          is_map(item),
          do: Map.new(Map.take(item, keys), &{String.to_existing_atom(elem(&1, 0)), elem(&1, 1)})
    end

    offers =
      for offer <- taken.(description["offers"], ~w(name kind description)), do: Map.update(offer, :kind, nil, &hook_kind/1)

    description = %{
      tools: taken.(description["tools"], ~w(name description inputs outputs read_only by_link ui)),
      apps: taken.(description["apps"], ~w(name page icon category)),
      title: description["title"],
      icon: description["icon"],
      widgets: taken.(description["widgets"], ~w(name tool sizes)),
      uses: description["uses"],
      hooks: description["hooks"],
      offers: offers,
      jobs: taken.(description["jobs"], ~w(tool every inputs)),
      essential: false,
      permissions: description["permissions"]
    }

    with {:ok, skills} <- Skills.found(name, skills),
         {:ok, version} <- checked(under, nil, description, [], rest) do
      served = Enum.sort(for path <- Map.keys(rest), loaded?(path), do: path)
      :persistent_term.put({__MODULE__, under}, %{files: served, hashes: %{}, skills: skills, own: %{}})

      version = %{
        version
        | code: code,
          ui: App.ui(files),
          title: version.title || name,
          artifact: artifact,
          sandbox: true
      }

      {:ok, version}
    end
  end

  defp hook_kind(kind) when kind in ["observe", "mutate", "validate", "collect"], do: String.to_existing_atom(kind)
  defp hook_kind(_kind), do: nil

  @doc "Whether a version is only described here, not loaded (`described/2`)."
  def described?(%__MODULE__{under: under}), do: is_tuple(under)

  @doc "Removes a version's modules from the VM."
  def unload(%__MODULE__{under: under}), do: unload(under)

  def unload({:described, _artifact, _code} = under), do: :persistent_term.erase({__MODULE__, under})

  def unload(under) do
    :persistent_term.erase({__MODULE__, under})

    for {module, _} <- :code.all_loaded(), under?(module, under) do
      :code.delete(module)
      :code.purge(module)
    end

    "Load" <> name = List.last(Module.split(under))
    :atomics.put(:persistent_term.get(__MODULE__), String.to_integer(name), 0)
  end

  defp under?(module, under), do: String.starts_with?(Atom.to_string(module), Atom.to_string(under) <> ".")

  # A NIF's module, `server/nif/<name>.ex`, is compiled under the name it gives it, which its library was built for,
  # with that library as it's built for this kind of machine, `server/nif/<name>/<kind>.so`, written to a folder named
  # by the build. The VM loads a library into a module once, and Rustler can't swap it for another while the module's
  # in use (it has no `upgrade`), so a name holds the first build loaded until the server restarts: the same build
  # again is that one, and another is kept for the next start, which loads the latest version's, and is said in the
  # warnings. Gives `{:ok, warnings}`, or what's wrong.
  defp nifs(nifs, rest) do
    Enum.reduce_while(nifs, {:ok, []}, fn nif, {:ok, warnings} ->
      case compile_nif(nif, rest) do
        :ok -> {:cont, {:ok, warnings}}
        {:warning, warning} -> {:cont, {:ok, [warning | warnings]}}
        {:error, _why} = error -> {:halt, error}
      end
    end)
  end

  defp compile_nif({path, source}, rest) do
    library = Path.rootname(path) <> "/#{Builds.target()}.so"
    build = code({source, rest[library]})

    with [name] <- defined(source),
         module = Module.concat([name]),
         {:built, true} <- {:built, is_map_key(rest, library)},
         nil <- :persistent_term.get({__MODULE__, module}, nil),
         :non_existing <- :code.which(module) do
      file = Path.join([Server.home(), "nif", build, Path.basename(path, ".ex")])
      File.mkdir_p!(Path.dirname(file))
      # Renamed into place, as another VM on this home may have the same library loaded from it.
      File.write!(file <> ".part", rest[library])
      File.rename!(file <> ".part", file <> library_extension())
      :persistent_term.put({__MODULE__, module}, {build, file})
      # What doesn't compile says why in its diagnostics.
      compile = fn -> Code.compile_string(source, path) end
      {_, diagnostics} = Code.with_diagnostics(fn -> try(do: compile.(), rescue: (_ in CompileError -> nil)) end)
      {_, message} = :persistent_term.get({__MODULE__, module})

      if :code.is_loaded(module) == false do
        :persistent_term.erase({__MODULE__, module})
        message = if message == file, do: Enum.map_join(diagnostics, "\n", & &1.message), else: message
        {:error, "#{path} didn't load on this machine, #{Builds.target()}: #{message}"}
      else
        :ok
      end
    else
      {:built, false} -> {:error, "#{path} has no build for this kind of machine, #{library}"}
      {^build, _} -> :ok
      {_other, _} -> {:warning, "#{path}: this server runs the NIF it loaded until it restarts, and this one after"}
      names when is_list(names) -> {:error, "#{path} defines one module, which its library was built for"}
      _ -> {:error, "#{path}'s module is one of the server's own"}
    end
  end

  # What the VM loads a library by, which it finds by the name without it: a DLL on Windows.
  defp library_extension, do: if(match?({:win32, _}, :os.type()), do: ".dll", else: ".so")

  @doc false
  def nif(module, load) do
    {build, file} = :persistent_term.get({__MODULE__, module})
    loaded = load.(String.to_charlist(file))
    with {:error, {_, why}} <- loaded, do: :persistent_term.put({__MODULE__, module}, {build, to_string(why)})
    loaded
  end

  # The files go to a folder of their own to be compiled together, so they can use each other's structs and macros
  # in any order. The aliases share each file's first line, which keeps its line numbers.
  defp compile(under, sources) do
    folder = Path.join([Server.home(), "compiling", List.last(Module.split(under))])
    names = Enum.uniq(for {_, source} <- sources, name <- defined(source), do: name)
    aliases = Enum.map_join(names, &"alias #{inspect(Module.concat(under, &1))}, as: #{&1}, warn: false; ")

    paths =
      for {path, source} <- sources do
        file = Path.join(folder, path)
        File.mkdir_p!(Path.dirname(file))
        File.write!(file, aliases <> source)
        file
      end

    try do
      case Kernel.ParallelCompiler.compile(paths, return_diagnostics: true) do
        {:ok, modules, warnings} ->
          case Enum.reject(modules, &under?(&1, under)) do
            [] ->
              {:ok, modules, messages(warnings.compile_warnings ++ warnings.runtime_warnings, under, folder)}

            outside ->
              Enum.each(outside, &(:code.delete(&1) && :code.purge(&1)))
              {:error, "Every module is a defmodule at the top of a file. These aren't: #{inspect(outside)}"}
          end

        {:error, errors, _warnings} ->
          {:error, messages(errors, under, folder)}
      end
    after
      File.rm_rf!(folder)
    end
  end

  # The first part of each module name a file defines. A file that doesn't parse defines none, and the compiler
  # says what's wrong with it.
  defp defined(source) do
    case Code.string_to_quoted(source) do
      {:ok, {:__block__, _, forms}} -> Enum.flat_map(forms, &defined_by/1)
      {:ok, form} -> defined_by(form)
      {:error, _} -> []
    end
  end

  defp defined_by({definer, _, [{:__aliases__, _, [name | _]} | _]}) when definer in [:defmodule, :defprotocol],
    do: [name]

  defp defined_by(_), do: []

  defp messages(diagnostics, under, folder) do
    Enum.map_join(diagnostics, "\n", fn %{file: file, position: position, message: message} ->
      line = if is_tuple(position), do: elem(position, 0), else: position
      own_names("#{file}:#{line}: #{String.trim(message)}", under) |> String.replace(folder <> "/", "")
    end)
  end

  @doc "A version's module that its plugin wrote as `name`, or `nil` if it has none by that name."
  def module(%__MODULE__{under: {:described, _artifact, _code}}, _name), do: nil

  def module(%__MODULE__{under: under}, name) do
    module = Module.concat(under, name)
    if :code.is_loaded(module), do: module
  end

  @doc "The name a version's modules sit under, from one of them."
  def under(module), do: module |> Module.split() |> Enum.take(3) |> Module.concat()

  @doc "The name of a version's module as its plugin wrote it: what follows the name the version's modules sit under."
  def own_name(module), do: module |> Module.split() |> Enum.drop(3) |> Enum.join(".")

  @doc "`text` with a version's module names as its plugin wrote them."
  def own_names(text, %__MODULE__{under: under}), do: own_names(text, under)
  def own_names(text, under), do: String.replace(text, inspect(under) <> ".", "")

  # What a plugin says of itself, by the functions that say it, with what each gives where it has none.
  @callbacks [
    tools: [],
    apps: [],
    title: nil,
    icon: nil,
    widgets: [],
    uses: [],
    hooks: [],
    offers: [],
    jobs: [],
    essential: false,
    permissions: []
  ]

  # `rest` is the files that aren't Elixir or skills. A plugin with no Elixir provides no tools and names no apps.
  defp describe(under, modules, rest) do
    case Enum.filter(modules, &(OneCore.Plugin in behaviours(&1))) do
      [] when modules == [] ->
        {:ok, version(under, nil, [], [], [], [], rest, nil)}

      [plugin] ->
        description =
          for {function, none} <- @callbacks,
              into: %{},
              do: {function, if(function_exported?(plugin, function, 0), do: apply(plugin, function, []), else: none)}

        services = Enum.filter(modules, &(OneCore.Service in behaviours(&1)))

        process? = &(GenServer in behaviours(&1) or :gen_statem in behaviours(&1))

        if Enum.all?(services, &(process?.(&1) and is_binary(&1.needs()) and is_binary(&1.called()))) do
          services =
            for module <- services, do: %{name: own_name(module), called: module.called(), needs: module.needs()}

          checked(under, plugin, description, services, rest)
        else
          {:error,
           "A service says `use GenServer` or `@behaviour :gen_statem`, its needs/0 gives what it needs of a " <>
             "machine, and its called/0 what it's called, each as text"}
        end

      found ->
        {:error, "One module of a plugin says `@behaviour OneCore.Plugin`, and #{length(found)} do"}
    end
  rescue
    error ->
      {:error,
       "tools/0, apps/0, widgets/0, uses/0, hooks/0, offers/0, jobs/0, essential/0, permissions/0 or a service's " <>
         "needs/0 or called/0 raised: " <>
         Exception.message(error)}
  end

  # A version from what its plugin said of itself, if that's what a plugin says: `plugin` is its module, or nil for
  # one in sandbox mode, whose module is in its own VM.
  defp checked(under, plugin, description, services, rest) do
    %{tools: tools, apps: apps, title: title, icon: icon, widgets: widgets, uses: uses, hooks: hooks} = description
    %{offers: offers, jobs: jobs, permissions: permissions} = description

    cond do
      not (is_list(tools) and Enum.all?(tools, &tool?(&1, rest))) ->
        {:error,
         "tools/0 gives a list of %{name: ..., description: ..., inputs: ..., outputs: ...}, where outputs is a JSON " <>
           "Schema of an object, and ui, if a tool has one, is a page of client/"}

      not ((title == nil or is_binary(title)) and (icon == nil or is_map_key(rest, icon))) ->
        {:error, "title/0 gives text, and icon/0 a file here"}

      not (is_list(apps) and Enum.all?(apps, &app?(&1, rest, icon))) ->
        {:error,
         "apps/0 gives a list of %{name: ..., page: ..., icon: ...}, where page is a page of client/ and icon a file " <>
           "here, which icon/0 gives instead where an app has none, and category, if it has one, is text"}

      not (is_list(widgets) and Enum.all?(widgets, &widget?(&1, tools))) ->
        {:error,
         "widgets/0 gives a list of %{name: ..., tool: ..., sizes: [...]}: one of this plugin's tools that only " <>
           "reads, and the sizes it comes in, of #{Enum.join(@sizes, ", ")}"}

      not (is_list(uses) and Enum.all?(uses, &is_binary/1)) ->
        {:error, "uses/0 gives a list of plugins' names"}

      not (is_list(hooks) and Enum.all?(hooks, &Hooks.hook?/1) and
               (hooks == [] or plugin == nil or function_exported?(plugin, :hook, 3))) ->
        {:error,
         "hooks/0 gives a list of hooks, and hook/3 is called at each. The hooks are One Core's, " <>
           Enum.join(Enum.sort(Map.keys(Hooks.kinds())), ", ") <>
           ", and the ones plugins offer, each as <plugin>.<hook>: recently-deleted.removed"}

      not (is_list(offers) and Enum.all?(offers, &offer?/1)) ->
        {:error,
         "offers/0 gives a list of %{name: ..., kind: ..., description: ...}: a name of lowercase letters, " <>
           "digits and _, and a kind of :observe, :mutate, :validate or :collect"}

      not (is_list(permissions) and Enum.all?(permissions, &permission?/1)) ->
        {:error,
         "permissions/0 gives a list of what the plugin may do beyond its own: another plugin's tools, by the " <>
           "plugin's name or a tool's (`gmail`, `notifications_send`), and the hosts it sends data to " <>
           "(`https://api.example.com`, `https://*.googleapis.com`)"}

      true ->
        tools = Enum.map(tools, &schemas/1)

        version = version(under, plugin, tools, apps, widgets, Enum.sort(services), rest, icon)
        version = %{version | title: title || version.title}

        with {:ok, jobs} <- jobs(jobs, tools),
             do:
               {:ok,
                %{
                  version
                  | uses: uses,
                    hooks: Enum.uniq(hooks),
                    offers: offers,
                    jobs: jobs,
                    essential: description.essential == true,
                    permissions: Enum.uniq(permissions)
                }}
    end
  end

  # What a plugin may do beyond its own: call another plugin's tools, all of them or one, or send data to a host.
  defp permission?(permission) when is_binary(permission) do
    case URI.parse(permission) do
      %URI{scheme: scheme, host: host, path: nil, query: nil, userinfo: nil} when scheme in ["https", "http"] ->
        is_binary(host) and host =~ ~r/^(\*\.)?[a-z0-9-]+(\.[a-z0-9-]+)*$/

      _ ->
        permission =~ ~r/^[a-z0-9][a-z0-9-]*(_[a-z][a-z0-9_]*)?$/
    end
  end

  defp permission?(_permission), do: false

  defp offer?(%{name: name, kind: kind, description: description}),
    do:
      is_binary(name) and name =~ ~r/^[a-z][a-z0-9_]*$/ and kind in [:observe, :mutate, :validate, :collect] and
        is_binary(description)

  defp offer?(_offer), do: false

  # A scheduled job calls one of the plugin's tools with inputs the tool takes, so one that could never run is
  # found out when it's deployed.
  defp jobs(jobs, tools) do
    checked =
      for job <- List.wrap(jobs) do
        with %{tool: name, every: every} when is_integer(every) and every > 0 <- job,
             inputs = JSON.decode!(JSON.encode!(Map.get(job, :inputs, %{}))),
             %{takes: takes} <- Enum.find(tools, &(&1.name == name)),
             :ok <- ExJsonSchema.Validator.validate(takes, inputs) do
          %{tool: name, every: every, inputs: inputs}
        else
          _ -> nil
        end
      end

    if is_list(jobs) and nil not in checked,
      do: {:ok, checked},
      else:
        {:error,
         "jobs/0 gives a list of %{tool: ..., every: ...}: one of this plugin's tools, how many seconds apart, and the inputs it takes, if any"}
  end

  @doc "The behaviours a module says it follows, one `behaviour` attribute each."
  def behaviours(module), do: List.flatten(Keyword.get_values(module.module_info(:attributes), :behaviour))

  # An icon goes to clients as a `data:` address, so a home screen asks for nothing more. The plugin's is the file it
  # names, and an app's its own or else the plugin's; a plugin that names none is drawn as its first app is.
  defp version(under, plugin, tools, apps, widgets, services, rest, icon) do
    apps =
      for %{name: name, page: page} = app <- apps,
          do: %{name: name, page: page, icon: data(app[:icon] || icon, rest), category: app[:category]}

    widgets = for %{name: name, tool: tool, sizes: sizes} <- widgets, do: %{name: name, tool: tool, sizes: sizes}

    %__MODULE__{
      under: under,
      plugin: plugin,
      tools: tools,
      apps: apps,
      title: List.first(for(app <- apps, do: app.name)),
      icon: if(icon, do: data(icon, rest), else: List.first(for(app <- apps, do: app.icon))),
      widgets: widgets,
      uses: [],
      services: services,
      hooks: [],
      offers: [],
      jobs: [],
      code: nil
    }
  end

  @doc "Whether `path` is of what runs in a client: the folder `client/`."
  def client?(path), do: String.starts_with?(path, "client/")

  @doc "Whether `path` is of what runs on a server, its Elixir among it: the folder `server/`."
  def server?(path), do: String.starts_with?(path, "server/")

  @doc """
  Whether a page loads `path`: a client may load nothing it isn't served (`clients/app`'s `APP_MAY_LOAD`), and is
  served pages, scripts, stylesheets, pictures, sounds, videos and fonts.
  """
  def loaded?(path) do
    client?(path) and
      (Path.extname(path) in ~w(.html .js .css) or
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

  defp data(path, rest), do: "data:#{kind(path)};base64,#{Base.encode64(rest[path])}"

  defp app?(%{name: name, page: page} = app, rest, icon) when is_binary(name),
    do:
      page?(page, rest) and is_map_key(rest, app[:icon] || icon) and
        (app[:category] == nil or is_binary(app[:category]))

  defp app?(_, _, _), do: false

  # A widget is drawn from what one of the plugin's tools answers, which a Home Screen page asks again and again.
  defp widget?(%{name: name, tool: tool, sizes: [_ | _] = sizes}, tools) when is_binary(name),
    do: Enum.any?(tools, &(&1.name == tool and &1[:read_only] == true)) and sizes -- @sizes == []

  defp widget?(_, _), do: false

  defp page?(page, rest),
    do: is_binary(page) and client?(page) and Path.extname(page) == ".html" and is_map_key(rest, page)

  # A tool may have a page of its own, its UI as MCP Apps has it, which a host shows for it. What it answers is an
  # object, which is what every version of MCP a client speaks takes as a tool's `outputSchema`. A version deployed
  # before tools said what they answer says nothing of it, and still loads (`OneCore.Plugins.Deploy` refuses a new one).
  defp tool?(%{name: name, description: description, inputs: %{}} = tool, rest)
       when is_binary(name) and is_binary(description),
       do: name =~ ~r/^[a-z][a-z0-9_]*$/ and (tool[:ui] == nil or page?(tool[:ui], rest)) and outputs?(tool[:outputs])

  defp tool?(_, _), do: false

  defp outputs?(nil), do: true
  defp outputs?(%{} = outputs), do: JSON.decode!(JSON.encode!(outputs))["type"] == "object"
  defp outputs?(_outputs), do: false

  @doc """
  A tool with what it takes and what it gives read once, as the JSON Schemas its inputs and its answers are checked
  against on every call: `takes` from its `inputs`, and `gives` from its `outputs`, where it says them.
  """
  def schemas(tool) do
    resolve = &ExJsonSchema.Schema.resolve(JSON.decode!(JSON.encode!(&1)))
    tool = Map.put(tool, :takes, resolve.(tool.inputs))
    if tool[:outputs], do: Map.put(tool, :gives, resolve.(tool.outputs)), else: tool
  end
end
