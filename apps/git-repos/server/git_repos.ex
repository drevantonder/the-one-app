defmodule GitRepos do
  @moduledoc """
  Git Repos: git hosting for people and their agents. A repo is an artifact, a person's, a group's or the network's,
  shared as anything is with relation tuples: a `viewer` clones and fetches it and opens pull requests, and an
  `editor` pushes to it and merges or closes pull requests. What it is in the bucket is `GitRepos.Mirror`'s.

  Git reaches a repo from a person's devices through `one`, which is git's remote helper for `one::` addresses
  (`git clone one::users/alice/made/git-repos/notes`): it runs git's own HTTP transport, and carries each of its
  requests to a server as a call of the tool `git`, signed as every call of the device is. So git needs no
  password, and works wherever the device reaches a server.

  A pull request is a push to `refs/pull/<name>` by anyone who may read the repo. The repo's editors
  merge it into the branch it's for, or close it.
  """
  @behaviour OneCore.App
  require Logger
  alias OneCore.{Artifact, Call}
  alias GitRepos.{Mirror, Wire}

  @zero Mirror.zero()
  @repo ~r{^(users/[^/]+|groups/[^/]+|network)/made/git-repos/[a-z0-9][a-z0-9_-]*$}
  # Larger than this, what git sends or is answered goes through the bucket instead of in the call.
  @inline 4_194_304
  # How long an answer kept in the bucket for git to fetch is kept.
  @responses_ttl :timer.minutes(10)
  # The most of a file that's shown, and of a commit's changes.
  @shown 1_048_576
  @page 25
  # A repo that's removed goes to Recently Deleted, which keeps it for 30 days (`SoftDelete`), and tells this app what became
  # of it, so `list` leaves out what's in Recently Deleted.
  @impl true
  def icon, do: "client/icon.svg"

  @impl true
  def requires, do: %{"core" => ">= 0.32.167"}

  # Every repo is listed in one index, the network's, which a call reads and writes whoever it's for, so what it
  # keeps is whoever Git Repos is installed for's, and it runs on their servers.
  @impl true
  def own, do: :installed

  @impl true
  def hooks, do: ["soft-delete.moved"]

  @impl true
  def entrypoints, do: [%{page: "client/index.html", runtime_version: 4}]

  @impl true
  def tools do
    repo = %{type: "string", description: "The repo, as `list` names it: users/alice/made/git-repos/notes"}
    ref = %{type: "string", description: "A branch, a tag or a commit. Left out, the repo's default branch"}
    path = %{type: "string", description: "A path in the repo, like src/app.js. Left out, the top"}
    pull_request = %{type: "string", description: "The pull_request's name, as `pull_requests` lists them"}
    clone = %{type: "string", description: "Its address, to clone it by"}
    ms = &%{type: "integer", description: "#{&1}, in milliseconds since 1970"}

    commit = %{
      type: "object",
      properties: %{
        sha: %{type: "string"},
        subject: %{type: "string"},
        author: %{type: "string"},
        at: %{type: "integer", description: "When, in milliseconds since 1970"}
      },
      required: ["sha", "subject", "author", "at"]
    }

    only = fn properties, required -> %{type: "object", properties: properties, required: required} end

    [
      %{
        name: "create",
        description:
          "Creates an empty repo, yours, or a group's or the network's when asked for them, and answers how to clone it",
        inputs:
          only.(%{name: %{type: "string", description: "Lowercase letters, digits, - and _, like notes"}}, ["name"]),
        outputs: only.(%{repo: repo, clone: clone}, ["repo", "clone"])
      },
      %{
        name: "list",
        read_only: true,
        description:
          "Every repo you may read, the latest pushed first: its name, whose it is, when it was last pushed, and whether you push to it",
        inputs: only.(%{}, []),
        outputs: only.(
          %{
            repos: %{
              type: "array",
              description: "The latest pushed first",
              items:
                only.(
                  %{
                    repo: repo,
                    whose: %{type: "string", description: "users/<name>, groups/<name> or network"},
                    pushed: ms.("When it was last pushed"),
                    pushes: %{type: "boolean", description: "Whether you push to it"},
                    clone: clone
                  },
                  ["repo", "whose", "pushed", "pushes", "clone"]
                )
            }
          },
          ["repos"]
        )
      },
      %{
        name: "branches",
        read_only: true,
        description:
          "A repo's branches, each with its newest commit, the default first, and its address to clone it by",
        inputs: only.(%{repo: repo}, ["repo"]),
        outputs: only.(
          %{
            repo: repo,
            head: %{type: "string", description: "The default branch"},
            whose: %{type: "string"},
            clone: clone,
            pushes: %{type: "boolean", description: "Whether you push to it"},
            owns: %{type: "boolean", description: "Whether you own it"},
            branches: %{
              type: "array",
              description: "The default first",
              items: Map.update!(commit, :properties, &Map.put(&1, :branch, %{type: "string"}))
            }
          },
          ["repo", "head", "whose", "clone", "pushes", "owns", "branches"]
        )
      },
      %{
        name: "tree",
        read_only: true,
        description:
          "A folder of a repo at a ref: its files and folders, folders first, the newest commit that touched it, and its README's text",
        inputs: only.(%{repo: repo, ref: ref, path: path}, ["repo"]),
        outputs: only.(
          %{
            repo: repo,
            empty: %{type: "boolean", description: "Only there, and true, for a repo nothing was pushed to"},
            ref: %{type: "string"},
            path: %{type: "string"},
            clone: clone,
            entries: %{
              type: "array",
              description: "Folders first",
              items:
                only.(
                  %{
                    name: %{type: "string"},
                    type: %{type: "string", description: "tree for a folder, blob for a file, commit for a submodule"},
                    mode: %{type: "string"},
                    sha: %{type: "string"},
                    size: %{type: ["integer", "null"], description: "A file's, in bytes"}
                  },
                  ["name", "type", "mode", "sha", "size"]
                )
            },
            readme: %{
              type: ["object", "null"],
              properties: %{name: %{type: "string"}, text: %{type: "string"}},
              description: "Its README, if it has one"
            },
            commit: %{anyOf: [commit, %{type: "null"}], description: "The newest commit that touched it"},
            commits: %{type: "integer", description: "How many commits the ref has"}
          },
          ["repo", "clone", "entries"]
        )
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
          ]),
        outputs: only.(
          %{
            path: %{type: "string"},
            size: %{type: "integer", description: "In bytes"},
            text: %{type: "string", description: "Its text, where it's text and not too large"},
            binary: %{type: "boolean", description: "True where it isn't text"},
            too_large: %{type: "boolean", description: "True where it's too large to show: clone the repo"}
          },
          ["path", "size"]
        )
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
          ),
        outputs: only.(
          %{
            commits: %{type: "array", items: commit, description: "Newest first"},
            more: %{type: "boolean", description: "Whether there's another page"}
          },
          ["commits", "more"]
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
          ]),
        outputs: only.(
          %{
            sha: %{type: "string"},
            parents: %{type: "array", items: %{type: "string"}},
            author: %{type: "string"},
            email: %{type: "string"},
            at: ms.("When"),
            message: %{type: "string"},
            diff: %{type: "string", description: "What it changed, cut short where it's long"}
          },
          ["sha", "parents", "author", "email", "at", "message", "diff"]
        )
      },
      %{
        name: "pull_requests",
        read_only: true,
        description:
          "A repo's pull requests, open ones first: each one's name, who opened it, the branch it's for, its title, its commits, and whether it was merged or closed",
        inputs: only.(%{repo: repo}, ["repo"]),
        outputs: only.(
          %{
            pull_requests: %{
              type: "array",
              description: "Open ones first",
              items:
                only.(
                  %{
                    name: pull_request,
                    by: %{type: "string", description: "Who opened it"},
                    into: %{type: "string", description: "The branch it's for"},
                    title: %{type: "string"},
                    at: ms.("When it was opened"),
                    state: %{type: "string", enum: ~w(open merged closed)},
                    by_then: %{type: "string", description: "Who merged or closed it"},
                    then: ms.("When it was merged or closed"),
                    commits: %{type: "array", items: commit}
                  },
                  ["name", "by", "into", "title", "at", "state", "commits"]
                )
            }
          },
          ["pull_requests"]
        )
      },
      %{
        name: "merge",
        description:
          "Merges a pull request into the branch it's for: a fast-forward where it can be, or else a merge. Only someone who pushes to the repo merges one",
        inputs: only.(%{repo: repo, pull_request: pull_request}, ["repo", "pull_request"]),
        outputs: only.(
          %{
            pull_request: pull_request,
            into: %{type: "string", description: "The branch it was merged into"},
            sha: %{type: "string", description: "The commit that branch is at now"}
          },
          ["pull_request", "into", "sha"]
        )
      },
      %{
        name: "close",
        description:
          "Closes a pull request: it's no longer open, and nothing of it reaches a branch. Whoever pushes to the repo closes one, and so does whoever opened it",
        inputs: only.(%{repo: repo, pull_request: pull_request}, ["repo", "pull_request"]),
        outputs: only.(%{pull_request: pull_request}, ["pull_request"])
      },
      %{
        name: "remove",
        description:
          "Removes a repo you own: nobody reads, clones or pushes to it, until `restore` brings it back as it was",
        inputs: only.(%{repo: repo}, ["repo"]),
        outputs: only.(%{repo: repo}, ["repo"])
      },
      %{
        name: "restore",
        description: "Brings back a repo you own that was removed in the last 30 days, as it was",
        inputs: only.(%{repo: repo}, ["repo"]),
        outputs: only.(%{repo: repo}, ["repo"])
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
          ),
        outputs: only.(
          %{mimeType: %{type: "string", description: "Git's Content-Type"}, size: %{type: "integer"}},
          ["mimeType"]
        )
      }
    ]
  end

  @impl true
  def hook("soft-delete.moved", %{"artifact" => name, "to" => to} = about, _call) do
    if name =~ @repo, do: indexed(name, to, about["as"] || name)
    nil
  end

  @impl true
  def call("create", %{"name" => name}, call) do
    if not (name =~ ~r/^[a-z0-9][a-z0-9_-]{0,63}$/),
      do: raise("A repo's name is lowercase letters, digits, - and _, starting with a letter or digit")

    repo = Call.make(call, name)
    now = System.system_time(:millisecond)

    state = %{
      "head" => "main",
      "refs" => %{},
      "packs" => [],
      "retired" => [],
      "pull_requests" => %{},
      "made" => now,
      "by" => call.actor || call.subject
    }

    if Artifact.write(repo, "repo.json", JSON.encode!(state)) == :exists,
      do: raise("#{Artifact.name(repo)} is a repo already. Pick another name.")

    # One deleted at this name is listed as this one now, and one restored goes beside it.
    Artifact.update(index(), "index/" <> Artifact.name(repo), fn _ -> JSON.encode!(%{"pushed" => now}) end)

    %{repo: Artifact.name(repo), clone: address(repo)}
  end

  def call("list", _inputs, call) do
    names = for "index/" <> name <- Artifact.list(index(), "index/"), Call.reaches?(call, name), do: name

    repos =
      names
      |> Enum.zip(Artifact.read_all(index(), Enum.map(names, &("index/" <> &1))))
      |> Enum.flat_map(fn {name, {:ok, data, _tag}} ->
        stored = JSON.decode!(data)

        # A repo restored while Git Repos wasn't installed wasn't heard of, so one marked removed is looked for.
        if stored["purging"] || (stored["removed"] && OneCore.Bucket.size(name <> "/repo.json") == :absent),
          do: [],
          else: [
            %{
              repo: name,
              whose: name |> String.split("/made/") |> hd(),
              pushed: stored["pushed"],
              pushes: Call.reaches?(call, name, "editor"),
              clone: address(name)
            }
          ]
      end)
      |> Enum.sort_by(& &1.pushed, :desc)

    %{repos: repos}
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

  def call("pull_requests", %{"repo" => name}, call) do
    {folder, state} = read(call, name)

    pull_requests =
      for {pull_request, about} <- state["pull_requests"] do
        tip = about["commit"] || state["refs"]["refs/pull/#{pull_request}"]
        # Its commits are those the branch it's for didn't have, then or now.
        into = if about["state"] == "open", do: state["refs"]["refs/heads/#{about["into"]}"], else: about["from"]
        range = if into, do: ["#{into}..#{tip}"], else: [tip]

        Map.merge(about, %{
          "name" => pull_request,
          "commits" =>
            for line <- lines(Mirror.git!(folder, ["log", "--format=%H%x1f%s%x1f%an%x1f%at" | range])) do
              [sha, subject, author, at] = String.split(line, "\x1f")
              %{sha: sha, subject: subject, author: author, at: String.to_integer(at) * 1000}
            end
        })
      end
      |> Enum.sort_by(&{&1["state"] != "open", -&1["at"]})

    %{pull_requests: pull_requests}
  end

  def call("merge", %{"repo" => name, "pull_request" => pull_request}, call) do
    repo = Call.reach(call, name, "editor")
    by = call.actor || call.subject
    %{"people" => [person]} = Call.tool(call, "sign-in", "people", %{"users" => [by]})

    {result, _state} =
      Mirror.change(Mirror.of(call, repo), repo, nil, fn state, folder ->
        about = open!(state, pull_request)
        tip = state["refs"]["refs/pull/#{pull_request}"]
        into = "refs/heads/" <> about["into"]
        base = state["refs"][into]

        {merged, merged_packs} =
          if base == nil or ancestor?(folder, base, tip),
            do: {tip, []},
            else: merge(folder, base, tip, "Take #{pull_request}: #{about["title"]}", person, by)

        refs = state["refs"] |> Map.put(into, merged) |> Map.delete("refs/pull/#{pull_request}")

        merged =
          Map.merge(about, %{"state" => "merged", "commit" => tip, "from" => base, "by_then" => by, "then" => now()})

        {%{state | "refs" => refs, "pull_requests" => Map.put(state["pull_requests"], pull_request, merged)}, merged_packs,
         %{pull_request: pull_request, into: about["into"], sha: refs[into]}}
      end)

    pushed(name)
    result
  end

  def call("close", %{"repo" => name, "pull_request" => pull_request}, call) do
    repo = writable(call, reach!(call, name))
    by = call.actor || call.subject
    editor = Call.reaches?(call, name, "editor")

    {result, _state} =
      Mirror.change(Mirror.of(call, repo), repo, nil, fn state, _folder ->
        about = open!(state, pull_request)

        if not editor and about["by"] != by,
          do: raise("Only whoever pushes to #{name}, or opened #{pull_request}, closes it.")

        tip = state["refs"]["refs/pull/#{pull_request}"]
        from = state["refs"]["refs/heads/#{about["into"]}"]

        closed =
          Map.merge(about, %{"state" => "closed", "commit" => tip, "from" => from, "by_then" => by, "then" => now()})

        refs = Map.delete(state["refs"], "refs/pull/#{pull_request}")

        {%{state | "refs" => refs, "pull_requests" => Map.put(state["pull_requests"], pull_request, closed)}, [],
         %{pull_request: pull_request}}
      end)

    result
  end

  def call("remove", %{"repo" => name}, call) do
    repo!(name)
    Call.tool(call, "soft-delete", "delete", %{"artifact" => name})
    indexed(name, "deleted", name)
    %{repo: name}
  end

  # Restored beside a repo made at its name since, it's at another name, notes-2, which it answers with.
  def call("restore", %{"repo" => name}, call) do
    repo!(name)

    item =
      Enum.find_value(Call.tool(call, "soft-delete", "list", %{})["deleted"], fn item ->
        if item["artifacts"] == [name], do: item["item"]
      end) || raise "#{name} isn't in Recently Deleted."

    [%{"as" => as}] = Call.tool(call, "soft-delete", "restore", %{"item" => item})["restored"]
    indexed(name, "active", as)
    %{repo: as}
  end

  def call("git", %{"repo" => name, "path" => path} = inputs, call) do
    repo = reach!(call, name)
    process = Mirror.of(call, repo)
    body = body(call, inputs)

    {type, result} =
      case String.split(path, "?", parts: 2) do
        ["info/refs", "service=git-receive-pack"] ->
          {_folder, state} = Mirror.sync(process, repo)
          Wire.advertise(state)

        ["git-receive-pack"] ->
          receive_pack(call, writable(call, repo), process, body)

        [service | _] when service in ["info/refs", "git-upload-pack"] ->
          {folder, _state} = Mirror.sync(process, repo)
          Wire.upload_pack(folder, path, body, inputs["protocol"], inputs["encoding"])

        _ ->
          raise "#{path} isn't a request of git's smart HTTP protocol"
      end

    response_for(call, type, result)
  end

  # A push: each ref moves only from where its pusher saw it. Whoever only reads the repo pushes pull_requests.
  defp receive_pack(call, repo, process, body) do
    name = Artifact.name(repo)
    editor = Call.reaches?(call, name, "editor")
    by = call.actor || call.subject
    {commands, pack} = Wire.commands(body || "")
    Logger.info("git-repos: taking a push of #{byte_size(pack)} bytes to #{name}")
    file = if pack != "", do: Path.join(System.tmp_dir!(), "git-repos-push-" <> random())
    if file, do: File.write!(file, pack)

    try do
      {results, _state} =
        Mirror.change(process, repo, file, fn state, folder ->

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
        {:no, "a repo keeps branches, tags and pull_requests (refs/pull/<name>)"}

      now == new ->
        {:ok, state}

      String.starts_with?(ref, "refs/pull/") ->
        pull_request(state, folder, {old, new, ref}, editor, by)

      not editor ->
        {:no,
         "you may read this repo but not push to it: open a pull request with git push origin HEAD:refs/pull/<name>"}

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

  defp pull_request(state, folder, {old, new, "refs/pull/" <> name = ref}, editor, by) do
    was = state["pull_requests"][name]
    now = state["refs"][ref] || @zero

    cond do
      new == @zero ->
        {:no, "a pull request is closed with `repos close`, not deleted"}

      was && was["state"] == "open" && was["by"] != by && not editor ->
        {:no, "#{name} is someone else's pull_request: pick another name"}

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
             "pull_requests" => Map.put(state["pull_requests"], name, about)
         }}
    end
  end

  defp ref?(_folder, "refs/pull/" <> name), do: name =~ ~r/^[a-z0-9][a-z0-9_-]*$/

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
                  "merges the branch it's for and pushes the pull_request again."
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

  defp open!(state, pull_request) do
    case state["pull_requests"][pull_request] do
      %{"state" => "open"} = about -> about
      %{"state" => was} -> raise "#{pull_request} was #{was} already."
      nil -> raise "There's no pull request #{pull_request}.\n\n`repos pull_requests` lists them."
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
    Mirror.sync(Mirror.of(call, repo), repo)
  end

  # Whoever only reads a repo still opens pull requests on it and closes their own, which this app keeps in the
  # repo: so it writes there as an editor, once it has decided what they may do.
  defp writable(call, %Artifact{reached: {app, principal, "viewer"}} = repo) do
    if Call.reaches?(call, Artifact.name(repo), "editor"),
      do: Call.reach(call, Artifact.name(repo), "editor"),
      else: %{repo | reached: {app, principal, "editor"}}
  end

  defp writable(_call, repo), do: repo

  defp reach!(call, name) do
    repo!(name)
    Call.reach(call, name)
  end

  defp repo!(name) do
    if not (name =~ @repo),
      do: raise("#{name} isn't a repo's name.\n\nA repo's name is as `repos list` gives them: users/alice/made/git-repos/notes")
  end

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
  defp response_for(_call, type, result) when byte_size(result) <= @inline, do: {:blob, type, result}

  defp response_for(call, type, result) do
    now = now()

    for "answers/" <> name = blob <- Artifact.list(call.own, "answers/"),
        [at | _] = String.split(name, "-"),
        now - String.to_integer(at) > @responses_ttl,
        do: Artifact.remove(call.own, blob)

    blob = "answers/#{now}-#{random()}"
    :ok = Artifact.write(call.own, blob, result)
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

  # A repo in Recently Deleted stays in the index, saying when it was removed, until it's purged or a repo is made at
  # its name. One restored beside a repo made at its name since is indexed at the name it's at now, `as`.
  defp indexed(name, "active", as) when as != name,
    do: Artifact.update(index(), "index/" <> as, &(&1 || JSON.encode!(%{"pushed" => now()})))

  defp indexed(name, to, _as) do
    key = "index/" <> name

    with {:ok, data, tag} <- Artifact.read(index(), key) do
      stored = JSON.decode!(data)

      case to do
        "deleted" ->
          Artifact.replace(index(), key, JSON.encode!(stored |> Map.put_new("removed", now()) |> Map.delete("purging")), tag)

        # It's at its name again while it's purged, and not listed.
        "purging" ->
          Artifact.replace(index(), key, JSON.encode!(stored |> Map.put_new("removed", now()) |> Map.put("purging", true)), tag)

        "active" ->
          Artifact.replace(index(), key, JSON.encode!(Map.delete(stored, "removed")), tag)

        "purged" when is_map_key(stored, "removed") ->
          Artifact.remove(index(), key)

        "purged" ->
          :ok
      end
    end
  end

  # Every repo, by its name, with when it was last pushed: in this app's own artifact for the network, wherever
  # it's added, so there's one list of them for `list` to keep to what each person may read.
  defp index, do: %Artifact{prefix: "network/own/repos/"}

  @doc "The address git clones a repo by, on any device of the network: `one::<repo>`."
  def address(%Artifact{} = repo), do: address(Artifact.name(repo))
  def address(name), do: "one::" <> name

  defp lines(text), do: String.split(text, "\n", trim: true)
  defp now, do: System.system_time(:millisecond)
  defp random, do: Base.encode16(:crypto.strong_rand_bytes(12), case: :lower)
end
