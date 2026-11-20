defmodule OneCore.Apps.Store do
  @moduledoc """
  An app as the bucket keeps it. An app is an artifact whose blobs are code, and it's whoever's deployed it:
  `<theirs>/apps/<name>`. Each set of files deployed is kept at `code/<its hash>/`, and where the server built
  some of them (an app), the files as they were deployed are kept at `source/<the same hash>/`. Its `latest` says
  which set it runs now, with that version's number. A deploy replaces `latest` only if it's unchanged, so two
  deploys at once can't both be the app's next version, and first writes the version it replaces to
  `versions/<number>`, with when; only the last few versions, and those replaced in the last week, keep their files
  (`prune/1`). Each deploy's set is its own
  (`deployment/1`), so one that goes back to an earlier version's files writes them again. `latest` names the files it was deployed from too, by their
  hash, its `source`, and the git repo it was installed from, if it was, its `from`: as a lock file has it, where
  it was fetched from, the ref asked for, the commit and the hash of the files as installed (`one install`). Each
  version after keeps its `from`, so one whose `source` isn't that hash was changed since. `trusted` says whether it's
  trusted: an untrusted app runs in a sandbox (`OneCore.Sandbox`). What a set of files says of itself once loaded is
  kept at `described/<its hash>`, for the servers that don't load it (`described/2`). A deploy writes a set, and
  what it describes, before it swaps `latest`, which is what commits it (`keep/8`): one that stops before leaves a set
  no version names, which `prune/1` sweeps.
  """
  require Logger
  alias OneCore.{Bucket, Server}
  alias OneCore.Apps.{LoadedVersion, Version}
  alias OneCore.Apps.Version.{Compile, Description}

  @doc "Whose the app `artifact` is: a user, a group or the network."
  def whose(artifact), do: String.replace_suffix(artifact, "/apps/" <> Path.basename(artifact), "")

  # Each deploy's set of files is its own, by an id no other deploy has, kept among them as One Core's own file
  # `deployment`, as `client/ui` is (`OneCore.Entrypoint`): so no deploy writes files an earlier one wrote, which a
  # prune may have chosen to remove (`prune/1`).
  @deployment "deployment"

  @doc "One file of the set of the app `artifact`'s files named `code`, which needn't be the set running now."
  def file(artifact, code, path) do
    with {:ok, text, _tag} <- Bucket.read("#{artifact}/code/#{code}/#{path}"), do: {:ok, text}
  end

  @doc "Each of several files, `{artifact, code, path}` as `file/3` takes them, read at once: `{:ok, text}` or `:absent`."
  def read_all(files) do
    for read <- Bucket.read_all(for {artifact, code, path} <- files, do: "#{artifact}/code/#{code}/#{path}"),
        do: with({:ok, text, _tag} <- read, do: {:ok, text})
  end

  @doc "The paths of the set of the app `artifact`'s files named `code`, without reading them."
  def paths(artifact, code) do
    prefix = "#{artifact}/code/#{code}/"
    for blob <- Bucket.list(prefix), do: String.replace_prefix(blob, prefix, "")
  end

  @doc """
  An app's files as JSON holds them, as `deploy` takes them and `code` gives them: each one's text, or
  `%{"base64" => ...}` for one that isn't text.
  """
  def sent(files) do
    Map.new(files, fn {path, data} ->
      {path, if(String.valid?(data), do: data, else: %{"base64" => Base.encode64(data)})}
    end)
  end

  @doc "An app's files from what `sent/1` gives."
  def received(files) do
    Map.new(files, fn
      {path, %{"base64" => data}} -> {path, Base.decode64!(data)}
      {path, text} -> {path, text}
    end)
  end

  @doc "The files the app `artifact`'s set of files named `code` was deployed with: its source."
  def source(artifact, code) do
    case files(artifact, code, "source") do
      deployed when deployed == %{} -> Map.delete(files(artifact, code), @deployment)
      deployed -> deployed
    end
  end

  @doc """
  An app's latest version as the bucket has it, its number and the set of files it runs, and the blob's tag.
  `nil` if there's no such app.
  """
  def latest(artifact) do
    with {:ok, latest, tag} <- Bucket.read("#{artifact}/latest"),
         %{"number" => number, "code" => code} = JSON.decode!(latest) do
      {number, code, tag}
    else
      :absent -> nil
    end
  end

  @doc """
  What the app `artifact`'s latest version says of itself: its `number`, `code`, `source` and, where the files came
  from a git repo, `from`. Empty if there's no such app.
  """
  def about(artifact) do
    with {:ok, latest, _tag} <- Bucket.read("#{artifact}/latest"), do: JSON.decode!(latest), else: (_ -> %{})
  end

  @doc """
  Whether the app `artifact` is trusted, as its latest version says (`about/1`): an untrusted one runs in a sandbox
  (`OneCore.Sandbox`), and one not deployed yet is trusted until it says otherwise.
  """
  def trusted?(artifact) when is_binary(artifact), do: trusted?(about(artifact))

  def trusted?(%{} = about), do: about["trusted"] != false

  @doc """
  Whether two apps are copies of the same app: the same artifact, or the same name deployed from the same git repo,
  whose folder `apps/<name>` both take their updates from (`OneCore.Apps.Lifecycle.check/1`). An app that says no
  repo is the same only as itself, so a different app that happens to share a name never is.
  """
  def same?(artifact, artifact), do: true

  def same?(one, other),
    do: Path.basename(one) == Path.basename(other) and repo(one) != nil and repo(one) == repo(other)

  @doc """
  The git repo an app was deployed from, as its latest version says (`about/1`), or as a deploy's `from` says,
  written one way however it was given: `owner/repo` for GitHub's, as `one install` takes it. `nil` for none.
  """
  def repo(artifact) when is_binary(artifact), do: repo(about(artifact)["from"])

  def repo(%{"source" => source}) when is_binary(source),
    do:
      source
      |> String.downcase()
      |> String.trim_trailing("/")
      |> String.replace_suffix(".git", "")
      |> String.replace(~r{^(https?://|ssh://git@|git@)github\.com[/:]}, "")

  def repo(_from), do: nil

  @doc "A deploy's `files`, with the id that makes its set of files its own (`keep/8`)."
  def deployment(files), do: Map.put(files, @deployment, Base.encode16(:crypto.strong_rand_bytes(16), case: :lower))

  @doc """
  Commits a deploy: writes its set of files, as `deployment/1` made them, and what its `version` says of itself
  (`describe/1`), and makes it the app's next version, then prunes the app (`prune/1`). `files` are as the server
  keeps them, and `deployed` as they were deployed. `was` is the app's `latest` as the deploy read it before it wrote
  anything (`latest/1`): `latest` is replaced only if it's still that, so two deploys at once can't both be the app's
  next version, and one that a prune began after can't name what the prune removes. The version being replaced goes
  to the app's history first, so every version but the latest is there, and `from`, the git repo its files came
  from, is kept with it (`:stored` keeps what the version before said, and `nil` says they came from none), and
  whether it's `trusted`. Gives the version's number.
  """
  def keep(artifact, name, version, files, deployed, from, trusted, was) do
    code = Version.code(files)
    sources = if deployed != Map.delete(files, @deployment), do: deployed, else: %{}

    # The version being replaced goes to the history.
    history =
      with {before, code, _tag} <- was do
        replaced = JSON.encode!(%{code: code, replaced: System.os_time(:second)})
        [fn -> Bucket.write("#{artifact}/versions/" <> String.pad_leading("#{before}", 8, "0"), replaced) end]
      else
        nil -> []
      end

    # Each blob is written at once beside the others, as a bucket at an address takes a round trip for each.
    writes =
      [fn -> :ok = describe(version) end | history] ++
        for({path, text} <- files, do: fn -> Bucket.write("#{artifact}/code/#{code}/#{path}", text) end) ++
        for({path, text} <- sources, do: fn -> Bucket.write("#{artifact}/source/#{code}/#{path}", text) end)

    Stream.run(Task.async_stream(writes, & &1.(), max_concurrency: 16, timeout: :infinity))
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
      with {_before, _code, tag} <- was do
        Bucket.replace("#{artifact}/latest", next, tag)
      else
        nil -> Bucket.write("#{artifact}/latest", next)
      end

    # Another deploy of this app got there first, or a prune began since this one read `latest`, and whoever asked
    # for this one hasn't seen it. What's pruned is nobody's to wait for.
    if wrote == :ok do
      Task.Supervisor.start_child(OneCore.Asks, fn -> prune(artifact) end)
      {:ok, number}
    else
      {:error, "#{name} was deployed by someone else just now. Look at it, and do this again."}
    end
  end

  # Which of an app's versions keep their files: the latest, which every server runs, the two before it, and every one
  # replaced in the last week, which a server still moving to the latest may run and list, and a device open on it may
  # still read, as a CDN keeps a site's earlier hashed assets: a server release builds every app again, so a day of
  # releases replaces an app's version many times.
  @versions_kept 3
  @kept_for 7 * 24 * 3600

  @doc """
  Removes the files of the app `artifact`'s versions before its last few (`@versions_kept`) and replaced over a week
  ago (`@kept_for`): the set each ran, the files it was deployed from and what it said of itself, and its entry in
  `versions/`. Nothing runs them, and nothing
  goes back to one: an app goes back by being deployed again. A set a kept version runs too stays. So do the sets no
  version names, as `git gc` removes what no ref reaches: what a deploy that failed, stopped or is still under way
  wrote. Gives how many blobs went.

  What it removes is chosen from one `latest`, which it replaces with itself, counted, before it removes anything:
  a deploy under way read `latest` before it wrote its set (`keep/8`), so its swap is refused and it names
  nothing removed, and every deploy after writes a set of its own that wasn't there to be chosen. One that finds
  `latest` changed meanwhile removes nothing: the deploy that changed it prunes after.
  """
  def prune(artifact) do
    with {:ok, json, tag} <- Bucket.read("#{artifact}/latest") do
      latest = JSON.decode!(json)
      history = Bucket.list("#{artifact}/versions/")
      replaced = Enum.zip_with(history, Bucket.read_all(history), fn blob, {:ok, text, _tag} -> {blob, replaced(text)} end)
      week_ago = System.os_time(:second) - @kept_for

      {old, retained} =
        Enum.split_with(replaced, fn {blob, %{"replaced" => at}} ->
          String.to_integer(Path.basename(blob)) <= latest["number"] - @versions_kept and at < week_ago
        end)

      retained = MapSet.new([latest["code"] | Enum.map(retained, &elem(&1, 1)["code"])])

      gone =
        for(
          prefix <- ~w(code source described),
          blob <- Bucket.list("#{artifact}/#{prefix}/"),
          set(artifact, prefix, blob) not in retained,
          do: blob
        ) ++ Enum.map(old, &elem(&1, 0))

      counted = JSON.encode!(Map.update(latest, "pruned", 1, &(&1 + 1)))

      if gone != [] and Bucket.replace("#{artifact}/latest", counted, tag) == :ok do
        Stream.run(Task.async_stream(gone, &Bucket.remove/1, max_concurrency: 16, timeout: :infinity))
        length(gone)
      else
        0
      end
    else
      :absent -> 0
    end
  end

  # An entry of `versions/`: the code of the version it replaced, and when. One written before they said when holds the
  # code alone, and was replaced longer ago than any kept.
  defp replaced(text) do
    case JSON.decode(text) do
      {:ok, %{"code" => _, "replaced" => _} = replaced} -> replaced
      _ -> %{"code" => text, "replaced" => 0}
    end
  end

  # The hash of the set a blob of `prefix` is in: `code/<hash>/<path>`, `source/<hash>/<path>`, `described/<hash>`.
  defp set(artifact, prefix, blob), do: blob |> String.replace_prefix("#{artifact}/#{prefix}/", "") |> String.split("/") |> hd()

  @doc """
  The versions in the bucket of the apps `artifacts` that are newer than the ones in `latest`: each one's
  app, its number, and the version loaded, which is `:error` for one this server can't compile, or with `load`
  false the version described (`described/2`), leaving out one not described yet. An untrusted one is never
  loaded here, only described. Read and loaded at once: a bucket at an address is asked for each over the
  internet, and a version loaded may be compiled.

  One that couldn't be read or loaded, as when the bucket didn't give its blobs or refused them, is left out, as one
  that isn't newer is, and said in the log: one app never stops the others. The first answer a bucket that didn't
  answer gives after has this server look again (`OneCore.Bucket`), which finds it newer then.
  """
  def newer(artifacts, latest, load \\ true) do
    caller = self()

    artifacts
    |> Task.async_stream(&newer(&1, latest, load, caller), max_concurrency: 16, timeout: :infinity)
    |> Enum.flat_map(fn {:ok, newer} -> newer end)
  end

  # Each is loaded beside the others, and held for the caller before the process that loaded it ends.
  defp newer(artifact, latest, load, caller) do
    with %{"number" => number, "code" => code} = in_bucket <- unanswered(artifact, fn -> about(artifact) end),
         true <- number > Map.get(latest, artifact, 0),
         version when version != nil <-
           unanswered(artifact, fn -> version(artifact, code, load, trusted?(in_bucket)) end),
         true <- load or version != :error do
      with {:ok, held} <- version, do: :ok = LoadedVersion.hold([held], caller)
      [{artifact, number, version}]
    else
      _ -> []
    end
  end

  defp unanswered(artifact, read) do
    read.()
  rescue
    unanswered ->
      Logger.error("#{artifact} isn't updated here: #{Exception.message(unanswered)}")
      nil
  end

  @doc """
  The set of the app `artifact`'s files named `code`, as this server would run it, held by the caller
  (`OneCore.Apps.LoadedVersion`): loaded, or described where `load` is false, and an untrusted one always
  described, as the server that deployed it described it in its sandbox. `{:ok, version}`, or `:error`.
  """
  def version(artifact, code, _load, false) do
    with {:ok, version} <- held(artifact, code), :ok <- runs(version), do: {:ok, %{version | trusted: false}}
  end

  def version(artifact, code, true, true), do: loaded(artifact, code)
  def version(artifact, code, false, true), do: held(artifact, code)

  @doc """
  The set of the app `artifact`'s files named `code` as it described itself where it was loaded, without its
  code, to read what it says: `{:ok, version}`, or `:error` while no server that loads it has said. It isn't held,
  so nothing it keeps (`OneCore.Apps.Version.files/1`) is there to read.
  """
  def described(artifact, code) do
    with {:ok, version, _kept} <- description(artifact, code), do: {:ok, version}
  end

  defp held(artifact, code) do
    with {:ok, version, kept} <- description(artifact, code) do
      LoadedVersion.described(version, kept)
      {:ok, version}
    end
  end

  defp description(artifact, code) do
    with {:ok, description, _tag} <- Bucket.read("#{artifact}/described/#{code}"),
         do: Description.described(artifact, description),
         else: (:absent -> :error)
  end

  @doc """
  Keeps what a loaded version says of itself, for the servers that don't load it, as this server says it. Only a server
  acting for whoever the app is says it, since only those may write the app's blobs: a user's own server loads the
  network's apps whose code runs inside its apps too, and the network's servers say what those are.
  """
  def describe(%Version{artifact: artifact, code: code} = version) do
    if OneCore.Servers.here?(OneCore.User.owner(artifact)) do
      blob = "#{artifact}/described/#{code}"
      description = Description.description(version)

      # One said otherwise, as an earlier build of this server said it, is said again.
      with :exists <- Bucket.write(blob, description),
           {:ok, was, tag} when was != description <- Bucket.read(blob),
           do: Bucket.replace(blob, description, tag)
    end

    :ok
  end

  # A version this server can't run, for its `core`, is let go of and said in telemetry (`OneCore.Apps.Version.platform/1`).
  defp runs(version) do
    if why = Version.waiting(Path.basename(version.artifact), version) do
      Logger.warning(why)
      LoadedVersion.release([version])
      :error
    else
      :ok
    end
  end

  @doc """
  Whether the app `artifact`'s latest version runs on this server's platform, as `name`
  (`OneCore.Apps.Version.platform/1`): `%{state: :runs | :waiting_for_server, why: ...}`,
  from what it said of itself, or `nil` where no server has described it.
  """
  def platform(name, artifact) do
    with %{"code" => code} <- about(artifact),
         {:ok, version} <- described(artifact, code),
         do: %{state: Version.platform(version), why: Version.waiting(name, version)},
         else: (_ -> nil)
  end

  @doc """
  A set of files of an app, by their paths: the files the server keeps, or with `"source"` the ones they were
  built from. Read at once: a bucket at an address is asked for each over the internet.

  The files a server keeps are named by their hash, so they never change: once read, they're kept on this machine
  beside what they're compiled to (`OneCore.Apps.Version.Compile`), which goes as that does, and read from there for
  as long as they still hash to their name. So a server that restarts asks the bucket for no app's files it had.
  """
  def files(artifact, code, prefix \\ "code")

  def files(artifact, code, "code") do
    kept = Path.join([Server.home(), "compiled", Version.code({artifact, code})])

    with %{} = files <- kept(kept), ^code <- Version.code(files) do
      File.touch(kept)
      files
    else
      _ -> tap(read(artifact, code, "code"), &keep_files(kept, code, &1))
    end
  end

  def files(artifact, code, prefix), do: read(artifact, code, prefix)

  defp kept(file) do
    :erlang.binary_to_term(File.read!(file))
  rescue
    _ -> nil
  end

  # Written beside, then renamed into place, so what's kept is all there or not at all; and only a set that's all
  # there, as its hash says, is kept.
  defp keep_files(kept, code, files) do
    if Version.code(files) == code do
      writing = "#{kept}.#{System.unique_integer([:positive])}"
      File.mkdir_p!(Path.dirname(kept))
      File.write!(writing, :erlang.term_to_binary(files))
      File.rename!(writing, kept)
    end
  rescue
    error -> Logger.warning("An app's files weren't kept on this machine: #{Exception.message(error)}")
  end

  defp read(artifact, code, prefix) do
    paths = Bucket.list("#{artifact}/#{prefix}/#{code}/")

    paths
    |> Task.async_stream(&Bucket.read/1, max_concurrency: 16, timeout: :infinity)
    |> Enum.zip_with(paths, fn {:ok, {:ok, text, _tag}}, path ->
      {String.replace_prefix(path, "#{artifact}/#{prefix}/#{code}/", ""), text}
    end)
    |> Map.new()
  end

  @doc """
  Loads the set of the app `artifact`'s files named `code`, held by the caller: `{:ok, version}`, or `:error` if it doesn't compile
  here, or doesn't run on this server's build (`OneCore.Apps.Version`).
  """
  def loaded(artifact, code) do
    case LoadedVersion.load(artifact, code, &Compile.compile(artifact, files(artifact, code), &1)) do
      {:ok, version, _warnings} ->
        describe(version)
        with :ok <- runs(version), do: {:ok, version}

      {:error, why} ->
        Logger.error("#{artifact} isn't updated here, because its latest version doesn't load:\n#{why}")
        :error
    end
  end
end
