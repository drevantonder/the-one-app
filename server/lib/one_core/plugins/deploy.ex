defmodule OneCore.Plugins.Deploy do
  @moduledoc """
  Deploying a plugin: a new one, or a new version of one. Its app, if it has one, is built first (`OneCore.App`),
  its Elixir is compiled and loaded (`OneCore.Plugins.Version`), and it's kept in the bucket
  (`OneCore.Plugins.Kept`) and runs at once.

  Deploying or adding a plugin brings along the ones it uses that the server ships with: its `:shipped` setting,
  each one's folder by its name. Those are added for the network.
  """
  require Logger
  alias OneCore.{App, Server, Servers}
  alias OneCore.Plugins
  alias OneCore.Plugins.{Added, Calls, Kept, Version}

  @network "network"

  @doc """
  Deploys a plugin for `owner` from its files by their paths: the plugin that `name` is for them is updated, and
  if there's none it's a new plugin of theirs, added for them. Its app, if it has one, is built first
  (`OneCore.App`). Gives its version's number and the compiler's warnings.
  """
  def deploy(owner, name, files) do
    artifact = Plugins.plugin(owner, name) || "#{owner}/plugins/#{name}"

    with :ok <- check(name, files),
         {:ok, kept} <- built(owner, name, files),
         {:ok, version, warnings} <- Version.load(Path.basename(artifact), kept) do
      version = %{version | artifact: artifact}

      with :ok <- fits(owner, name, version),
           {:ok, number} <- Kept.keep(artifact, name, kept, files),
           :ok <- Added.note(owner, &{:ok, Map.put(&1, name, artifact)}) do
        Plugins.look(owner, [{artifact, number, {:ok, version}}])
        Server.tell_servers(Enum.uniq([owner | Added.added_for(artifact)]))
        {:ok, number, warnings}
      else
        {:error, why} ->
          Version.unload(version)
          {:error, why}
      end
    end
  end

  @doc """
  Deploys for the network each plugin this server ships with that the network has never had. One the network has
  is left as it is: deploying it again is what updates it. Gives what `deploy/3` gave for each, by its name.
  """
  def ship do
    for {name, folder} <- Enum.sort(Application.get_env(:one_core, :shipped, %{})),
        Kept.newest("#{@network}/plugins/#{name}") == nil do
      case shipped(name) do
        files when files == %{} -> {name, {:error, "There are no files in #{folder}"}}
        files -> {name, deploy(@network, name, files)}
      end
    end
  end

  @doc """
  Whether a plugin fits whoever it's added for: the plugins it uses are there for them, and one with scheduled jobs
  uses a plugin that keeps them. `:ok`, or `{:error, why}`.
  """
  def fits(owner, name, version) do
    with :ok <- bring(owner, name, version.uses), do: scheduled(owner, name, version)
  end

  # A plugin's scheduled jobs are kept by a plugin it uses.
  defp scheduled(_owner, _name, %Version{jobs: []}), do: :ok

  defp scheduled(owner, name, version) do
    with {:ok, used} <- Plugins.used(owner, version.uses),
         [_ | _] <- Calls.keepers(used) do
      :ok
    else
      _ -> {:error, "#{name} has scheduled jobs, so it uses a plugin that keeps its schedule, as database does"}
    end
  end

  # A plugin's app is built against the kit that runs for whoever it's deployed for, which is brought along as a
  # plugin it uses is.
  defp built(owner, name, files) do
    if App.app?(files) do
      with :ok <- bring(owner, name, ["ui"]), do: App.build(files, Plugins.running(owner)["ui"])
    else
      {:ok, files}
    end
  end

  @doc """
  Builds again, from its source, each app built against a kit that has changed since, as a server acting for
  whoever the plugin is, as they would deploy it again.
  """
  def rebuild do
    for {owner, added} <- Plugins.added(),
        {name, artifact} <- added,
        %Version{kit: built_with} = version <- [Plugins.version(artifact)],
        built_with != nil,
        kit = Plugins.running(owner)["ui"],
        kit != nil and kit.code != built_with,
        whose = Kept.whose(artifact),
        Servers.here?(whose) and Plugins.plugin(whose, Path.basename(artifact)) == artifact,
        uniq: true do
      with {:error, why} <- deploy(whose, Path.basename(artifact), Kept.source(artifact, version.code)),
           do: Logger.error("#{name}'s app wasn't built again against the kit that changed: #{why}")
    end
  end

  # The plugins a plugin uses are added before it is: the ones missing come from what the server ships with, and
  # are added for the network, by a server acting for it.
  defp bring(owner, name, uses) do
    shipped = &(Servers.here?(@network) and match?({:ok, _, _}, deploy(@network, &1, shipped(&1))))

    case Enum.reject(uses, &(is_map_key(Plugins.running(owner), &1) or shipped.(&1))) do
      [] ->
        :ok

      [missing] ->
        {:error,
         "#{name} uses #{missing}, which isn't added for #{owner} or the network. Add it for them first, or deploy it."}

      missing ->
        {:error,
         "#{name} uses #{Enum.join(missing, ", ")}, which aren't added for #{owner} or the network. " <>
           "Add them for them first, or deploy them."}
    end
  end

  # The files of a plugin this server ships with, or none if it ships with no such plugin.
  defp shipped(name) do
    with %{^name => folder} <- Application.get_env(:one_core, :shipped, %{}) do
      files = Enum.filter(Path.wildcard(Path.join(folder, "**")), &File.regular?/1)
      Map.new(files, &{Path.relative_to(&1, folder), File.read!(&1)})
    else
      _ -> %{}
    end
  end

  defp check(name, files) do
    cond do
      Added.named(name) != :ok ->
        Added.named(name)

      not (is_map(files) and map_size(files) > 0 and
               Enum.all?(files, fn {path, text} -> path?(path) and is_binary(text) end)) ->
        {:error, "A plugin's files are text by their paths, with no path part starting with a dot"}

      true ->
        :ok
    end
  end

  defp path?(path), do: is_binary(path) and Enum.all?(String.split(path, "/"), &(&1 =~ ~r/^[^.]/))
end
