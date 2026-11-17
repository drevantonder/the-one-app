defmodule OneCore.Plugins.Store do
  @moduledoc """
  A plugin as the bucket keeps it. A plugin is an artifact whose blobs are code, and it's whoever's deployed it:
  `<theirs>/plugins/<name>`. Each set of files deployed is kept at `code/<its hash>/`, and where the server built
  some of them (an app), the files as they were deployed are kept at `source/<the same hash>/`. Its `latest` says
  which set it runs now, with that version's number. A deploy replaces `latest` only if it's unchanged, so two
  deploys at once can't both be the plugin's next version, and first writes the version it replaces to
  `versions/<number>`; only the last few versions keep their files (`prune/1`). `latest` names the files it was deployed from too, by their
  hash, its `source`, and the git repo it was installed from, if it was, its `from`: as a lock file has it, where
  it was fetched from, the ref asked for, the commit and the hash of the files as installed (`one install`). Each
  version after keeps its `from`, so one whose `source` isn't that hash was changed since. `trusted` says whether it's
  trusted: an untrusted plugin runs in a sandbox (`OneCore.Sandbox`). What a set of files says of itself once loaded is
  kept at `described/<its hash>`, for the servers that don't load it (`described/2`).
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
  A plugin's latest version as the bucket has it, its number and the set of files it runs, and the blob's tag.
  `nil` if there's no such plugin.
  """
  def latest(artifact) do
    with {:ok, latest, tag} <- Bucket.read("#{artifact}/latest"),
         %{"number" => number, "code" => code} = JSON.decode!(latest) do
      {number, code, tag}
    else
      :absent -> nil
    end
  end

  @doc "The hash of the files the plugin `artifact`'s latest version was deployed from, or `nil` if there's none."
  def latest_source(artifact), do: about(artifact)["source"]

  @doc """
  What the plugin `artifact`'s latest version says of itself: its `number`, `code`, `source` and, where the files came
  from a git repo, `from`. Empty if there's no such plugin.
  """
  def about(artifact) do
    with {:ok, latest, _tag} <- Bucket.read("#{artifact}/latest"), do: JSON.decode!(latest), else: (_ -> %{})
  end

  @doc """
  Writes a set of files, and makes it the plugin's next version: `files` as the server keeps them, and `deployed`
  as they were deployed. The version being replaced goes to the plugin's history first, so every version but the
  latest is there, and `from`, the git repo its files came from, is kept with it (`:stored` keeps what the version
  before said, and `nil` says they came from none), and whether it's `trusted`. Gives the version's number.
  """
  def keep(artifact, name, files, deployed, from, trusted) do
    code = Version.code(files)
    for {path, text} <- files, do: Bucket.write("#{artifact}/code/#{code}/#{path}", text)

    if deployed != files,
      do: for({path, text} <- deployed, do: Bucket.write("#{artifact}/source/#{code}/#{path}", text))

    was = latest(artifact)
    number = if was, do: elem(was, 0) + 1, else: 1
    source = Version.code(deployed)

    from =
      case from do
        :stored -> about(artifact)["from"]
        nil -> nil
        from -> Map.put(from, "hash", source)
      end

    next =
      %{number: number, code: code, source: source, from: from, trusted: trusted}
      |> Map.reject(&is_nil(elem(&1, 1)))
      |> JSON.encode!()

    wrote =
      with {before, code, tag} <- was do
        Bucket.write("#{artifact}/versions/" <> String.pad_leading("#{before}", 8, "0"), code)
        Bucket.replace("#{artifact}/latest", next, tag)
      else
        nil -> Bucket.write("#{artifact}/latest", next)
      end

    # Another deploy of this plugin got there first, and whoever asked for this one hasn't seen it.
    if wrote == :ok,
      do: {:ok, number},
      else: {:error, "#{name} was deployed by someone else just now. Look at it, and do this again."}
  end

  # How many of a plugin's versions keep their files: the latest, which every server runs, and the two before it, which
  # a server may still hold while it moves to the latest, and an app open on a device may still be reading.
  @versions_kept 3

  @doc """
  Removes the files of the plugin `artifact`'s versions before its last few (`@versions_kept`): the set each ran, the files
  it was deployed from and what it said of itself, and its entry in `versions/`. Nothing runs them, and nothing
  goes back to one: a plugin goes back by being deployed again from its git repo. A set a kept version runs too
  stays. Gives how many blobs went.
  """
  def prune(artifact) do
    with {number, latest, _tag} <- latest(artifact) do
      history = Bucket.list("#{artifact}/versions/")
      codes = Enum.zip_with(history, Bucket.read_all(history), fn blob, {:ok, code, _tag} -> {blob, code} end)
      {old, retained} = Enum.split_with(codes, fn {blob, _code} -> String.to_integer(Path.basename(blob)) <= number - @versions_kept end)
      retained = MapSet.new([latest | Enum.map(retained, &elem(&1, 1))])

      files =
        for code <- Enum.uniq(Enum.map(old, &elem(&1, 1))), code not in retained, prefix <- ~w(code source described) do
          Bucket.list("#{artifact}/#{prefix}/#{code}")
        end

      gone = List.flatten(files) ++ Enum.map(old, &elem(&1, 0))
      Stream.run(Task.async_stream(gone, &Bucket.remove/1, max_concurrency: 16, timeout: :infinity))
      length(gone)
    else
      nil -> 0
    end
  end

  @doc """
  The versions in the bucket of the plugins `artifacts` that are newer than the ones in `latest`: each one's
  plugin, its number, and the version loaded, which is `:error` for one this server can't compile, or with `load`
  false the version described (`described/2`), leaving out one not described yet. An untrusted one is never
  loaded here, only described. Read at once: a bucket at an address is asked for each over the internet.

  One that couldn't be read or loaded, as when the bucket didn't give its blobs or refused them, is left out, as one
  that isn't newer is, and said in the log: one plugin never stops the others. The first answer a bucket that didn't
  answer gives after has this server look again (`OneCore.Bucket`), which finds it newer then.
  """
  def newer(artifacts, latest, load \\ true) do
    artifacts
    |> Task.async_stream(&{&1, unanswered(&1, fn -> about(&1) end)}, max_concurrency: 16, timeout: :infinity)
    |> Enum.flat_map(fn {:ok, {artifact, in_bucket}} ->
      with %{"number" => number, "code" => code} <- in_bucket,
           true <- number > Map.get(latest, artifact, 0),
           version when version != nil <-
             unanswered(artifact, fn -> version(artifact, code, load, in_bucket["trusted"] != false) end),
           true <- load or version != :error,
           do: [{artifact, number, version}],
           else: (_ -> [])
    end)
  end

  defp unanswered(artifact, read) do
    read.()
  rescue
    unanswered ->
      Logger.error("#{artifact} isn't updated here: #{Exception.message(unanswered)}")
      nil
  end

  @doc """
  The set of the plugin `artifact`'s files named `code`, as this server would run it: loaded, or described where
  `load` is false, and an untrusted one always described, as the server that deployed it described it in its sandbox.
  `{:ok, version}`, or `:error`.
  """
  def version(artifact, code, _load, false) do
    with {:ok, version} <- described(artifact, code), do: {:ok, %{version | trusted: false}}
  end

  def version(artifact, code, true, true), do: loaded(artifact, code)
  def version(artifact, code, false, true), do: described(artifact, code)

  @doc """
  The set of the plugin `artifact`'s files named `code` as it described itself where it was loaded, without its
  code: `{:ok, version}`, or `:error` while no server that loads it has said.
  """
  def described(artifact, code) do
    with {:ok, description, _tag} <- Bucket.read("#{artifact}/described/#{code}"),
         do: Version.described(artifact, description),
         else: (:absent -> :error)
  end

  @doc """
  Keeps what a loaded version says of itself, for the servers that don't load it, as this server says it. Only a server
  acting for whoever the plugin is says it, since only those may write the plugin's blobs: a user's own server loads the
  network's plugins whose code runs inside its plugins too, and the network's servers say what those are.
  """
  def describe(%Version{artifact: artifact, code: code} = version) do
    if OneCore.Servers.here?(OneCore.User.owner(artifact)) do
      blob = "#{artifact}/described/#{code}"
      description = Version.description(version)

      # One said otherwise, as a server from before descriptions named only what any server knows said it, is said again.
      with :exists <- Bucket.write(blob, description),
           {:ok, was, tag} when was != description <- Bucket.read(blob),
           do: Bucket.replace(blob, description, tag)
    end

    :ok
  end

  @doc """
  A set of files of a plugin, by their paths: the files the server keeps, or with `"source"` the ones they were
  built from. Read at once: a bucket at an address is asked for each over the internet.
  """
  def files(artifact, code, prefix \\ "code") do
    paths = Bucket.list("#{artifact}/#{prefix}/#{code}/")

    paths
    |> Task.async_stream(&Bucket.read/1, max_concurrency: 16, timeout: :infinity)
    |> Enum.zip_with(paths, fn {:ok, {:ok, text, _tag}}, path ->
      {String.replace_prefix(path, "#{artifact}/#{prefix}/#{code}/", ""), text}
    end)
    |> Map.new()
  end

  @doc """
  Loads the set of the plugin `artifact`'s files named `code`: `{:ok, version}`, or `:error` if it doesn't compile
  here, or doesn't run on this server's build (`OneCore.Plugins.Version`).
  """
  def loaded(artifact, code) do
    case Version.load(Path.basename(artifact), files(artifact, code)) do
      {:ok, version, _warnings} ->
        version = %{version | artifact: artifact}
        describe(version)
        {:ok, version}

      {:error, why} ->
        Logger.error("#{artifact} isn't updated here, because its latest version doesn't load:\n#{why}")
        :error
    end
  end
end
