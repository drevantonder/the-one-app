defmodule Repos do
  @moduledoc """
  Repos: git hosting for people and their agents. A repo is an artifact, a person's, a group's or the network's,
  shared as anything is with relation tuples: a `viewer` clones and fetches it and suggests changes, and an
  `editor` pushes to it and takes or declines what's suggested. What it is in the bucket is `Repos.Mirror`'s.

  Git reaches a repo from a person's devices through `one`, which is git's remote helper for `one::` addresses
  (`git clone one::users/alice/made/repos/notes`): it runs git's own HTTP transport, and carries each of its
  requests to a server as a call of the tool `git`, signed as every call of the device is. So git needs no
  password, and works wherever the device reaches a server.

  A suggested change is a push to `refs/suggestions/<name>` by anyone who may read the repo. The repo's editors
  take it, which merges it into the branch it's for, or decline it.
  """
  @behaviour OneCore.Plugin
  require Logger
  alias OneCore.{Artifact, Call}
  alias Repos.{Mirror, Wire}

  @zero Mirror.zero()
  @repo ~r{^(users/[^/]+|groups/[^/]+|network)/made/repos/[a-z0-9][a-z0-9_-]*$}
  # Larger than this, what git sends or is answered goes through the bucket instead of in the call.
  @inline 4_194_304
  # How long an answer kept in the bucket for git to fetch is kept.
  @answers_kept :timer.minutes(10)
  # The most of a file that's shown, and of a commit's changes.
  @shown 1_048_576
  @page 25

  @impl true
  def apps, do: [%{name: "Repos", page: "app/index.html", icon: "icon.svg"}]

  @impl true
  def tools do
    repo = %{type: "string", description: "The repo, as `list` names it: users/alice/made/repos/notes"}
    ref = %{type: "string", description: "A branch, a tag or a commit. Left out, the repo's default branch"}
    path = %{type: "string", description: "A path in the repo, like src/app.js. Left out, the top"}
    suggestion = %{type: "string", description: "The suggestion's name, as `suggestions` lists them"}
    only = fn properties, required -> %{type: "object", properties: properties, required: required} end

    [
      %{
        name: "make",
        description:
          "Makes an empty repo, yours, or a group's or the network's when asked for them, and answers how to clone it",
        inputs:
          only.(%{name: %{type: "string", description: "Lowercase letters, digits, - and _, like notes"}}, ["name"])
      },
      %{
        name: "list",
        read_only: true,
        description:
          "Every repo you may read, the latest pushed first: its name, whose it is, when it was last pushed, and whether you push to it",
        inputs: only.(%{}, [])
      },
      %{
        name: "branches",
        read_only: true,
        description:
          "A repo's branches, each with its newest commit, the default first, and its address to clone it by",
        inputs: only.(%{repo: repo}, ["repo"])
      },
      %{
        name: "tree",
        read_only: true,
        description:
          "A folder of a repo at a ref: its files and folders, folders first, the newest commit that touched it, and its README's text",
        inputs: only.(%{repo: repo, ref: ref, path: path}, ["repo"])
      },
      %{
        name: "file",
        read_only: true,
        description:
          "A file of a repo at a ref: its text, or that it's binary or too large to show (then clone it), and its size",
        inputs:
          only.(%{repo: repo, ref: ref, path: %{path | description: "The file's path, like src/app.js"}}, [
            "repo",
            "path"
          ])
      },
      %{
        name: "log",
        read_only: true,
        description:
          "A repo's history at a ref, newest first, #{@page} commits a page, of the whole repo or one path: each commit's sha, subject, author and time, and whether there's more",
        inputs:
          only.(
            %{
              repo: repo,
              ref: ref,
              path: path,
              page: %{type: "integer", description: "0, the default, for the newest, then 1, 2 and on"}
            },
            ["repo"]
          )
      },
      %{
        name: "commit",
        read_only: true,
        description: "One commit: its message, author, time, parents, and what it changed, as a diff",
        inputs:
          only.(%{repo: repo, sha: %{type: "string", description: "The commit's sha, or its start"}}, [
            "repo",
            "sha"
          ])
      },
      %{
        name: "suggestions",
        read_only: true,
        description:
          "The changes suggested to a repo, open ones first: each one's name, who suggested it, the branch it's for, its title, its commits, and whether it was taken or declined",
        inputs: only.(%{repo: repo}, ["repo"])
      },
      %{
        name: "take",
        description:
          "Takes a suggested change into the branch it's for: a fast-forward where it can be, or else a merge. Only someone who pushes to the repo takes one",
        inputs: only.(%{repo: repo, suggestion: suggestion}, ["repo", "suggestion"])
      },
      %{
        name: "decline",
        description:
          "Declines a suggested change: it's no longer open, and nothing of it reaches a branch. Whoever pushes to the repo declines one, and so does whoever suggested it",
        inputs: only.(%{repo: repo, suggestion: suggestion}, ["repo", "suggestion"])
      },
      %{
        name: "remove",
        description:
          "Removes a repo you own: nobody reads, clones or pushes to it, until `restore` brings it back as it was",
        inputs: only.(%{repo: repo}, ["repo"])
      },
      %{
        name: "restore",
        description: "Brings back a repo you own that was removed, as it was",
        inputs: only.(%{repo: repo}, ["repo"])
      },
      %{
        name: "git",
        description:
          "Carries one request of git's smart HTTP protocol to a repo. It's for `git-remote-one`, which git runs for `one::` addresses, not for calling by hand: clone, fetch and push with git",
        inputs:
          only.(
            %{
              repo: repo,
              path: %{
                type: "string",
                description: "What follows the repo's address: info/refs?service=..., or the service"
              },
              body: %{type: "string", contentEncoding: "base64", description: "What git sent, if it's small"},
              upload: %{type: "string", format: "upload", description: "What git sent, if it's large"},
              protocol: %{type: "string", description: "Git's Git-Protocol header"},
              encoding: %{type: "string", description: "Git's Content-Encoding header"}
            },
            ["repo", "path"]
          )
      }
    ]
  end

  @impl true
  def call("make", %{"name" => name}, call) do
    if not (name =~ ~r/^[a-z0-9][a-z0-9_-]{0,63}$/),
      do: raise("A repo's name is lowercase letters, digits, - and _, starting with a letter or digit")

    repo = Call.make(call, name)
    now = System.system_time(:millisecond)

    state = %{
      "head" => "main",
      "refs" => %{},
      "packs" => [],
      "retired" => [],
      "suggestions" => %{},
      "made" => now,
      "by" => call.by || call.for
    }

    if Artifact.write(repo, "repo.json", JSON.encode!(state)) == :exists,
      do: raise("#{Artifact.name(repo)} is a repo already. Pick another name.")

    Artifact.write(index(), "index/" <> Artifact.name(repo), JSON.encode!(%{"pushed" => now}))

    "Made #{Artifact.name(repo)}. Clone it with `git clone #{address(repo)}`, or push to it from a repo you have " <>
      "with `git remote add origin #{address(repo)}` and `git push -u origin main`."
  end

  def call("list", _inputs, call) do
    names = for "index/" <> name <- Artifact.list(index(), "index/"), Call.reaches?(call, name), do: name

    names
    |> Enum.zip(Artifact.read_all(index(), Enum.map(names, &("index/" <> &1))))
    |> Enum.flat_map(fn {name, {:ok, data, _tag}} ->
      kept = JSON.decode!(data)

      if kept["removed"],
        do: [],
        else: [
          %{
            repo: name,
            whose: name |> String.split("/made/") |> hd(),
            pushed: kept["pushed"],
            pushes: Call.reaches?(call, name, "editor"),
            clone: address(name)
          }
        ]
    end)
    |> Enum.sort_by(& &1.pushed, :desc)
  end

  def call("branches", %{"repo" => name}, call) do
    {folder, state} = read(call, name)

    branches =
      for line <-
            lines(
              Mirror.git!(folder, [
                "for-each-ref",
                "--sort=-committerdate",
                "--format=%(refname:lstrip=2)%1f%(objectname)%1f%(subject)%1f%(authorname)%1f%(committerdate:unix)",
                "refs/heads"
              ])
            ) do
        [branch, sha, subject, author, at] = String.split(line, "\x1f")
        %{branch: branch, sha: sha, subject: subject, author: author, at: String.to_integer(at) * 1000}
      end

    %{
      repo: name,
      head: state["head"],
      whose: name |> String.split("/made/") |> hd(),
      clone: address(name),
      pushes: Call.reaches?(call, name, "editor"),
      owns: Call.reaches?(call, name, "owner"),
      branches: Enum.sort_by(branches, &(&1.branch != state["head"]))
    }
  end

  def call("tree", %{"repo" => name} = inputs, call) do
    {folder, state} = read(call, name)
    at = rev(state, inputs)
    path = String.trim(inputs["path"] || "", "/")

    if state["refs"] == %{} do
      %{repo: name, empty: true, clone: address(name), entries: []}
    else
      entries =
        for entry <- String.split(Mirror.git!(folder, ["ls-tree", "-z", "-l", "#{at}:#{path}"]), "\0", trim: true) do
          [about, file] = String.split(entry, "\t", parts: 2)
          [mode, type, sha, size] = String.split(about, ~r/ +/)
          %{name: file, type: type, mode: mode, sha: sha, size: if(size == "-", do: nil, else: String.to_integer(size))}
        end
        |> Enum.sort_by(&{&1.type != "tree", String.downcase(&1.name)})

      readme =
        Enum.find_value(entries, fn entry ->
          if entry.type == "blob" and entry.name =~ ~r/^readme(\.(md|markdown|txt))?$/i and entry.size < @shown do
            %{name: entry.name, text: Mirror.git!(folder, ["cat-file", "blob", entry.sha])}
          end
        end)

      %{
        repo: name,
        ref: at,
        path: path,
        clone: address(name),
        entries: entries,
        readme: readme,
        commit: List.first(commits(folder, at, path, 0, 1)),
        commits: String.to_integer(String.trim(Mirror.git!(folder, ["rev-list", "--count", at])))
      }
    end
  end

  def call("file", %{"repo" => name, "path" => path} = inputs, call) do
    {folder, state} = read(call, name)
    at = "#{rev(state, inputs)}:#{String.trim(path, "/")}"
    size = String.to_integer(String.trim(Mirror.git!(folder, ["cat-file", "-s", at])))

    cond do
      size > @shown ->
        %{path: path, size: size, too_large: true}

      true ->
        data = Mirror.git!(folder, ["cat-file", "blob", at], binary: true)

        if String.valid?(data) and not String.contains?(data, "\0"),
          do: %{path: path, size: size, text: data},
          else: %{path: path, size: size, binary: true}
    end
  end

  def call("log", %{"repo" => name} = inputs, call) do
    {folder, state} = read(call, name)
    page = inputs["page"] || 0

    if state["refs"] == %{} do
      %{commits: [], more: false}
    else
      found = commits(folder, rev(state, inputs), inputs["path"] || "", page * @page, @page + 1)
      %{commits: Enum.take(found, @page), more: length(found) > @page}
    end
  end

  def call("commit", %{"repo" => name, "sha" => sha}, call) do
    {folder, _state} = read(call, name)
    format = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%B%x1e"

    [about, diff] =
      String.split(Mirror.git!(folder, ["show", "--no-color", "--patch", "--stat", format, sha]), "\x1e", parts: 2)

    [sha, parents, author, email, at, message] = String.split(about, "\x1f")

    %{
      sha: sha,
      parents: String.split(parents),
      author: author,
      email: email,
      at: String.to_integer(at) * 1000,
      message: String.trim(message),
      diff: if(byte_size(diff) > @shown, do: binary_part(diff, 0, @shown) <> "\n…", else: String.trim(diff))
    }
  end

  def call("suggestions", %{"repo" => name}, call) do
    {folder, state} = read(call, name)

    for {suggestion, about} <- state["suggestions"] do
      tip = about["commit"] || state["refs"]["refs/suggestions/#{suggestion}"]
      # Its commits are those the branch it's for didn't have, then or now.
      into = if about["state"] == "open", do: state["refs"]["refs/heads/#{about["into"]}"], else: about["from"]
      range = if into, do: ["#{into}..#{tip}"], else: [tip]

      Map.merge(about, %{
        "name" => suggestion,
        "commits" =>
          for line <- lines(Mirror.git!(folder, ["log", "--format=%H%x1f%s%x1f%an%x1f%at" | range])) do
            [sha, subject, author, at] = String.split(line, "\x1f")
            %{sha: sha, subject: subject, author: author, at: String.to_integer(at) * 1000}
          end
      })
    end
    |> Enum.sort_by(&{&1["state"] != "open", -&1["at"]})
  end

  def call("take", %{"repo" => name, "suggestion" => suggestion}, call) do
    repo = Call.reach(call, name, "editor")
    by = call.by || call.for
    [person] = Call.tool(call, "sign-in", "people", %{"users" => [by]})

    {answer, _state} =
      Mirror.change(Mirror.of(call, repo), repo, nil, fn state, folder ->
        about = open!(state, suggestion)
        tip = state["refs"]["refs/suggestions/#{suggestion}"]
        into = "refs/heads/" <> about["into"]
        base = state["refs"][into]

        {merged, made} =
          if base == nil or ancestor?(folder, base, tip),
            do: {tip, []},
            else: merge(folder, base, tip, "Take #{suggestion}: #{about["title"]}", person, by)

        refs = state["refs"] |> Map.put(into, merged) |> Map.delete("refs/suggestions/#{suggestion}")

        taken =
          Map.merge(about, %{"state" => "taken", "commit" => tip, "from" => base, "by_then" => by, "then" => now()})

        {%{state | "refs" => refs, "suggestions" => Map.put(state["suggestions"], suggestion, taken)}, made,
         "Took #{suggestion} into #{about["into"]}, which is at #{String.slice(merged, 0, 7)} now."}
      end)

    pushed(name)
    answer
  end

  def call("decline", %{"repo" => name, "suggestion" => suggestion}, call) do
    repo = writable(call, reach!(call, name))
    by = call.by || call.for
    editor = Call.reaches?(call, name, "editor")

    {answer, _state} =
      Mirror.change(Mirror.of(call, repo), repo, nil, fn state, _folder ->
        about = open!(state, suggestion)

        if not editor and about["by"] != by,
          do: raise("Only whoever pushes to #{name}, or suggested #{suggestion}, declines it.")

        tip = state["refs"]["refs/suggestions/#{suggestion}"]
        from = state["refs"]["refs/heads/#{about["into"]}"]

        declined =
          Map.merge(about, %{"state" => "declined", "commit" => tip, "from" => from, "by_then" => by, "then" => now()})

        refs = Map.delete(state["refs"], "refs/suggestions/#{suggestion}")

        {%{state | "refs" => refs, "suggestions" => Map.put(state["suggestions"], suggestion, declined)}, [],
         "Declined #{suggestion}."}
      end)

    answer
  end

  def call(tool, %{"repo" => name}, call) when tool in ["remove", "restore"] do
    repo = Call.reach(call, name, "owner")
    removed = if tool == "remove", do: now()

    Mirror.change(Mirror.of(call, repo), repo, nil, fn state, _folder ->
      {Map.put(state, "removed", removed), [], :ok}
    end)

    key = "index/" <> name
    {:ok, data, tag} = Artifact.read(index(), key)
    Artifact.replace(index(), key, JSON.encode!(Map.put(JSON.decode!(data), "removed", removed)), tag)

    if removed,
      do: "Removed #{name}. `repos restore` brings it back as it was.",
      else: "Restored #{name}. It's as it was."
  end

  def call("git", %{"repo" => name, "path" => path} = inputs, call) do
    repo = reach!(call, name)
    process = Mirror.of(call, repo)
    body = body(call, inputs)

    {type, answer} =
      case String.split(path, "?", parts: 2) do
        ["info/refs", "service=git-receive-pack"] ->
          {_folder, state} = Mirror.sync(process, repo)
          live!(state, name)
          Wire.advertise(state)

        ["git-receive-pack"] ->
          receive_pack(call, writable(call, repo), process, body)

        [service | _] when service in ["info/refs", "git-upload-pack"] ->
          {folder, state} = Mirror.sync(process, repo)
          live!(state, name)
          Wire.upload_pack(folder, path, body, inputs["protocol"], inputs["encoding"])

        _ ->
          raise "#{path} isn't a request of git's smart HTTP protocol"
      end

    answered(call, type, answer)
  end

  # A push: each ref moves only from where its pusher saw it. Whoever only reads the repo pushes suggestions.
  defp receive_pack(call, repo, process, body) do
    name = Artifact.name(repo)
    editor = Call.reaches?(call, name, "editor")
    by = call.by || call.for
    {commands, pack} = Wire.commands(body || "")
    Logger.info("repos: taking a push of #{byte_size(pack)} bytes to #{name}")
    file = if pack != "", do: Path.join(System.tmp_dir!(), "repos-push-" <> random())
    if file, do: File.write!(file, pack)

    try do
      {results, _state} =
        Mirror.change(process, repo, file, fn state, folder ->
          live!(state, name)

          {changed, results} =
            Enum.reduce(commands, {state, []}, fn {old, new, ref}, {state, results} ->
              case push(state, folder, {old, new, ref}, editor, by) do
                {:ok, state} -> {state, [{ref, :ok} | results]}
                no -> {state, [{ref, no} | results]}
              end
            end)

          {if(changed != state, do: changed), [], Enum.reverse(results)}
        end)

      if Enum.any?(results, &(elem(&1, 1) == :ok)), do: pushed(name)
      Wire.report(results)
    after
      if file, do: File.rm(file)
    end
  end

  defp push(state, folder, {old, new, ref}, editor, by) do
    now = state["refs"][ref] || @zero

    cond do
      not ref?(folder, ref) ->
        {:no, "a repo keeps branches, tags and suggestions (refs/suggestions/<name>)"}

      now == new ->
        {:ok, state}

      String.starts_with?(ref, "refs/suggestions/") ->
        suggest(state, folder, {old, new, ref}, editor, by)

      not editor ->
        {:no,
         "you may read this repo but not push to it: suggest a change with git push origin HEAD:refs/suggestions/<name>"}

      now != old ->
        {:no, "fetch first: it moved since you fetched"}

      new == @zero ->
        {:ok, %{state | "refs" => Map.delete(state["refs"], ref)}}

      not connected?(folder, new) ->
        {:no, "objects are missing from the push"}

      true ->
        {:ok, %{state | "refs" => Map.put(state["refs"], ref, new)}}
    end
  end

  defp suggest(state, folder, {old, new, "refs/suggestions/" <> name = ref}, editor, by) do
    was = state["suggestions"][name]
    now = state["refs"][ref] || @zero

    cond do
      new == @zero ->
        {:no, "a suggestion is declined with `repos decline`, not deleted"}

      was && was["state"] == "open" && was["by"] != by && not editor ->
        {:no, "#{name} is someone else's suggestion: pick another name"}

      now != old ->
        {:no, "fetch first: it moved since you fetched"}

      not connected?(folder, new) ->
        {:no, "objects are missing from the push"}

      true ->
        title = String.trim(Mirror.git!(folder, ["log", "-1", "--format=%s", new]))
        into = if was && was["state"] == "open", do: was["into"], else: state["head"]
        about = %{"by" => by, "into" => into, "title" => title, "at" => now(), "state" => "open"}

        {:ok,
         %{
           state
           | "refs" => Map.put(state["refs"], ref, new),
             "suggestions" => Map.put(state["suggestions"], name, about)
         }}
    end
  end

  defp ref?(_folder, "refs/suggestions/" <> name), do: name =~ ~r/^[a-z0-9][a-z0-9_-]*$/

  defp ref?(folder, ref),
    do: ref =~ ~r{^refs/(heads|tags)/} and elem(Mirror.git(folder, ["check-ref-format", ref]), 1) == 0

  defp connected?(folder, sha),
    do: elem(Mirror.git(folder, ["rev-list", "--objects", "--quiet", sha, "--not", "--all"]), 1) == 0

  defp ancestor?(folder, base, tip), do: elem(Mirror.git(folder, ["merge-base", "--is-ancestor", base, tip]), 1) == 0

  # A merge of `tip` into `base` that git makes without a working tree, as its own pack. Gives the merge and the
  # pack, or raises with the files that conflict.
  defp merge(folder, base, tip, message, person, by) do
    tree =
      case Mirror.git(folder, ["merge-tree", "--write-tree", "--name-only", "--no-messages", base, tip]) do
        {out, 0} ->
          String.trim(out)

        {out, 1} ->
          [_tree | files] = lines(out)

          raise "It doesn't merge cleanly: #{Enum.join(files, ", ")} changed on both sides. Its author fetches, " <>
                  "merges the branch it's for and pushes the suggestion again."
      end

    # Git wants an email, and where signing in has none, whoever took it is said as relation tuples say them.
    {name, email} = {person["called"] || by, person["email"] || by}

    ident = [
      {"GIT_AUTHOR_NAME", name},
      {"GIT_AUTHOR_EMAIL", email},
      {"GIT_COMMITTER_NAME", name},
      {"GIT_COMMITTER_EMAIL", email}
    ]

    merged = String.trim(Mirror.git!(folder, ["commit-tree", tree, "-p", base, "-p", tip, "-m", message], env: ident))

    out =
      Mirror.git!(folder, ["pack-objects", "--revs", "-q", "objects/pack/pack"],
        input: "#{merged}\n^#{base}\n^#{tip}\n"
      )

    {merged, [String.trim(out)]}
  end

  defp open!(state, suggestion) do
    case state["suggestions"][suggestion] do
      %{"state" => "open"} = about -> about
      %{"state" => was} -> raise "#{suggestion} was #{was} already."
      nil -> raise "There's no suggestion #{suggestion}. `repos suggestions` lists them."
    end
  end

  defp commits(folder, at, path, skip, count) do
    args = ["log", "--format=%H%x1f%s%x1f%an%x1f%at", "--skip=#{skip}", "-n", "#{count}", at, "--"]

    for line <- lines(Mirror.git!(folder, args ++ if(path == "", do: [], else: [path]))) do
      [sha, subject, author, at] = String.split(line, "\x1f")
      %{sha: sha, subject: subject, author: author, at: String.to_integer(at) * 1000}
    end
  end

  # The repo as a viewer, its mirror brought up to date: its folder and `repo.json`.
  defp read(call, name) do
    repo = reach!(call, name)
    {folder, state} = Mirror.sync(Mirror.of(call, repo), repo)
    live!(state, name)
    {folder, state}
  end

  # Whoever only reads a repo still suggests changes to it and declines their own, which this plugin keeps in the
  # repo: so it writes there as an editor, once it has decided what they may do.
  defp writable(call, %Artifact{reached: {plugin, asked, "viewer"}} = repo) do
    if Call.reaches?(call, Artifact.name(repo), "editor"),
      do: Call.reach(call, Artifact.name(repo), "editor"),
      else: %{repo | reached: {plugin, asked, "editor"}}
  end

  defp writable(_call, repo), do: repo

  defp reach!(call, name) do
    if not (name =~ @repo),
      do: raise("#{name} isn't a repo's name, as `repos list` gives them: users/alice/made/repos/notes")

    Call.reach(call, name)
  end

  defp live!(%{"removed" => at}, name) when is_integer(at),
    do: raise("#{name} was removed. Its owner brings it back with `repos restore`.")

  defp live!(_state, _name), do: :ok

  # A ref asked for, which git checks; the default branch if none was.
  defp rev(state, inputs) do
    at = inputs["ref"] || state["head"]
    if at =~ ~r/^[A-Za-z0-9._\/-]+$/ and not String.starts_with?(at, "-"), do: at, else: raise("#{at} isn't a ref")
  end

  defp body(call, %{"upload" => upload}) do
    {:ok, data, _tag} = Artifact.read(call.own, upload)
    Artifact.remove(call.own, upload)
    data
  end

  defp body(_call, %{"body" => body}), do: Base.decode64!(body)
  defp body(_call, _inputs), do: nil

  # What git is answered: in the call, or if it's large, kept in the bucket a few minutes for it to fetch, which
  # removes what was kept for answers before.
  defp answered(_call, type, answer) when byte_size(answer) <= @inline, do: {:blob, type, answer}

  defp answered(call, type, answer) do
    now = now()

    for "answers/" <> kept = blob <- Artifact.list(call.own, "answers/"),
        [at | _] = String.split(kept, "-"),
        now - String.to_integer(at) > @answers_kept,
        do: Artifact.remove(call.own, blob)

    blob = "answers/#{now}-#{random()}"
    :ok = Artifact.write(call.own, blob, answer)
    {:file, type, call.own, blob}
  end

  # When a repo was last pushed, for `list`. Two pushes at once each say about the same time, so one that finds
  # the other got there first leaves it.
  defp pushed(name) do
    key = "index/" <> name

    case Artifact.read(index(), key) do
      {:ok, data, tag} ->
        Artifact.replace(index(), key, JSON.encode!(Map.put(JSON.decode!(data), "pushed", now())), tag)

      :absent ->
        Artifact.write(index(), key, JSON.encode!(%{"pushed" => now()}))
    end
  end

  # Every repo, by its name, with when it was last pushed: in this plugin's own artifact for the network, wherever
  # it's added, so there's one list of them for `list` to keep to what each person may read.
  defp index, do: %Artifact{prefix: "network/own/repos/"}

  @doc "The address git clones a repo by, on any device of the network: `one::<repo>`."
  def address(%Artifact{} = repo), do: address(Artifact.name(repo))
  def address(name), do: "one::" <> name

  defp lines(text), do: String.split(text, "\n", trim: true)
  defp now, do: System.system_time(:millisecond)
  defp random, do: Base.encode16(:crypto.strong_rand_bytes(12), case: :lower)
end
