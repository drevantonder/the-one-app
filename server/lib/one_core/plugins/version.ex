defmodule OneCore.Plugins.Version do
  @moduledoc """
  One version of a plugin's code, compiled and loaded.

  The VM keeps only two versions of a module, so each version loaded gets module names of its own: its modules
  sit under a name no other version has (`OneCore.Plugins.Load7.Tracker`), by an alias put before each file. A
  version is unloaded by removing every module under that name.

  A version's files that aren't Elixir are its parts that run in a client (`files`, by their paths), and `code` names
  the whole set, so a client can tell one version's files from the next's. `hashes` names each of those files by
  what's in it, so a service can tell whether a program it brings changed.

  `services` lists its services: each one's `name`, which is its module's as the plugin wrote it, and what it
  `needs` of a machine.
  """

  @enforce_keys [:under, :plugin, :tools, :guide, :apps, :uses, :services, :code, :files, :hashes]
  defstruct [:under, :plugin, :tools, :guide, :apps, :uses, :services, :code, :files, :hashes]

  @doc "The name of a set of files: the same files have the same name, on any server."
  def code(files),
    do: Base.encode16(:crypto.hash(:sha256, :erlang.term_to_binary(files, [:deterministic])), case: :lower)

  @doc "Compiles and loads a plugin's files. Gives the version and the compiler's warnings, or what's wrong."
  def load(files) do
    under = Module.concat(OneCore.Plugins, "Load#{System.unique_integer([:positive])}")
    {sources, client} = Enum.split_with(files, fn {path, _} -> String.ends_with?(path, ".ex") end)

    with {:ok, modules, warnings} <- compile(under, sources),
         {:ok, version} <- describe(under, modules, Map.new(client)) do
      {:ok, %{version | code: code(files)}, warnings}
    else
      {:error, why} ->
        unload(under)
        {:error, why}
    end
  end

  @doc "Removes a version's modules from the VM."
  def unload(%__MODULE__{under: under}), do: unload(under)

  def unload(under) do
    for {module, _} <- :code.all_loaded(), under?(module, under) do
      :code.delete(module)
      :code.purge(module)
    end

    :ok
  end

  defp under?(module, under), do: String.starts_with?(Atom.to_string(module), Atom.to_string(under) <> ".")

  # The files go to a folder of their own to be compiled together, so they can use each other's structs and macros
  # in any order. The aliases share each file's first line, which keeps its line numbers.
  defp compile(under, sources) do
    folder = Path.join(System.tmp_dir!(), "one-core-" <> List.last(Module.split(under)))
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

  @doc "A version's service named `name`: its module, or `nil` if it has none by that name."
  def service(%__MODULE__{under: under, services: services}, name) do
    if Enum.any?(services, &(&1.name == name)), do: Module.concat(under, name)
  end

  @doc "The name of a version's module as its plugin wrote it: what follows the name the version's modules sit under."
  def own_name(module), do: module |> Module.split() |> Enum.drop(3) |> Enum.join(".")

  @doc "`text` with a version's module names as its plugin wrote them."
  def own_names(text, %__MODULE__{under: under}), do: own_names(text, under)
  def own_names(text, under), do: String.replace(text, inspect(under) <> ".", "")

  # A plugin with no Elixir has only parts that run in a client: it provides no tools and names no apps.
  defp describe(under, modules, client) do
    case Enum.filter(modules, &(OneCore.Plugin in behaviours(&1))) do
      [] when modules == [] ->
        {:ok, version(under, nil, [], nil, [], [], client)}

      [plugin] ->
        tools = if function_exported?(plugin, :tools, 0), do: plugin.tools(), else: []
        guide = if function_exported?(plugin, :guide, 0), do: plugin.guide()
        apps = if function_exported?(plugin, :apps, 0), do: plugin.apps(), else: []
        uses = if function_exported?(plugin, :uses, 0), do: plugin.uses(), else: []
        services = Enum.filter(modules, &(OneCore.Service in behaviours(&1)))

        cond do
          not (is_list(tools) and Enum.all?(tools, &tool?/1) and (is_nil(guide) or is_binary(guide))) ->
            {:error, "tools/0 gives a list of %{name: ..., description: ..., inputs: ...}, and guide/0 gives text"}

          not (is_list(apps) and Enum.all?(apps, &app?(&1, client))) ->
            {:error, "apps/0 gives a list of %{name: ..., page: ..., icon: ...}, where page and icon are files here"}

          not (is_list(uses) and Enum.all?(uses, &is_binary/1)) ->
            {:error, "uses/0 gives a list of plugins' names"}

          not Enum.all?(services, &(GenServer in behaviours(&1) and is_binary(&1.needs()))) ->
            {:error, "A service says `use GenServer`, and its needs/0 gives what it needs of a machine, as text"}

          true ->
            services = for module <- services, do: %{name: own_name(module), needs: module.needs()}
            {:ok, %{version(under, plugin, tools, guide, apps, Enum.sort(services), client) | uses: uses}}
        end

      found ->
        {:error, "One module of a plugin says `@behaviour OneCore.Plugin`, and #{length(found)} do"}
    end
  rescue
    error -> {:error, "tools/0, guide/0, apps/0, uses/0 or a service's needs/0 raised: " <> Exception.message(error)}
  end

  # A module has one `behaviour` attribute for each behaviour it says it follows.
  defp behaviours(module), do: List.flatten(Keyword.get_values(module.module_info(:attributes), :behaviour))

  # An app's icon goes to clients with its name, as a `data:` address, so a home screen asks for nothing more.
  defp version(under, plugin, tools, guide, apps, services, client) do
    apps =
      for %{name: name, page: page, icon: icon} <- apps do
        %{name: name, page: page, icon: "data:#{kind(icon)};base64,#{Base.encode64(client[icon])}"}
      end

    %__MODULE__{
      under: under,
      plugin: plugin,
      tools: tools,
      guide: guide,
      apps: apps,
      uses: [],
      services: services,
      code: nil,
      files: Enum.sort(Map.keys(client)),
      hashes: Map.new(client, fn {path, text} -> {path, :crypto.hash(:sha256, text)} end)
    }
  end

  @doc "What kind of file `path` is, as a web view is told it."
  def kind(path) do
    case Path.extname(path) do
      # What MCP Apps calls a page of an app.
      ".html" -> "text/html;profile=mcp-app"
      ".js" -> "text/javascript"
      ".css" -> "text/css"
      ".svg" -> "image/svg+xml"
      ".json" -> "application/json"
      _ -> "text/plain"
    end
  end

  defp app?(%{name: name, page: page, icon: icon}, client) when is_binary(name),
    do: Path.extname(page) == ".html" and is_map_key(client, page) and is_map_key(client, icon)

  defp app?(_, _), do: false

  defp tool?(%{name: name, description: description, inputs: %{}}) when is_binary(name) and is_binary(description),
    do: name =~ ~r/^[a-z][a-z0-9_]*$/

  defp tool?(_), do: false
end
