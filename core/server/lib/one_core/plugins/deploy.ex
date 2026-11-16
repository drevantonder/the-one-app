defmodule OneCore.Plugins.Deploy do
  @moduledoc """
  Deploying a plugin: a new one, or a new version of one. Its app, if it has one, is built first (`OneCore.App`),
  its Elixir is compiled and loaded (`OneCore.Plugins.Version`), and it's kept in the bucket
  (`OneCore.Plugins.Store`) and runs at once.

  A server deploys each plugin it ships with for the network: its `:bundled` setting, each one's folder by its name.
  One that says it's essential (`OneCore.Plugin`) is installed for the network; the rest are only available, in the
  App Store for anyone to add, until someone does. Deploying or adding a plugin installs the ones it uses that the
  server ships with for whoever it's installed for, where they aren't there.
  """
  require Logger
  alias OneCore.{App, Bucket, Folder, Server, Servers}
  alias OneCore.Plugins
  alias OneCore.Plugins.{Installed, Calls, Store, Version}

  @network "network"
  # What each owner has deployed, as the network keeps it: `deployed/<owner>/<name>`.
  @deployed "network/deployed/"

  @doc """
  Deploys a plugin for `owner` from its files by their paths: the plugin that `name` is for them is updated, and
  if there's none it's a new plugin of theirs, installed for them. Its app, if it has one, is built first
  (`OneCore.App`). `from` is the git repo its files came from, if they did (`OneCore.Plugins.Store`). Gives its
  version's number and the compiler's warnings.
  """
  def deploy(owner, name, files, from \\ nil), do: deploy(owner, name, files, from, fn _version -> true end)

  # Deploys as `deploy/4` does, installing it for `owner` only where `install?` says so of the version.
  defp deploy(owner, name, files, from, install?) do
    artifact = Plugins.plugin(owner, name) || "#{owner}/plugins/#{name}"

    with :ok <- check(name, files),
         {:ok, kept} <- built(owner, name, files),
         {:ok, version, warnings} <- Version.load(Path.basename(artifact), kept) do
      version = %{version | artifact: artifact}
      install = install?.(version)

      with :ok <- if(install, do: fits(owner, name, version), else: :ok),
           {:ok, number} <- Store.keep(artifact, name, kept, files, from),
           :ok <- if(install, do: Installed.note(owner, &{:ok, Map.put(&1, name, artifact)}), else: :ok) do
        listed(owner, name, version)
        if install, do: Plugins.reload(owner, [{artifact, number, {:ok, version}}]), else: Version.unload(version)
        Server.notify_servers(Enum.uniq([owner | Installed.installed_for(artifact)]))
        {:ok, number, warnings}
      else
        {:error, why} ->
          Version.unload(version)
          {:error, why}
      end
    end
  end

  @doc """
  Every plugin anyone has deployed, with its apps as its latest version has them: `[%{plugin: artifact, apps:
  [...]}]`. It's what lists every app there is, whether or not it's on anyone's page now. One that was removed
  (Recently Deleted's `remove`) has no latest version, and isn't listed.
  """
  def deployed do
    blobs = Bucket.list(@deployed)
    named = for blob <- blobs, do: Regex.run(~r{^#{@deployed}(.+)/([^/]+)$}, blob, capture: :all_but_first)
    latest = for [owner, name] <- named, do: "#{owner}/plugins/#{name}/latest"

    for {[owner, name], {:ok, said, _tag}, {:ok, _latest, _}} <-
          Enum.zip([named, Bucket.read_all(blobs), Bucket.read_all(latest)]) do
      apps =
        for app <- JSON.decode!(said)["apps"],
            do: %{
              name: app["name"],
              page: app["page"],
              icon: app["icon"],
              settings: app["settings"],
              account: app["account"] == true
            }

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
  latest version came from files no server shipped was changed on purpose, and is left as it is. Gives what
  `deploy/3` gave for each, by its name, and says each in the log and in `deploying_bundled/0` as it goes.

  What servers have shipped of a plugin is the plugin's blob `bundled`: the hashes of every set of files they
  deployed it from.
  """
  def deploy_bundled do
    names =
      for {name, _folder} <- Enum.sort(Application.get_env(:one_core, :bundled, %{})),
          deploy_bundled?(name, Version.code(bundled(name))),
          do: name

    for name <- names, do: deploying(name, "waiting")
    for name <- names, deploy_bundled?(name, Version.code(bundled(name))), do: {name, deploy_bundled(name)}
  end

  @doc """
  Waits until this server has deployed the plugins it bundles, as it does once it's started (`deploy_bundled/0`).
  """
  def await_bundled do
    case List.keyfind(Supervisor.which_children(OneCore.Supervisor), :bundled, 0) do
      {:bundled, pid, _type, _modules} when is_pid(pid) ->
        deploying = Process.monitor(pid)
        receive do: ({:DOWN, ^deploying, _, _, _} -> :ok)

      _ ->
        :ok
    end
  end

  @doc """
  What of the plugins this server ships it's still deploying, or failed to, by name: `"waiting"`, `"building"`, or
  why it failed. Empty once every one is deployed.
  """
  def deploying_bundled, do: Agent.get(__MODULE__, & &1)

  @doc false
  def child_spec(_), do: %{id: __MODULE__, start: {Agent, :start_link, [fn -> %{} end, [name: __MODULE__]]}}

  defp deploying(name, nil), do: Agent.update(__MODULE__, &Map.delete(&1, name))
  defp deploying(name, said), do: Agent.update(__MODULE__, &Map.put(&1, name, said))

  # Deploys one plugin this server ships for the network, and notes its files among what servers have shipped. It's
  # installed for the network if it's essential or the network has it already.
  defp deploy_bundled(name) do
    files = bundled(name)
    source = Version.code(files)
    started = System.monotonic_time(:millisecond)
    Logger.info("Shipping #{name}")
    deploying(name, "building")

    with true <- files != %{} || {:error, "There are no files to deploy it from"},
         {installed, _tag} = Installed.read(@network),
         install? = &(&1.essential or is_map_key(installed, name)),
         {:ok, number, _warnings} = deployed <- deploy(@network, name, files, nil, install?),
         :ok <- note_bundled("#{@network}/plugins/#{name}/bundled", source) do
      Logger.info("Shipped #{name} as version #{number} in #{System.monotonic_time(:millisecond) - started} ms")
      deploying(name, nil)
      deployed
    else
      {:error, why} = failed ->
        Logger.error("#{name} wasn't deployed: #{why}")
        deploying(name, why)
        failed
    end
  end

  defp note_bundled(blob, source) do
    case Bucket.read(blob) do
      {:ok, said, tag} ->
        sources = bundled_sources(said)

        cond do
          source in sources -> :ok
          Bucket.replace(blob, JSON.encode!([source | sources]), tag) == :ok -> :ok
          true -> note_bundled(blob, source)
        end

      :absent ->
        with :exists <- Bucket.write(blob, JSON.encode!([source])), do: note_bundled(blob, source)
    end
  end

  # Whether the plugin `name` that this server ships, from files whose hash is `source`, is to be deployed for the
  # network: it never had it, or it runs what a server shipped once and that isn't this. One whose latest version
  # doesn't say what it was deployed from is from before versions said so, and was shipped too.
  defp deploy_bundled?(name, source) do
    artifact = "#{@network}/plugins/#{name}"

    case Store.latest_source(artifact) do
      nil ->
        true

      ^source ->
        false

      latest ->
        case Bucket.read(artifact <> "/bundled") do
          {:ok, said, _tag} -> latest in bundled_sources(said)
          :absent -> false
        end
    end
  end

  defp bundled_sources(said),
    do: with(sources when is_list(sources) <- JSON.decode!(said), do: sources, else: (_ -> []))

  @doc """
  Whether a plugin fits whoever it's installed for: the plugins it uses are there for them, and one with scheduled jobs
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
         true <- client.(Store.source(artifact, kept)) == client.(files) do
      {:ok,
       Map.merge(
         Map.reject(files, fn {path, _} -> Version.client?(path) end),
         client.(Store.files(artifact, kept))
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
    for {owner, installed} <- Plugins.installed(),
        {name, artifact} <- installed,
        %Version{ui: built_with} = version <- [Plugins.version(artifact)],
        built_with != nil,
        ui = Plugins.running(owner)["ui"],
        ui != nil and ui.code != built_with,
        whose = Store.whose(artifact),
        Servers.here?(whose) and Plugins.plugin(whose, Path.basename(artifact)) == artifact,
        uniq: true do
      with {:error, why} <- deploy(whose, Path.basename(artifact), Store.source(artifact, version.code)),
           do: Logger.error("#{name}'s app wasn't built again against the UI library that changed: #{why}")
    end
  end

  # The plugins a plugin uses are there before it is. One this server ships that's running from files a server
  # shipped before this one's is deployed again, by a server acting for the network, and one that's missing is
  # installed for `owner` from what the network has of it. Bundled plugins go by name, so a plugin can come before
  # one it uses that's still to be shipped.
  defp bring(owner, name, uses) do
    for used <- uses,
        Servers.here?(@network) and bundled(used) != %{} and deploy_bundled?(used, Version.code(bundled(used))),
        do: deploy_bundled(used)

    there =
      &(is_map_key(Plugins.running(owner), &1) or
          (bundled(&1) != %{} and Installed.install(owner, &1, "#{@network}/plugins/#{&1}") == :ok))

    case Enum.reject(uses, there) do
      [] ->
        :ok

      [missing] ->
        {:error,
         "#{name} uses #{missing}, which isn't installed for #{owner} or the network. Add it for them first, or deploy it."}

      missing ->
        {:error,
         "#{name} uses #{Enum.join(missing, ", ")}, which aren't installed for #{owner} or the network. " <>
           "Add them for them first, or deploy them."}
    end
  end

  # The files of a plugin this server ships with, as a deploy from its folder sends them, or none if it ships with
  # no such plugin.
  defp bundled(name) do
    with %{^name => folder} <- Application.get_env(:one_core, :bundled, %{}),
         {:ok, paths} <- Folder.files(folder) do
      Map.new(paths, &{&1, File.read!(Path.join(folder, &1))})
    else
      _ -> %{}
    end
  end

  defp check(name, files) do
    cond do
      Installed.named(name) != :ok ->
        Installed.named(name)

      not (is_map(files) and map_size(files) > 0 and
               Enum.all?(files, fn {path, text} -> path?(path) and is_binary(text) end)) ->
        {:error, "A plugin's files are text by their paths, with no path part starting with a dot"}

      true ->
        :ok
    end
  end

  defp path?(path), do: is_binary(path) and Enum.all?(String.split(path, "/"), &(&1 =~ ~r/^[^.]/))
end
