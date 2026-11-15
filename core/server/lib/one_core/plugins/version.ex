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

  `apps` lists its apps: each one's `name`, its `page`, its `icon` as a `data:` address, and `settings`, whether
  it's a part of Settings. `widgets` lists its widgets: each one's `name`, the `tool` that answers its layout, and
  the `sizes` it comes in.

  `services` lists its services: each one's `name`, which is its module's as the plugin wrote it, what it's
  `called` where a person reads it, and what it `needs` of a machine. `hooks` names the hooks it hooks into, and `jobs` lists its scheduled jobs: each one's
  `tool`, the `inputs` it's called with, and `every` how many seconds. `artifact` is the plugin it's a version of,
  which whatever loaded it says.
  """
  alias OneCore.{App, Builds, Hooks, Server, Skills}

  @enforce_keys [:under, :plugin, :tools, :apps, :widgets, :uses, :services, :hooks, :jobs, :code]
  defstruct [:under, :plugin, :tools, :apps, :widgets, :uses, :services, :hooks, :jobs, :code, :ui, :artifact]

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
         nil <- Enum.find_value(nifs, &compile_nif(&1, rest)),
         {:ok, modules, warnings} <- compile(under, sources),
         {:ok, version} <- describe(under, modules, rest) do
      hashes = Map.new(rest, fn {path, text} -> {path, :crypto.hash(:sha256, text)} end)
      served = Enum.sort(for path <- Map.keys(rest), loaded?(path), do: path)
      own = Map.reject(rest, fn {path, _} -> client?(path) end)
      :persistent_term.put({__MODULE__, under}, %{files: served, hashes: hashes, skills: skills, own: own})
      {:ok, %{version | code: code(files), ui: App.ui(files)}, warnings}
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

  @doc "Removes a version's modules from the VM."
  def unload(%__MODULE__{under: under}), do: unload(under)

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
  # by the build. A name holds one build in the VM, which nothing running on it may lose: the same build again is the
  # one loaded, and another is refused. Gives nil, or what's wrong.
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
      File.rename!(file <> ".part", file <> ".so")
      :persistent_term.put({__MODULE__, module}, {build, file})
      # What doesn't compile says why in its diagnostics.
      compile = fn -> Code.compile_string(source, path) end
      {_, diagnostics} = Code.with_diagnostics(fn -> try(do: compile.(), rescue: (_ in CompileError -> nil)) end)
      {_, said} = :persistent_term.get({__MODULE__, module})

      if :code.is_loaded(module) == false do
        :persistent_term.erase({__MODULE__, module})
        said = if said == file, do: Enum.map_join(diagnostics, "\n", & &1.message), else: said
        {:error, "#{path} didn't load on this machine, #{Builds.target()}: #{said}"}
      end
    else
      {:built, false} -> {:error, "#{path} has no build for this kind of machine, #{library}"}
      {^build, _} -> nil
      names when is_list(names) -> {:error, "#{path} defines one module, which its library was built for"}
      _ -> {:error, "#{path}'s module is loaded already as another build. A new build takes a new module name."}
    end
  end

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
              {:ok, modules, said(warnings.compile_warnings ++ warnings.runtime_warnings, under, folder)}

            outside ->
              Enum.each(outside, &(:code.delete(&1) && :code.purge(&1)))
              {:error, "Every module is a defmodule at the top of a file. These aren't: #{inspect(outside)}"}
          end

        {:error, errors, _warnings} ->
          {:error, said(errors, under, folder)}
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

  defp said(diagnostics, under, folder) do
    Enum.map_join(diagnostics, "\n", fn %{file: file, position: position, message: message} ->
      line = if is_tuple(position), do: elem(position, 0), else: position
      own_names("#{file}:#{line}: #{String.trim(message)}", under) |> String.replace(folder <> "/", "")
    end)
  end

  @doc "A version's module that its plugin wrote as `name`, or `nil` if it has none by that name."
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

  # `rest` is the files that aren't Elixir or skills. A plugin with no Elixir provides no tools and names no apps.
  defp describe(under, modules, rest) do
    case Enum.filter(modules, &(OneCore.Plugin in behaviours(&1))) do
      [] when modules == [] ->
        {:ok, version(under, nil, [], [], [], [], rest)}

      [plugin] ->
        tools = if function_exported?(plugin, :tools, 0), do: plugin.tools(), else: []
        apps = if function_exported?(plugin, :apps, 0), do: plugin.apps(), else: []
        widgets = if function_exported?(plugin, :widgets, 0), do: plugin.widgets(), else: []
        uses = if function_exported?(plugin, :uses, 0), do: plugin.uses(), else: []
        hooks = if function_exported?(plugin, :hooks, 0), do: plugin.hooks(), else: []
        jobs = if function_exported?(plugin, :jobs, 0), do: plugin.jobs(), else: []
        services = Enum.filter(modules, &(OneCore.Service in behaviours(&1)))

        cond do
          not (is_list(tools) and Enum.all?(tools, &tool?/1)) ->
            {:error, "tools/0 gives a list of %{name: ..., description: ..., inputs: ...}"}

          not (is_list(apps) and Enum.all?(apps, &app?(&1, rest))) ->
            {:error,
             "apps/0 gives a list of %{name: ..., page: ..., icon: ...}, where page is a page of client/ and icon a file here"}

          not (is_list(widgets) and Enum.all?(widgets, &widget?(&1, tools))) ->
            {:error,
             "widgets/0 gives a list of %{name: ..., tool: ..., sizes: [...]}: one of this plugin's tools that only " <>
               "reads, and the sizes it comes in, of #{Enum.join(@sizes, ", ")}"}

          not (is_list(uses) and Enum.all?(uses, &is_binary/1)) ->
            {:error, "uses/0 gives a list of plugins' names"}

          not (is_list(hooks) and Enum.all?(hooks, &is_map_key(Hooks.kinds(), &1)) and
                   (hooks == [] or function_exported?(plugin, :hook, 3))) ->
            {:error,
             "hooks/0 gives a list of hooks, and hook/3 is called at each. The hooks are: " <>
               Enum.join(Enum.sort(Map.keys(Hooks.kinds())), ", ")}

          not Enum.all?(services, &(GenServer in behaviours(&1) and is_binary(&1.needs()) and is_binary(&1.called()))) ->
            {:error,
             "A service says `use GenServer`, its needs/0 gives what it needs of a machine, and its called/0 what " <>
               "it's called, each as text"}

          true ->
            services =
              for module <- services, do: %{name: own_name(module), called: module.called(), needs: module.needs()}

            # What each tool takes is read once here, as the schema its inputs are checked against on every call.
            tools =
              for tool <- tools,
                  do: Map.put(tool, :takes, ExJsonSchema.Schema.resolve(JSON.decode!(JSON.encode!(tool.inputs))))

            version = version(under, plugin, tools, apps, widgets, Enum.sort(services), rest)

            with {:ok, jobs} <- jobs(jobs, tools),
                 do: {:ok, %{version | uses: uses, hooks: Enum.uniq(hooks), jobs: jobs}}
        end

      found ->
        {:error, "One module of a plugin says `@behaviour OneCore.Plugin`, and #{length(found)} do"}
    end
  rescue
    error ->
      {:error,
       "tools/0, apps/0, widgets/0, uses/0, hooks/0, jobs/0 or a service's needs/0 or called/0 raised: " <>
         Exception.message(error)}
  end

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

  # A module has one `behaviour` attribute for each behaviour it says it follows.
  defp behaviours(module), do: List.flatten(Keyword.get_values(module.module_info(:attributes), :behaviour))

  # An app's icon goes to clients with its name, as a `data:` address, so a home screen asks for nothing more.
  defp version(under, plugin, tools, apps, widgets, services, rest) do
    apps =
      for %{name: name, page: page, icon: icon} = app <- apps do
        icon = "data:#{kind(icon)};base64,#{Base.encode64(rest[icon])}"
        %{name: name, page: page, icon: icon, settings: app[:settings] == true}
      end

    widgets = for %{name: name, tool: tool, sizes: sizes} <- widgets, do: %{name: name, tool: tool, sizes: sizes}

    %__MODULE__{
      under: under,
      plugin: plugin,
      tools: tools,
      apps: apps,
      widgets: widgets,
      uses: [],
      services: services,
      hooks: [],
      jobs: [],
      code: nil
    }
  end

  @doc "Whether `path` is of what runs in a client: the folder `client/`."
  def client?(path), do: String.starts_with?(path, "client/")

  @doc "Whether `path` is of what runs on a server, its Elixir among it: the folder `server/`."
  def server?(path), do: String.starts_with?(path, "server/")

  # Whether a page loads `path`: a client may load nothing it isn't served (`clients/app`'s `APP_MAY_LOAD`), and
  # is served pages, scripts, stylesheets, pictures, sounds, videos and fonts.
  defp loaded?(path) do
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

  defp app?(%{name: name, page: page, icon: icon}, rest) when is_binary(name),
    do: client?(page) and Path.extname(page) == ".html" and is_map_key(rest, page) and is_map_key(rest, icon)

  defp app?(_, _), do: false

  # A widget is drawn from what one of the plugin's tools answers, which a Home Screen page asks again and again.
  defp widget?(%{name: name, tool: tool, sizes: [_ | _] = sizes}, tools) when is_binary(name),
    do: Enum.any?(tools, &(&1.name == tool and &1[:read_only] == true)) and sizes -- @sizes == []

  defp widget?(_, _), do: false

  defp tool?(%{name: name, description: description, inputs: %{}}) when is_binary(name) and is_binary(description),
    do: name =~ ~r/^[a-z][a-z0-9_]*$/

  defp tool?(_), do: false
end
