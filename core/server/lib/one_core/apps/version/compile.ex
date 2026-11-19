defmodule OneCore.Apps.Version.Compile do
  @moduledoc """
  Compiles a version's Elixir under the name `OneCore.Apps.LoadedVersion` gives it, as it's loading: each of its
  modules sits under that name (`OneCore.Apps.V<hash>.Notes`), by an alias put before each file, and what its app
  says of itself is checked (`OneCore.Apps.Version.Checks`).

  What it's compiled to is kept on this machine, as `.beam` files, and loaded instead of compiling it again for as
  long as nothing its compile read has changed: Elixir and OTP, and each module outside it that it used as it
  compiled, as Mix tracks and compares them (`Kernel.LexicalTracker`): one whose macros it ran, or that it `use`d or
  `require`d, by its MD5, and one whose struct it built, by its exports' MD5. So a server that restarts compiles
  again only the apps whose code, or what they're compiled with, changed.
  """
  alias OneCore.{Entrypoint, Builds, Nif, Server, Skills}
  alias OneCore.App.Manifest
  alias OneCore.Apps.Version
  alias OneCore.Apps.Version.Checks
  require Logger

  @doc """
  Compiles the `files` of the app `artifact` under the name `under`, as `OneCore.Apps.LoadedVersion.load/3`
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
         {:ok, version} <- described(under, modules, rest, skills) do
      kept = %{
        files: Enum.sort(for path <- Map.keys(rest), Version.loaded?(path), do: path),
        hashes: Map.new(rest, fn {path, text} -> {path, :crypto.hash(:sha256, text)} end),
        skills: skills,
        own: Map.reject(rest, fn {path, _} -> Version.client?(path) end)
      }

      {:ok, %{version | code: Version.code(files), ui: Entrypoint.ui(files), }, warnings,
       kept}
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
        _ -> {:halt, {:error, "#{path}'s module is named in one word, which this app's code reaches it by"}}
      end
    end)
  end

  # What a version was compiled to, kept by its name and the Elixir and OTP it was compiled with, or else its files
  # compiled together.
  defp compiled(under, sources, builds) do
    cache =
      Path.join([Server.home(), "compiled", Version.code({under, System.version(), :erlang.system_info(:otp_release)})])

    with :error <- kept(cache) do
      with {:ok, modules, warnings, binaries, references} <- compile_files(under, sources, builds) do
        keep(cache, binaries, references, warnings)
        {:ok, modules, warnings}
      end
    end
  end

  # What's kept is loaded at once (`:code.atomic_load`), if each module it was compiled with is as it was then.
  defp kept(cache) do
    with {:ok, kept} <- File.read(Path.join(cache, "compiled")),
         %{modules: modules, references: references, warnings: warnings} <- :erlang.binary_to_term(kept),
         true <- Enum.all?(references, fn {module, {kind, md5}} -> md5(module, kind) == md5 end),
         :ok <- :code.atomic_load(for module <- modules, do: beam(cache, module)) do
      File.touch(cache)
      {:ok, modules, warnings}
    else
      _ -> :error
    end
  end

  defp beam(cache, module) do
    file = Path.join(cache, "#{module}.beam")
    {module, String.to_charlist(file), File.read!(file)}
  end

  defp md5(module, kind) do
    cond do
      not Code.ensure_loaded?(module) -> nil
      kind == :export and function_exported?(module, :__info__, 1) -> module.__info__(:exports_md5)
      true -> module.module_info(:md5)
    end
  end

  # Written beside, then renamed into place, so what's kept is all there or not at all. What wasn't loaded for a month
  # goes, a version's files kept here among it too (`OneCore.Apps.Store.files/3`): its version, or the Elixir it was
  # compiled with, is long gone.
  defp keep(cache, binaries, references, warnings) do
    writing = cache <> ".writing"
    File.rm_rf!(writing)
    File.mkdir_p!(writing)
    for {module, binary} <- binaries, do: File.write!(Path.join(writing, "#{module}.beam"), binary)

    kept = %{
      modules: Map.keys(binaries),
      references: Map.new(references, fn {module, kind} -> {module, {kind, md5(module, kind)}} end),
      warnings: warnings
    }

    File.write!(Path.join(writing, "compiled"), :erlang.term_to_binary(kept))
    File.rm_rf!(cache)
    File.rename!(writing, cache)
    month_ago = System.os_time(:second) - 30 * 24 * 3600

    for folder <- Path.wildcard(Path.join(Path.dirname(cache), "*")),
        {:ok, %{mtime: used}} <- [File.stat(folder, time: :posix)],
        used < month_ago,
        do: File.rm_rf!(folder)
  rescue
    error -> Logger.warning("What an app was compiled to wasn't kept: #{Exception.message(error)}")
  end

  # The files go to a folder of their own to be compiled together, so they can use each other's structs and macros
  # in any order. The aliases share each file's first line, which keeps its line numbers. A loading that was killed
  # left its folder, which the next under its name clears. The compiler hands each module's binary, and each file's
  # references, to the callbacks here, in this process, as it does Mix's.
  defp compile_files(under, sources, builds) do
    folder = Path.join([Server.home(), "compiling", List.last(Module.split(under))])
    File.rm_rf!(folder)
    names = Enum.uniq(for {_, source} <- sources, name <- defined(source), do: name)
    names = Map.merge(builds, Map.new(names, &{Atom.to_string(&1), Module.concat(under, &1)}))
    aliases = Enum.map_join(names, fn {name, module} -> "alias #{inspect(module)}, as: #{name}, warn: false; " end)
    me = self()
    ref = make_ref()

    options = [
      return_diagnostics: true,
      each_file: fn _file, lexical -> send(me, {ref, :references, Kernel.LexicalTracker.references(lexical)}) end,
      each_module: fn _file, module, binary -> send(me, {ref, :module, module, binary}) end
    ]

    try do
      paths =
        for {path, source} <- sources do
          file = Path.join(folder, path)
          File.mkdir_p!(Path.dirname(file))
          File.write!(file, aliases <> source)
          file
        end

      outcome = Kernel.ParallelCompiler.compile(paths, options)
      {binaries, references} = received(ref, %{}, %{})

      case outcome do
        {:ok, modules, warnings} ->
          case Enum.reject(modules, &(Version.under(&1) == under)) do
            [] ->
              own = Atom.to_string(under) <> "."

              references =
                Map.reject(references, fn {module, _kind} -> String.starts_with?(Atom.to_string(module), own) end)

              {:ok, modules, messages(warnings.compile_warnings ++ warnings.runtime_warnings, under, folder), binaries,
               references}

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

  # Each module's binary, and the modules each file used as it compiled, by how: `:compile` for one whose code ran
  # as it compiled, `:export` for one whose struct or exports it read (Mix's compile and export references).
  defp received(ref, binaries, references) do
    receive do
      {^ref, :module, module, binary} ->
        received(ref, Map.put(binaries, module, binary), references)

      {^ref, :references, {compile, exports, _runtime, _env}} ->
        references = Enum.reduce(exports, references, &Map.put_new(&2, &1, :export))
        received(ref, binaries, Enum.reduce(compile, references, &Map.put(&2, &1, :compile)))
    after
      0 -> {binaries, references}
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

  # `rest` is the files that aren't Elixir or skills. An app with no Elixir provides no tools and names no entrypoints.
  defp described(under, modules, rest, skills) do
    case Enum.filter(modules, &(OneCore.App in Version.behaviours(&1))) do
      [] when modules == [] ->
        Checks.checked(under, nil, Manifest.decode(%{}), [], rest, skills)

      [app] ->
        services = Enum.filter(modules, &(OneCore.Service in Version.behaviours(&1)))
        process? = &(GenServer in Version.behaviours(&1) or :gen_statem in Version.behaviours(&1))

        with {:ok, description, services} <- said(app, services) do
          if Enum.all?(services, fn {module, called, needs, os} ->
               process?.(module) and is_binary(needs) and is_binary(called) and
                 (is_nil(os) or (is_list(os) and Enum.all?(os, &is_binary/1)))
             end) do
            services =
              for {module, called, needs, os} <- services,
                  do: %{name: Version.own_name(module), called: called, needs: needs, os: os}

            Checks.checked(under, app, description, services, rest, skills)
          else
            {:error,
             "A service says `use GenServer` or `@behaviour :gen_statem`, its needs/0 gives what it needs of a " <>
               "machine, and its called/0 what it's called, each as text, and its os/0, if it has one, the systems it " <>
               "runs on, as a list of text"}
          end
        end

      found ->
        {:error, "One module of an app says `@behaviour OneCore.App`, and #{length(found)} do"}
    end
  end

  # What the app and its services say of themselves, by their own functions, which may raise.
  defp said(app, services) do
    {:ok, Manifest.said(app), for(module <- services, do: {module, module.called(), module.needs(), if(function_exported?(module, :os, 0), do: module.os())})}
  rescue
    error ->
      functions = Enum.map_join(Manifest.said(), ", ", &"#{&1}/0")
      {:error, "#{functions} or a service's needs/0 or called/0 raised: " <> Exception.message(error)}
  end
end
