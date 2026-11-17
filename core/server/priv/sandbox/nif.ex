defmodule OneCore.Nif do
  @moduledoc """
  A NIF: native code a server runs, in Rust. One Core has its own (`core/server/nif/`), and an app brings its own
  (`server/nif/`, `OneCore.App`).

  The VM loads a NIF's library into a module once, and Rustler has no `upgrade` to load another build of it beside
  the first. So each build is a module of its own, `OneCore.Nifs.B<build>`, named for what it's built from
  (`tooling/input-hash`), and a changed NIF is a new module, loaded beside the one before, as an app's new version
  is (`OneCore.Apps.Version`):

      built, by its input hash (tooling/nifs) ─► loaded, beside any other build ─► unloaded, once nothing that loaded it does
      loading ─fails─► what brought it doesn't load: an app's version doesn't deploy, and the one before runs on

  Its Elixir says `use OneCore.Nif`, with each function `:erlang.nif_error(:nif_not_loaded)` as Rustler has it, and
  is compiled under the build's name as the build loads (`load/4`). Under its own name it hands each call to the
  newest build loaded, which is how One Core's code and other apps reach it. An app's own code reaches the build
  its version brought, so what it opened with a build it uses with that build.
  """

  @prefix "Elixir.OneCore.Nifs.B"

  defmacro __using__(options) do
    if String.starts_with?(Atom.to_string(__CALLER__.module), @prefix) do
      quote do
        @on_load :__load__
        @doc false
        def __load__, do: OneCore.Nif.loaded(__MODULE__, :erlang.load_nif(OneCore.Nif.file(__MODULE__), 0))
      end
    else
      # One Core's own are loaded the first time they're called, from where its release keeps them, as this source.
      library = options[:library]
      core = if library, do: quote(do: @external_resource(unquote(__CALLER__.file)))
      nif = if library, do: Macro.escape({library, File.read!(__CALLER__.file)})

      quote do
        unquote(core)
        @before_compile OneCore.Nif
        @doc false
        def __nif__, do: unquote(nif)
      end
    end
  end

  defmacro __before_compile__(env) do
    for {name, arity} <- Module.definitions_in(env.module, :def), name != :__nif__ do
      arguments = Macro.generate_arguments(arity, env.module)

      quote do
        defoverridable [{unquote(name), unquote(arity)}]

        def unquote(name)(unquote_splicing(arguments)),
          do: OneCore.Nif.build(__MODULE__).unquote(name)(unquote_splicing(arguments))
      end
    end
  end

  @doc "The newest build loaded of the NIF whose module is `name`. One Core's own is loaded the first time it's asked."
  def build(name) do
    case :persistent_term.get({__MODULE__, name}, []) do
      [build | _] ->
        build

      [] ->
        case name.__nif__() do
          {library, source} ->
            folder = Path.join(:code.priv_dir(:one_core), "native")
            library = File.read!(Path.join(folder, library <> extension()))

            case load(source, library, :one_core, folder) do
              {:ok, _name, build} -> build
              {:error, why} -> raise "#{inspect(name)} didn't load: #{why}"
            end

          nil ->
            raise "#{inspect(name)} isn't loaded here: no app running here brings it"
        end
    end
  end

  @doc """
  Loads the build of a NIF in `library`, its library for this machine, with its Elixir `source`, for `user`, writing
  the library in `folder` if it isn't there. Gives `{:ok, name, build}`, where `name` is its module's name in
  `source`, or `{:error, why}`. A build that's loaded already is the one loaded.
  """
  def load(source, library, user, folder) do
    with {:ok, build} <- named(library),
         {:ok, name, quoted} <- module(source) do
      :global.trans({__MODULE__, self()}, fn -> load(build, name, quoted, library, user, folder) end, [node()])
    end
  end

  defp load(build, name, quoted, library, user, folder) do
    file = Path.join(folder, build |> Module.split() |> List.last())

    with :ok <- own(name, user),
         :ok <- compiled(build, quoted, library, file),
         :ok <- forwards(name, quoted, build) do
      users = :persistent_term.get({__MODULE__, :users}, %{})

      users =
        Map.update(users, build, {name, MapSet.new([user])}, fn {name, users} -> {name, MapSet.put(users, user)} end)

      :persistent_term.put({__MODULE__, :users}, users)

      :persistent_term.put({__MODULE__, name}, [
        build | List.delete(:persistent_term.get({__MODULE__, name}, []), build)
      ])

      {:ok, name, build}
    end
  end

  @doc "Lets go of the builds `user` loaded. One that nothing else loaded is unloaded."
  def release(user) do
    held? = fn ->
      Enum.any?(:persistent_term.get({__MODULE__, :users}, %{}), fn {_, {_, users}} -> user in users end)
    end

    if held?.() do
      :global.trans({__MODULE__, self()}, fn -> unload(user) end, [node()])
    end

    :ok
  end

  defp unload(user) do
    {gone, kept} =
      :persistent_term.get({__MODULE__, :users}, %{})
      |> Map.new(fn {build, {name, users}} -> {build, {name, MapSet.delete(users, user)}} end)
      |> Map.split_with(fn {_build, {_name, users}} -> Enum.empty?(users) end)

    :persistent_term.put({__MODULE__, :users}, kept)

    for {build, {name, _users}} <- gone do
      :persistent_term.put({__MODULE__, name}, List.delete(:persistent_term.get({__MODULE__, name}, []), build))
      :persistent_term.erase({__MODULE__, :file, build})
      :code.delete(build)
      :code.purge(build)
    end
  end

  @doc false
  # Where `load/4` wrote a build's library, which its module loads as it's loaded: the VM takes it only from there.
  def file(build), do: String.to_charlist(:persistent_term.get({__MODULE__, :file, build}))

  @doc false
  # What loading a build's library said, keeping why it didn't load if it didn't.
  def loaded(build, loaded) do
    with {:error, {_, why}} <- loaded, do: :persistent_term.put({__MODULE__, :why, build}, to_string(why))
    loaded
  end

  # The build a library is: the module Rustler was told to name it, which it keeps as text.
  defp named(library) do
    with {at, length} <- :binary.match(library, @prefix),
         <<_::binary-size(^at + ^length), hash::binary-size(64), 0, _::binary>> <- library,
         true <- hash =~ ~r/^[0-9a-f]{64}$/ do
      {:ok, String.to_atom(@prefix <> hash)}
    else
      _ -> {:error, "its library isn't named for its build, as tooling/nifs builds it"}
    end
  end

  defp module(source) do
    case Code.string_to_quoted(source) do
      {:ok, {:defmodule, _, [{:__aliases__, _, parts}, _]} = quoted} -> {:ok, Module.concat(parts), quoted}
      _ -> {:error, "its Elixir isn't one module, which says `use OneCore.Nif`"}
    end
  end

  # One Core's NIFs are its own, and an app's can't take the name of anything else the server has.
  defp own(name, user) do
    case :code.ensure_loaded(name) do
      {:error, _} -> :ok
      {:module, _} when user == :one_core -> :ok
      {:module, _} -> if function_exported?(name, :__nif__, 0) and name.__nif__() == nil, do: :ok, else: owned(name)
    end
  end

  defp owned(name), do: {:error, "#{inspect(name)} is one of the server's own modules"}

  defp compiled(build, quoted, library, file) do
    if :code.is_loaded(build) do
      :ok
    else
      # Renamed into place, as another VM on this machine may have the same library loaded from it.
      if not File.exists?(file <> extension()) do
        File.mkdir_p!(Path.dirname(file))
        File.write!(file <> ".part", library)
        File.rename!(file <> ".part", file <> extension())
      end

      :persistent_term.put({__MODULE__, :file, build}, file)
      {:defmodule, meta, [_name, body]} = quoted
      compile = fn -> Code.compile_quoted({:defmodule, meta, [build, body]}) end
      # What doesn't compile says why in its diagnostics, and what doesn't load in what loading it said.
      {_, diagnostics} = Code.with_diagnostics(fn -> try(do: compile.(), rescue: (_ in CompileError -> nil)) end)
      why = :persistent_term.get({__MODULE__, :why, build}, nil)
      :persistent_term.erase({__MODULE__, :why, build})

      if :code.is_loaded(build),
        do: :ok,
        else: {:error, why || Enum.map_join(diagnostics, "\n", & &1.message)}
    end
  end

  # An app's NIF under its own name, compiled from its source with each function handing on to the newest build,
  # once, and again if a build has other functions. One Core's is compiled with One Core.
  defp forwards(name, quoted, build) do
    functions = Enum.sort(build.__info__(:functions) -- [__load__: 0])

    case :code.is_loaded(name) && name.__nif__() do
      {_library, _source} ->
        :ok

      nil ->
        if Enum.sort(name.__info__(:functions) -- [__nif__: 0]) == functions, do: :ok, else: forwarder(name, quoted)

      false ->
        forwarder(name, quoted)
    end
  end

  defp forwarder(name, quoted) do
    :code.purge(name)
    :code.delete(name)
    :code.purge(name)
    Code.compile_quoted(quoted)
    :ok
  rescue
    error in CompileError -> {:error, Exception.message(error)}
  end

  # What the VM loads a library by, which it finds by the name without it: a DLL on Windows.
  defp extension, do: if(match?({:win32, _}, :os.type()), do: ".dll", else: ".so")
end
