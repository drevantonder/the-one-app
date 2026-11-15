defmodule OneCore.Plugins.Deploy do
  @moduledoc """
  Deploying a plugin: a new one, or a new version of one. Its app, if it has one, is built first (`OneCore.App`),
  its Elixir is compiled and loaded (`OneCore.Plugins.Version`), and it's kept in the bucket
  (`OneCore.Plugins.Kept`) and runs at once.

  Deploying or adding a plugin brings along the ones it uses that the server ships with: its `:shipped` setting,
  each one's folder by its name. Those are added for the network.
  """
  require Logger
  alias OneCore.{App, Bucket, Folder, Server, Servers}
  alias OneCore.Plugins
  alias OneCore.Plugins.{Added, Calls, Kept, Version}

  @network "network"
  # What each owner has deployed, as the network keeps it: `deployed/<owner>/<name>`.
  @deployed "network/deployed/"

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
        listed(owner, name, version)
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
  Every plugin anyone has deployed, with its apps as its newest version has them: `[%{plugin: artifact, apps:
  [...]}]`. It's what lists every app there is, whether or not it's on anyone's page now.
  """
  def deployed do
    blobs = Bucket.list(@deployed)

    for {blob, {:ok, said, _tag}} <- Enum.zip(blobs, Bucket.read_all(blobs)) do
      [_, owner, name] = Regex.run(~r{^#{@deployed}(.+)/([^/]+)$}, blob)

      apps =
        for app <- JSON.decode!(said)["apps"],
            do: %{name: app["name"], page: app["page"], icon: app["icon"], settings: app["settings"]}

      %{plugin: "#{owner}/plugins/#{name}", apps: apps}
    end
  end

  # That `owner` deployed the plugin `name`, with the apps this version has, among what everyone has deployed.
  defp listed(owner, name, version) do
    blob = "#{@deployed}#{owner}/#{name}"
    said = JSON.encode!(%{apps: version.apps})

    case Bucket.read(blob) do
      {:ok, ^said, _tag} -> :ok
      {:ok, _was, tag} -> Bucket.replace(blob, said, tag)
      :absent -> Bucket.write(blob, said)
    end
  end

  @doc """
  Deploys for the network each plugin this server ships with that the network has never had, and each that runs
  what a server once shipped but not what this one ships: so an updated server brings its plugins along. One whose
  newest version came from files no server shipped was changed on purpose, and is left as it is. Gives what
  `deploy/3` gave for each, by its name, and says each in the log and in `shipping/0` as it goes.

  What servers have shipped of a plugin is the plugin's blob `shipped`: the hashes of every set of files they
  deployed it from.
  """
  def ship do
    names =
      for {name, _folder} <- Enum.sort(Application.get_env(:one_core, :shipped, %{})),
          ship?(name, Version.code(shipped(name))),
          do: name

    for name <- names, do: shipping(name, "waiting")
    for name <- names, ship?(name, Version.code(shipped(name))), do: {name, ship(name)}
  end

  @doc """
  What of the plugins this server ships it's still deploying, or failed to, by name: `"waiting"`, `"building"`, or
  why it failed. Empty once every one is deployed.
  """
  def shipping, do: Agent.get(__MODULE__, & &1)

  @doc false
  def child_spec(_), do: %{id: __MODULE__, start: {Agent, :start_link, [fn -> %{} end, [name: __MODULE__]]}}

  defp shipping(name, nil), do: Agent.update(__MODULE__, &Map.delete(&1, name))
  defp shipping(name, said), do: Agent.update(__MODULE__, &Map.put(&1, name, said))

  # Deploys one plugin this server ships for the network, and notes its files among what servers have shipped.
  defp ship(name) do
    files = shipped(name)
    source = Version.code(files)
    started = System.monotonic_time(:millisecond)
    Logger.info("Shipping #{name}")
    shipping(name, "building")

    with true <- files != %{} || {:error, "There are no files to ship it from"},
         {:ok, number, _warnings} = deployed <- deploy(@network, name, files),
         :ok <- note_shipped("#{@network}/plugins/#{name}/shipped", source) do
      Logger.info("Shipped #{name} as version #{number} in #{System.monotonic_time(:millisecond) - started} ms")
      shipping(name, nil)
      deployed
    else
      {:error, why} = failed ->
        Logger.error("#{name} wasn't shipped: #{why}")
        shipping(name, why)
        failed
    end
  end

  defp note_shipped(blob, source) do
    case Bucket.read(blob) do
      {:ok, said, tag} ->
        sources = shipped_sources(said)

        cond do
          source in sources -> :ok
          Bucket.replace(blob, JSON.encode!([source | sources]), tag) == :ok -> :ok
          true -> note_shipped(blob, source)
        end

      :absent ->
        with :exists <- Bucket.write(blob, JSON.encode!([source])), do: note_shipped(blob, source)
    end
  end

  # Whether the plugin `name` that this server ships, from files whose hash is `source`, is to be deployed for the
  # network: it never had it, or it runs what a server shipped once and that isn't this. One whose newest version
  # doesn't say what it was deployed from is from before versions said so, and was shipped too.
  defp ship?(name, source) do
    artifact = "#{@network}/plugins/#{name}"

    case Kept.newest_source(artifact) do
      nil ->
        true

      ^source ->
        false

      newest ->
        case Bucket.read(artifact <> "/shipped") do
          {:ok, said, _tag} -> newest in shipped_sources(said)
          :absent -> false
        end
    end
  end

  defp shipped_sources(said),
    do: with(sources when is_list(sources) <- JSON.decode!(said), do: sources, else: (_ -> []))

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
      _ -> {:error, "#{name} has scheduled jobs, so it uses a plugin that keeps its schedule: one with schedule/3"}
    end
  end

  # A plugin's app is built against the UI library that runs for whoever it's deployed for, which is brought along as a
  # plugin it uses is.
  defp built(owner, name, files) do
    if App.app?(files) do
      with :ok <- bring(owner, name, ["ui"]) do
        ui = Plugins.running(owner)["ui"]
        built_before(Plugins.plugin(owner, name), files, ui) || App.build(files, ui)
      end
    else
      {:ok, files}
    end
  end

  # The apps as the plugin's running version built them, where that was from the same source against the same UI library,
  # since building it again would make the same: `{:ok, files}`, or `nil`.
  defp built_before(nil, _files, _kit), do: nil

  defp built_before(artifact, files, ui) do
    client = &Map.filter(&1, fn {path, _} -> Version.client?(path) end)

    with %Version{ui: code, code: kept} when code == ui.code <- Plugins.version(artifact),
         true <- client.(Kept.source(artifact, kept)) == client.(files) do
      {:ok,
       Map.merge(
         Map.reject(files, fn {path, _} -> Version.client?(path) end),
         client.(Kept.files(artifact, kept))
       )}
    else
      _ -> nil
    end
  end

  @doc """
  Builds again, from its source, each app built against a UI library that has changed since, as a server acting for
  whoever the plugin is, as they would deploy it again.
  """
  def rebuild do
    for {owner, added} <- Plugins.added(),
        {name, artifact} <- added,
        %Version{ui: built_with} = version <- [Plugins.version(artifact)],
        built_with != nil,
        ui = Plugins.running(owner)["ui"],
        ui != nil and ui.code != built_with,
        whose = Kept.whose(artifact),
        Servers.here?(whose) and Plugins.plugin(whose, Path.basename(artifact)) == artifact,
        uniq: true do
      with {:error, why} <- deploy(whose, Path.basename(artifact), Kept.source(artifact, version.code)),
           do: Logger.error("#{name}'s app wasn't built again against the UI library that changed: #{why}")
    end
  end

  # The plugins a plugin uses are added before it is: the ones missing come from what the server ships with, and
  # are added for the network, by a server acting for it.
  defp bring(owner, name, uses) do
    shipped = &(Servers.here?(@network) and shipped(&1) != %{} and match?({:ok, _, _}, ship(&1)))

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

  # The files of a plugin this server ships with, as a deploy from its folder sends them, or none if it ships with
  # no such plugin.
  defp shipped(name) do
    with %{^name => folder} <- Application.get_env(:one_core, :shipped, %{}),
         {:ok, paths} <- Folder.files(folder) do
      Map.new(paths, &{&1, File.read!(Path.join(folder, &1))})
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
