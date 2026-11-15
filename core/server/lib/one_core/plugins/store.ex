defmodule OneCore.Plugins.Store do
  @moduledoc """
  A plugin as the bucket keeps it. A plugin is an artifact whose blobs are code, and it's whoever's deployed it:
  `<theirs>/plugins/<name>`. Each set of files deployed is kept at `code/<its hash>/`, and where the server built
  some of them (an app), the files as they were deployed are kept at `source/<the same hash>/`. Its `newest` says
  which set it runs now, with that version's number. A deploy replaces `newest` only if it's unchanged, so two
  deploys at once can't both be the plugin's next version, and first writes the version it replaces to
  `versions/<number>`, so a plugin keeps its history. `newest` names the files it was deployed from too, by their
  hash, its `source`.
  """
  require Logger
  alias OneCore.Bucket
  alias OneCore.Plugins.Version

  @doc "Whose the plugin `artifact` is: a user, a group or the network."
  def whose(artifact), do: String.replace_suffix(artifact, "/plugins/" <> Path.basename(artifact), "")

  @doc "One file of the set of the plugin `artifact`'s files named `code`, which needn't be the set running now."
  def file(artifact, code, path) do
    with {:ok, text, _tag} <- Bucket.read("#{artifact}/code/#{code}/#{path}"), do: {:ok, text}
  end

  @doc """
  A plugin's files as JSON holds them, as `deploy` takes them and `code` gives them: each one's text, or
  `%{"base64" => ...}` for one that isn't text.
  """
  def sent(files) do
    Map.new(files, fn {path, data} ->
      {path, if(String.valid?(data), do: data, else: %{"base64" => Base.encode64(data)})}
    end)
  end

  @doc "A plugin's files from what `sent/1` gives."
  def received(files) do
    Map.new(files, fn
      {path, %{"base64" => data}} -> {path, Base.decode64!(data)}
      {path, text} -> {path, text}
    end)
  end

  @doc "The files the plugin `artifact`'s set of files named `code` was deployed with: its source."
  def source(artifact, code) do
    case files(artifact, code, "source") do
      deployed when deployed == %{} -> files(artifact, code)
      deployed -> deployed
    end
  end

  @doc """
  A plugin's newest version as the bucket has it, its number and the set of files it runs, and the blob's tag.
  `nil` if there's no such plugin.
  """
  def newest(artifact) do
    with {:ok, newest, tag} <- Bucket.read("#{artifact}/newest"),
         %{"number" => number, "code" => code} = JSON.decode!(newest) do
      {number, code, tag}
    else
      :absent -> nil
    end
  end

  @doc "The hash of the files the plugin `artifact`'s newest version was deployed from, or `nil` if there's none."
  def newest_source(artifact) do
    with {:ok, newest, _tag} <- Bucket.read("#{artifact}/newest"), do: JSON.decode!(newest)["source"], else: (_ -> nil)
  end

  @doc """
  Writes a set of files, and makes it the plugin's next version: `files` as the server keeps them, and `deployed`
  as they were deployed. The version being replaced goes to the plugin's history first, so every version but the
  newest is there. Gives the version's number.
  """
  def keep(artifact, name, files, deployed) do
    code = Version.code(files)
    for {path, text} <- files, do: Bucket.write("#{artifact}/code/#{code}/#{path}", text)

    if deployed != files,
      do: for({path, text} <- deployed, do: Bucket.write("#{artifact}/source/#{code}/#{path}", text))

    was = newest(artifact)
    number = if was, do: elem(was, 0) + 1, else: 1
    next = JSON.encode!(%{number: number, code: code, source: Version.code(deployed)})

    wrote =
      with {before, code, tag} <- was do
        Bucket.write("#{artifact}/versions/" <> String.pad_leading("#{before}", 8, "0"), code)
        Bucket.replace("#{artifact}/newest", next, tag)
      else
        nil -> Bucket.write("#{artifact}/newest", next)
      end

    # Another deploy of this plugin got there first, and whoever asked for this one hasn't seen it.
    if wrote == :ok,
      do: {:ok, number},
      else: {:error, "#{name} was deployed by someone else just now. Look at it, and do this again."}
  end

  @doc """
  The versions in the bucket of the plugins `artifacts` that are newer than the ones in `newest`: each one's
  plugin, its number, and the version loaded, which is `:error` for one this server can't compile. Read at once:
  a bucket at an address is asked for each over the internet.
  """
  def newer(artifacts, newest) do
    artifacts
    |> Task.async_stream(&{&1, newest(&1)}, max_concurrency: 16, timeout: :infinity)
    |> Enum.flat_map(fn {:ok, {artifact, in_bucket}} ->
      with {number, code, _tag} <- in_bucket,
           true <- number > Map.get(newest, artifact, 0),
           do: [{artifact, number, loaded(artifact, code)}],
           else: (_ -> [])
    end)
  end

  @doc """
  A set of files of a plugin, by their paths: the files the server keeps, or with `"source"` the ones they were
  built from. Read at once: a bucket at an address is asked for each over the internet.
  """
  def files(artifact, code, kept \\ "code") do
    paths = Bucket.list("#{artifact}/#{kept}/#{code}/")

    paths
    |> Task.async_stream(&Bucket.read/1, max_concurrency: 16, timeout: :infinity)
    |> Enum.zip_with(paths, fn {:ok, {:ok, text, _tag}}, path ->
      {String.replace_prefix(path, "#{artifact}/#{kept}/#{code}/", ""), text}
    end)
    |> Map.new()
  end

  @doc "Loads the set of the plugin `artifact`'s files named `code`: `{:ok, version}`, or `:error` if it doesn't compile here."
  def loaded(artifact, code) do
    case Version.load(Path.basename(artifact), files(artifact, code)) do
      {:ok, version, _warnings} ->
        {:ok, %{version | artifact: artifact}}

      {:error, why} ->
        Logger.error("#{artifact} isn't updated here, because its newest version doesn't compile:\n#{why}")
        :error
    end
  end
end
