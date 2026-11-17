defmodule OneCore.Plugins.Git do
  @moduledoc """
  A plugin's git source, as a server reads it to find a newer version: a copy of the repo kept in the server's folder,
  `git/<the source's hash>`, made with `git clone --mirror --filter=blob:none`, so a fetch brings only the commits and
  the files a server reads. It's git itself that's run, signed in as the machine's user is, so a private repo is
  fetched where that user may fetch it.
  """
  alias OneCore.Server

  @doc "Fetches `source` (a URL, `owner/repo` on GitHub, or a path): `{:ok, copy}`, or `{:error, why}`."
  def fetch(source) do
    copy = Path.join([Server.home(), "git", Base.encode16(:crypto.hash(:sha256, source), case: :lower)])

    if File.dir?(copy) do
      with {:ok, _} <- git(copy, ["fetch", "--quiet", "--prune", "--force"]), do: {:ok, copy}
    else
      File.mkdir_p!(Path.dirname(copy))

      with {:ok, _} <- git(Path.dirname(copy), ["clone", "--quiet", "--mirror", "--filter=blob:none", url(source), copy]),
           do: {:ok, copy}
    end
  end

  # What git fetches a source from: `owner/repo` is on GitHub, as `one install` reads it.
  defp url(source) do
    if source =~ ~r{^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$} and not File.exists?(source),
      do: "https://github.com/#{source}.git",
      else: source
  end

  @doc "The commit `ref` is in a copy, or what the repo has by default where it's `nil`."
  def commit(copy, ref) do
    git(copy, ["rev-parse", "--verify", "--quiet", "#{ref || "HEAD"}^{commit}"])
  end

  @doc "Whether `ancestor` is `commit` or comes before it, so moving to `commit` loses nothing of it."
  def ancestor?(copy, ancestor, commit), do: match?({:ok, _}, git(copy, ["merge-base", "--is-ancestor", ancestor, commit]))

  @doc "The tree of `path` at `commit`, which is the same for two commits only where its files are."
  def tree(copy, commit, path), do: git(copy, ["rev-parse", "--verify", "--quiet", "#{commit}:#{path}"])

  @doc """
  The commits after `from` up to `to` that changed `path`, newest first, each with its `commit`, `subject`,
  `author` and `date`: what an update brings, for whoever decides whether to take it.
  """
  def commits(copy, from, to, path) do
    range = if from && ancestor?(copy, from, to), do: "#{from}..#{to}", else: to

    with {:ok, log} <- git(copy, ["log", "--max-count=50", "--format=%H%x1f%s%x1f%an%x1f%aI", range, "--", path]) do
      {:ok,
       for line <- String.split(log, "\n", trim: true), [commit, subject, author, date] = String.split(line, "\x1f") do
         %{"commit" => commit, "subject" => subject, "author" => author, "date" => date}
       end}
    end
  end

  @doc "The files of `path` at `commit`, by their paths in it: `{:ok, files}`, or `{:error, why}`."
  def files(copy, commit, path) do
    with {:ok, tar} <- git(copy, ["archive", "--format=tar", commit, path], false),
         {:ok, entries} <- :erl_tar.extract({:binary, tar}, [:memory]) do
      {:ok, Map.new(entries, fn {name, data} -> {String.replace_prefix(to_string(name), path <> "/", ""), data} end)}
    else
      {:error, why} when is_binary(why) -> {:error, why}
      {:error, why} -> {:error, inspect(why)}
    end
  end

  defp git(folder, args, trim \\ true) do
    # An archive is its output alone; anything else is said with what git says.
    case System.cmd("git", ["-C", folder | args], stderr_to_stdout: trim, env: [{"GIT_TERMINAL_PROMPT", "0"}]) do
      {out, 0} -> {:ok, if(trim, do: String.trim(out), else: out)}
      {out, _} -> {:error, "git #{hd(args)} said: #{String.trim(out)}"}
    end
  end
end
