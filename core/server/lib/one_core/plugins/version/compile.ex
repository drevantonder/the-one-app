defmodule OneCore.Plugins.Version.Compile do
  @moduledoc """
  Compiles a version's Elixir under the name `OneCore.Plugins.LoadedVersion` gives it, as it's loading: each of its
  modules sits under that name (`OneCore.Plugins.Load7.Notes`), by an alias put before each file, and what its plugin
  says of itself is checked (`OneCore.Plugins.Version.Checks`).
  """
  alias OneCore.{App, Builds, Nif, Server, Skills}
  alias OneCore.Plugins.Version
  alias OneCore.Plugins.Version.Checks

  @doc """
  Compiles the `files` of the plugin `artifact` under the name `under`, as `OneCore.Plugins.LoadedVersion.load/3`
  calls it: `{:ok, version, warnings, kept}`, where `warnings` are the compiler's, or `{:error, why}`.
  """
  def compile(artifact, files, under) do
    name = Path.basename(artifact)
    {sources, rest} = Enum.split_with(files, fn {path, _} -> Version.server?(path) and Path.extname(path) == ".ex" end)
    {nifs, sources} = Enum.split_with(sources, fn {path, _} -> Path.dirname(path) == "server/nif" end)

    {skills, rest} =
      Map.split_with(Map.new(rest), fn {path, _} -> path == "SKILL.md" or String.starts_with?(path, "skills/") end)

    with {:ok, skills} <- Skills.found(name, skills),
         {:ok, builds} <- nifs(under, nifs, rest),
         {:ok, modules, warnings} <- compiled(under, sources, builds),
         {:ok, version} <- described(under, modules, rest) do
      kept = %{
        files: Enum.sort(for path <- Map.keys(rest), Version.loaded?(path), do: path),
        hashes: Map.new(rest, fn {path, text} -> {path, :crypto.hash(:sha256, text)} end),
        skills: skills,
        own: Map.reject(rest, fn {path, _} -> Version.client?(path) end)
      }

      {:ok, %{version | code: Version.code(files), ui: App.ui(files), title: version.title || name}, warnings, kept}
    end
  end

  # A NIF, `server/nif/<name>.ex` and its library as it's built for this kind of machine, `server/nif/<name>/<kind>.so`,
  # loaded as its build (`OneCore.Nif`), which this version's code reaches by its name. Gives each build by that name,
  # or what's wrong.
  defp nifs(under, nifs, rest) do
    Enum.reduce_while(nifs, {:ok, %{}}, fn {path, source}, {:ok, builds} ->
      library = Path.rootname(path) <> "/#{Builds.target()}.so"

      with {:built, true} <- {:built, is_map_key(rest, library)},
           {:ok, name, build} <- Nif.load(source, rest[library], under, Path.join(Server.home(), "nif")),
           [_one_word] <- Module.split(name) do
        {:cont, {:ok, Map.put(builds, inspect(name), build)}}
      else
        {:built, false} -> {:halt, {:error, "#{path} has no build for this kind of machine, #{library}"}}
        {:error, why} -> {:halt, {:error, "#{path} didn't load on this machine, #{Builds.target()}: #{why}"}}
        _ -> {:halt, {:error, "#{path}'s module is named in one word, which this plugin's code reaches it by"}}
      end
    end)
  end

  # The files go to a folder of their own to be compiled together, so they can use each other's structs and macros
  # in any order. The aliases share each file's first line, which keeps its line numbers. A loading that was killed
  # left its folder, which the next under its name clears.
  defp compiled(under, sources, builds) do
    folder = Path.join([Server.home(), "compiling", List.last(Module.split(under))])
    File.rm_rf!(folder)
    names = Enum.uniq(for {_, source} <- sources, name <- defined(source), do: name)
    names = Map.merge(builds, Map.new(names, &{Atom.to_string(&1), Module.concat(under, &1)}))
    aliases = Enum.map_join(names, fn {name, module} -> "alias #{inspect(module)}, as: #{name}, warn: false; " end)

    try do
      paths =
        for {path, source} <- sources do
          file = Path.join(folder, path)
          File.mkdir_p!(Path.dirname(file))
          File.write!(file, aliases <> source)
          file
        end

      case Kernel.ParallelCompiler.compile(paths, return_diagnostics: true) do
        {:ok, modules, warnings} ->
          case Enum.reject(modules, &(Version.under(&1) == under)) do
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
      Version.own_names("#{file}:#{line}: #{String.trim(message)}", under) |> String.replace(folder <> "/", "")
    end)
  end

  # What a plugin says of itself, by the functions that say it, with what each gives where it has none.
  @callbacks [
    tools: [],
    apps: [],
    title: nil,
    icon: nil,
    widgets: [],
    hooks: [],
    offers: [],
    jobs: [],
    requires: %{}
  ]

  # `rest` is the files that aren't Elixir or skills. A plugin with no Elixir provides no tools and names no apps.
  defp described(under, modules, rest) do
    case Enum.filter(modules, &(OneCore.Plugin in Version.behaviours(&1))) do
      [] when modules == [] ->
        Checks.checked(under, nil, Map.put(Map.new(@callbacks), :exports, []), [], rest)

      [plugin] ->
        description =
          for {function, none} <- @callbacks,
              into: %{exports: exports(plugin)},
              do: {function, if(function_exported?(plugin, function, 0), do: apply(plugin, function, []), else: none)}

        services = Enum.filter(modules, &(OneCore.Service in Version.behaviours(&1)))

        process? = &(GenServer in Version.behaviours(&1) or :gen_statem in Version.behaviours(&1))

        if Enum.all?(services, &(process?.(&1) and is_binary(&1.needs()) and is_binary(&1.called()))) do
          services =
            for module <- services,
                do: %{name: Version.own_name(module), called: module.called(), needs: module.needs()}

          Checks.checked(under, plugin, description, services, rest)
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
       "tools/0, apps/0, widgets/0, hooks/0, offers/0, jobs/0, requires/0 or a service's " <>
         "needs/0 or called/0 raised: " <>
         Exception.message(error)}
  end

  # The functions a plugin's module provides other plugins, which run inside each plugin that requires it
  # (`OneCore.Plugin`): its public ones besides the callbacks, as name/arity.
  defp exports(plugin) do
    callbacks = OneCore.Plugin.behaviour_info(:callbacks)

    Enum.sort(
      for {name, arity} <- plugin.__info__(:functions),
          {name, arity} not in callbacks,
          not String.starts_with?(Atom.to_string(name), "__"),
          do: "#{name}/#{arity}"
    )
  end
end
