defmodule OneCore.Plugins.Version do
  @moduledoc """
  One version of a plugin's code, compiled and loaded.

  The VM keeps only two versions of a module, so each version loaded gets module names of its own: its modules
  sit under a name no other version has (`OneCore.Plugins.Load7.Tracker`), by an alias put before each file. A
  version is unloaded by removing every module under that name.
  """

  @enforce_keys [:under, :plugin, :tools, :guide]
  defstruct [:under, :plugin, :tools, :guide]

  @doc "Compiles and loads a plugin's files. Gives the version and the compiler's warnings, or what's wrong."
  def load(files) do
    under = Module.concat(OneCore.Plugins, "Load#{System.unique_integer([:positive])}")
    sources = for {path, source} <- files, String.ends_with?(path, ".ex"), do: {path, source}

    with {:ok, modules, warnings} <- compile(under, sources),
         {:ok, version} <- describe(under, modules) do
      {:ok, version, warnings}
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

  @doc "`text` with a version's module names as its plugin wrote them."
  def own_names(text, %__MODULE__{under: under}), do: own_names(text, under)
  def own_names(text, under), do: String.replace(text, inspect(under) <> ".", "")

  defp describe(under, modules) do
    case Enum.filter(modules, &(OneCore.Plugin in Keyword.get(&1.module_info(:attributes), :behaviour, []))) do
      [plugin] ->
        tools = plugin.tools()
        guide = if function_exported?(plugin, :guide, 0), do: plugin.guide()

        if is_list(tools) and Enum.all?(tools, &tool?/1) and (is_nil(guide) or is_binary(guide)) do
          {:ok, %__MODULE__{under: under, plugin: plugin, tools: tools, guide: guide}}
        else
          {:error, "tools/0 gives a list of %{name: ..., description: ..., inputs: ...}, and guide/0 gives text"}
        end

      found ->
        {:error, "One module of a plugin says `@behaviour OneCore.Plugin`, and #{length(found)} do"}
    end
  rescue
    error -> {:error, "tools/0 or guide/0 raised: " <> Exception.message(error)}
  end

  defp tool?(%{name: name, description: description, inputs: %{}}) when is_binary(name) and is_binary(description),
    do: name =~ ~r/^[a-z][a-z0-9_]*$/

  defp tool?(_), do: false
end
