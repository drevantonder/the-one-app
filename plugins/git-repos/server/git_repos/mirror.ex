defmodule GitRepos.Mirror do
  @moduledoc """
  One repo on this server: a bare git repo in the plugin's folder, kept the same as the repo in the bucket, so git
  itself reads it, packs what a fetch wants and checks what a push brings.

  A repo in the bucket is `repo.json` and its packs. `repo.json` holds its refs, which branch is its `head`, the
  names of the packs that hold its objects, the pull requests, and the packs merged away lately, which are
  removed an hour after. Each pack is `packs/<name>.pack` and its index `packs/<name>.idx`, written once and never
  changed. So a push writes its own pack and replaces `repo.json`, and nothing else: a small commit to a large repo
  writes a small pack.

  A change is a new `repo.json` replacing the one it was decided from, only if that's still the one there. Another
  push, on this server or another, that got there first makes it read `repo.json` again and decide again: a ref
  that moved meanwhile is refused, so nothing a push answered is lost. Packs are written before `repo.json` names
  them, so one a server stopped before naming is only unused.

  One process for each repo on each server does the changes and brings the mirror up to date, one at a time, so
  the mirror's packs are never removed from under a change. Reading runs beside it, once it's up to date.
  """
  use GenServer
  alias OneCore.Artifact

  @zero String.duplicate("0", 40)
  # How many packs a repo keeps before the newest are merged, as git's geometric repack merges them: each pack at
  # least twice the size of all the newer ones together, so a merge rewrites mostly what's new.
  @packs 8
  # How long a pack merged away is kept, for a server that read `repo.json` before the merge and is fetching it.
  @kept_for :timer.hours(1)
  # How many times a change is decided again after another got there first.
  @tries 8

  def zero, do: @zero

  @doc "The process for the repo `repo` (its artifact) on the server running `call`."
  def of(call, repo) do
    name = Artifact.name(repo)
    folder = Path.join([call.folder, "mirrors", Base.encode16(:crypto.hash(:sha256, name), case: :lower) <> ".git"])
    OneCore.Call.process(call, __MODULE__, {call.server, name}, folder)
  end

  @doc "Brings the mirror up to date with the bucket. Gives its folder and `repo.json` as it is."
  def sync(process, repo), do: answered(GenServer.call(process, {:sync, repo}, :infinity))

  @doc """
  Changes the repo. `pack`, a file of a pack a push brought, or `nil`, is indexed into the mirror first.
  `decide.(state, folder)` gives the new `repo.json` (or `nil` to change nothing), the names of any packs it made
  in the mirror, and what to answer; it's called again with what's there now if another change got there first.
  Gives what it answered, and `repo.json` as it is after.
  """
  def change(process, repo, pack, decide),
    do: answered(GenServer.call(process, {:change, repo, pack, decide}, :infinity))

  defp answered({:ok, answer}), do: answer
  defp answered({:error, why}), do: raise(why)

  @impl true
  def init(folder), do: {:ok, folder}

  @impl true
  def handle_call({:sync, repo}, _from, folder) do
    {:reply,
     answering(fn ->
       {state, _tag} = read!(repo)
       reconcile(folder, repo, state)
       {folder, state}
     end), folder}
  end

  def handle_call({:change, repo, pack, decide}, _from, folder) do
    {:reply,
     answering(fn ->
       {state, tag} = read!(repo)
       reconcile(folder, repo, state)
       indexed = pack && index(folder, pack)

       try do
         {answer, state} = attempt(folder, repo, state, tag, indexed, MapSet.new(), decide, @tries)
         if length(state["packs"]) > @packs, do: send(self(), {:merge, repo})
         {answer, state}
       after
         # The mirror is what the bucket says, whatever was decided: a pack nothing names goes.
         {state, _tag} = read!(repo)
         reconcile(folder, repo, state)
       end
     end), folder}
  end

  # What went wrong is said to whoever asked, and this carries on.
  defp answering(fun) do
    {:ok, fun.()}
  rescue
    error -> {:error, Exception.message(error)}
  end

  # The newest packs are merged into one, as git's geometric repack chooses, which is then the repo's in their
  # place. If another change got there first, this waits for the next.
  @impl true
  def handle_info({:merge, repo}, folder) do
    {state, tag} = read!(repo)
    reconcile(folder, repo, state)
    git!(folder, ["repack", "-d", "-q", "--geometric=2", "--no-write-bitmap-index"])
    packs = local_packs(folder)
    now = System.system_time(:millisecond)
    {old, kept} = Enum.split_with(state["retired"], &(now - &1["at"] > @kept_for))
    merged = for pack <- state["packs"] -- packs, do: %{"pack" => pack, "at" => now}

    if merged != [] do
      Enum.each(packs -- state["packs"], &store(folder, repo, &1))
      state = %{state | "packs" => packs, "retired" => kept ++ merged}

      if Artifact.replace(repo, "repo.json", JSON.encode!(state), tag) == :ok do
        for %{"pack" => pack} <- old, kind <- ["pack", "idx"], do: Artifact.remove(repo, "packs/#{pack}.#{kind}")
      end
    end

    {:noreply, folder}
  end

  defp attempt(_folder, repo, _state, _tag, _indexed, _stored, _decide, 0),
    do: raise("#{Artifact.name(repo)} is being changed by too many at once. Try again.")

  defp attempt(folder, repo, state, tag, indexed, stored, decide, tries) do
    case decide.(state, folder) do
      {nil, _made, answer} ->
        {answer, state}

      {changed, made, answer} ->
        packs = Enum.reject([indexed | made], &is_nil/1)
        for pack <- packs, pack not in stored, do: store(folder, repo, pack)
        changed = %{changed | "packs" => Enum.uniq(changed["packs"] ++ packs)}

        case Artifact.replace(repo, "repo.json", JSON.encode!(changed), tag) do
          :ok ->
            {answer, changed}

          :changed ->
            {state, tag} = read!(repo)
            attempt(folder, repo, state, tag, indexed, MapSet.new(packs), decide, tries - 1)
        end
    end
  end

  @doc "`repo.json` and the tag to replace it with, or raises if there's no such repo."
  def read!(repo) do
    case Artifact.read(repo, "repo.json") do
      {:ok, json, tag} -> {JSON.decode!(json), tag}
      :absent ->
        raise "There's no repo #{Artifact.name(repo)}. `repos list` lists the ones you may read, and its owner " <>
                "brings back one removed in the last 30 days with `repos restore`."
    end
  end

  # The mirror's packs become the ones `repo.json` names, and its refs and HEAD what it says.
  defp reconcile(folder, repo, state) do
    if not File.dir?(folder) do
      File.mkdir_p!(Path.dirname(folder))
      git!(Path.dirname(folder), ["init", "-q", "--bare", folder])
      git!(folder, ["config", "core.logAllRefUpdates", "false"])
    end

    have = local_packs(folder)
    Enum.each(state["packs"] -- have, &fetch(folder, repo, &1))

    for pack <- have -- state["packs"],
        file <- Path.wildcard(Path.join(folder, "objects/pack/pack-#{pack}.*")),
        do: File.rm!(file)

    now =
      for line <- String.split(git!(folder, ["for-each-ref", "--format=%(objectname) %(refname)"]), "\n", trim: true),
          into: %{},
          do: line |> String.split(" ", parts: 2) |> Enum.reverse() |> List.to_tuple()

    lines =
      Enum.flat_map(state["refs"], fn {ref, sha} -> if now[ref] == sha, do: [], else: ["update #{ref} #{sha}\n"] end) ++
        for ref <- Map.keys(now) -- Map.keys(state["refs"]), do: "delete #{ref}\n"

    if lines != [], do: git!(folder, ["update-ref", "--stdin"], input: Enum.join(lines))
    git!(folder, ["symbolic-ref", "HEAD", "refs/heads/" <> state["head"]])
  end

  defp local_packs(folder) do
    for file <- Path.wildcard(Path.join(folder, "objects/pack/pack-*.pack")),
        do: file |> Path.basename(".pack") |> String.replace_prefix("pack-", "")
  end

  # Its index last, since git finds a pack by its index.
  defp fetch(folder, repo, pack) do
    for kind <- ["pack", "idx"] do
      {:ok, data, _tag} = Artifact.read(repo, "packs/#{pack}.#{kind}")
      file = Path.join(folder, "objects/pack/pack-#{pack}.#{kind}")
      File.write!(file <> ".part", data)
      File.rename!(file <> ".part", file)
    end
  end

  defp store(folder, repo, pack) do
    for kind <- ["pack", "idx"] do
      data = File.read!(Path.join(folder, "objects/pack/pack-#{pack}.#{kind}"))
      :ok = with :exists <- Artifact.write(repo, "packs/#{pack}.#{kind}", data), do: :ok
    end
  end

  # A pack a push brought, completed with what it's thin against, as a pack of the mirror's. Gives its name, or
  # `nil` for one with no objects.
  defp index(folder, pack) do
    case File.read!(pack) do
      <<"PACK", _version::32, 0::32, _::binary>> -> nil
      _ -> git!(folder, ["index-pack", "--stdin", "--fix-thin"], from: pack) |> String.split() |> List.last()
    end
  end

  @doc """
  Runs git in `folder` and gives what it printed, or raises with what it said. `input:` is given to it, and
  `from:` names a file it's given instead.
  """
  def git!(folder, args, options \\ []) do
    case git(folder, args, options) do
      {out, 0} -> out
      {out, _} -> raise "git #{hd(args)} failed: #{String.trim(out)}"
    end
  end

  # Where this server finds git, or a sentence that says it can't.
  defp git_program do
    System.find_executable("git") ||
      raise "This server can't find git, which Git Repos needs: install it, then run `one serve on` again on its machine."
  end

  @doc "Runs git as `git!/3` does, and gives what it printed and how it ended."
  def git(folder, args, options \\ []) do
    env = [{"GIT_CONFIG_NOSYSTEM", "1"}, {"GIT_CONFIG_GLOBAL", "/dev/null"}, {"GIT_TERMINAL_PROMPT", "0"}]
    env = env ++ Keyword.get(options, :env, [])

    case {options[:input], options[:from]} do
      {nil, nil} ->
        System.cmd(git_program(), args, cd: folder, env: env, stderr_to_stdout: !options[:binary])

      {input, from} ->
        file = from || Path.join(System.tmp_dir!(), "git-repos-" <> Base.encode16(:crypto.strong_rand_bytes(8)))
        if input, do: File.write!(file, input)
        redirect = if options[:binary], do: "", else: " 2>&1"

        try do
          System.cmd("sh", ["-c", ~s(exec "#{git_program()}" "$@" < "$0"#{redirect}), file | args], cd: folder, env: env)
        after
          if input, do: File.rm(file)
        end
    end
  end
end
