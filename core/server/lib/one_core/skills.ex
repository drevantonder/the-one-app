defmodule OneCore.Skills do
  @moduledoc """
  Skills: text for agents on when and how to use a plugin's tools, which an agent reads when it needs it. Each is
  a `SKILL.md` as Agent Skills has it (https://agentskills.io/specification): the skill's `name` and its
  `description`, which says when to read it, as YAML between two lines of `---`, and then the text.

  A plugin's skill is the `SKILL.md` at the root of its folder, named as the plugin is. A plugin offers more as
  folders of `skills/`, each a skill named as its folder: its `SKILL.md`, and any other files in the folder, which
  the skill points to. One Core has its own: `building-plugins`, which is `OneCore.Plugin.skill/1`,
  `installing-plugins`, which is here, and the UI library's, `ui`, with Solid's beneath it (`OneCore.App`).

  They reach agents as MCP's Skills Extension says (SEP-2640): each file of a skill is a resource,
  `skill://<path>/<file>`, its `SKILL.md` too, and what's listed of a skill is what its `SKILL.md` says of itself,
  with each file's digest and size. Whoever asks sees One Core's own, and those of the plugins that run for them.

  A skill's path ends with its name, as the extension has it, and what comes before is the plugin's, as it's found
  where it runs: the name it's installed by there, then the plugin's own name, as its folder is. So a plugin's skill is
  `skill://<installed by>/<own name>/SKILL.md`, and a skill of its `skills/` is beneath that, as in its files:
  `skill://<installed by>/<own name>/skills/<name>/SKILL.md`. One Core's own are `skill://building-plugins/SKILL.md`,
  `skill://installing-plugins/SKILL.md` and `skill://ui/SKILL.md`, the only skills whose paths are their names alone,
  and Solid's beneath the UI library's, `skill://ui/skills/<name>/SKILL.md`. No two plugins are installed by one name, so no skill's address is
  another's.
  """
  alias OneCore.{App, Plugin, Plugins}
  alias OneCore.Plugins.{Store, Version}

  @installing """
  ---
  name: installing-plugins
  description: Read before installing a plugin from a git repo (`one install <owner>/<repo> --plugin <name>`), or updating one (`one plugins update`). It says how to check it for the person first.
  ---

  A plugin from a git repo is code someone else wrote, and its server code runs on the person's own machines with
  everything they can reach: their files, the network, their other plugins' tools and the Vault items it asks for.
  Nothing sandboxes it. So you check it for them, as a colleague would before running a stranger's script, and
  install it only once they say yes.

  It runs on servers of whoever it's installed for: for the person, a server of their own (set up in Devices),
  and for the network, the network's, where only its owners and installers install it. Someone who has neither
  asks them instead: `home-screen_request --source <owner>/<repo> --name <name> --reason <why>`. An owner who
  approves such a request is answered the command to install it, after reading it as below.

  1. `one install <owner>/<repo> --list` fetches the repo and lists its plugins, and says the folder it's in on this
     device. Pin what you read: `<owner>/<repo>@<commit>`, which `--list` says, so what you install is what you read.
  2. Read the plugin's folder there: every file of `server/` (its tools, what each does with its inputs, what it
     reads and writes, and anything it starts or calls out to: `System.cmd`, `Port`, `:httpc`, `Req`, files outside
     its own artifact), its `edge/` function if it has one (what comes in from the internet, and what it lets in),
     the plugins it `uses` and the Vault items it asks for. `server/nif/` is native code: say so, and read its source.
  3. Tell the person, in a few plain sentences: what it does, what it reaches, anything that surprised you (code that
     sends data somewhere it doesn't need to, that's hidden or obfuscated, that downloads and runs more code, or that
     does more than its `SKILL.md` says), and whether it has native code. Then ask whether to install it.
  4. On their yes, `one install <owner>/<repo>@<commit> --plugin <name>` (`ONE_FOR=groups/<name>` or `network` for a
     group or everyone). It's their own copy, theirs to change, and the network keeps where it came from. In a
     network's repo, `plugins.lock` records it, and `one install` there installs everything it records again.

  An update is read the same way: `one plugins check` says which have moved on, and `git -C <its folder> diff
  <installed commit> <new commit> -- plugins/<name>` shows what changed. Tell the person what's new in what it
  reaches before `one plugins update <name>`. One changed in the network since it was installed is left as it is
  unless they say to overwrite it (`--force`): bring their change across by hand if they want both.

  A plugin with a NIF that isn't built for the network's servers stops with what to build, for which machines, and
  the command: build it from the source you read, then install again.
  """

  @own "building-plugins"

  @doc """
  Reads the skill of whatever is named `name` from its `SKILL.md`. Gives `{:ok, skill}`: its `frontmatter` as JSON
  holds it, and the file's `digest` and `size`. `{:error, why}` says how to put right a file that isn't one.
  """
  def read(name, text) do
    with [_, yaml] <- Regex.run(~r/\A---\r?\n(.*?)\r?\n---(?:\r?\n|\z)/s, text),
         {:ok, %{"name" => named, "description" => description} = frontmatter} <- YamlElixir.read_from_string(yaml) do
      cond do
        named != name ->
          {:error,
           "SKILL.md names its skill #{inspect(named)}. A skill is named as its folder is, the plugin's at its root: `name: #{name}`."}

        # Agent Skills names have no dots, but Solid's skills are named for its version (`solid-2.0`).
        not (name =~ ~r/^[a-z0-9]+([-.][a-z0-9]+)*$/ and byte_size(name) <= 64) ->
          {:error,
           "A skill's name is at most 64 lowercase letters, digits and single hyphens or dots, which #{name} isn't. " <>
             "A plugin by that name has no SKILL.md."}

        not (is_binary(description) and String.trim(description) != "" and String.length(description) <= 1024) ->
          {:error, "SKILL.md's `description` says when an agent should read it, in at most 1024 characters."}

        true ->
          {:ok, %{frontmatter: frontmatter, digest: digest(text), size: byte_size(text)}}
      end
    else
      {:error, %{message: wrong}} ->
        {:error, "SKILL.md's first lines, between its two lines of `---`, aren't YAML: #{wrong}"}

      _ ->
        {:error,
         """
         SKILL.md starts with its skill's name, which is the plugin's, and when an agent should read it:

         ---
         name: #{name}
         description: Read before ...
         ---
         """}
    end
  end

  @doc """
  The skills among a plugin's `files`, the plugin being named `plugin`: each as `read/2` gives it, with `at`, the
  folder it's in, and its other `files`, each with its `path` in that folder, `digest` and `size`. `{:error, why}`
  for a `SKILL.md` that isn't one, or a folder of `skills/` with none.
  """
  def found(plugin, files) do
    files
    |> Enum.group_by(fn {path, _} -> List.first(Regex.run(~r{^skills/[^/]+/}, path) || [""]) end)
    |> Enum.sort()
    |> Enum.reduce_while({:ok, []}, fn {at, in_folder}, {:ok, found} ->
      {skill, others} = Map.pop(Map.new(in_folder), at <> "SKILL.md")
      name = if at == "", do: plugin, else: at |> String.trim_trailing("/") |> Path.basename()

      case skill && read(name, skill) do
        nil ->
          {:halt, {:error, "#{at} is a skill's folder, so it has a SKILL.md"}}

        {:ok, read} ->
          others =
            for {path, text} <- Enum.sort(others),
                do: %{path: String.replace_prefix(path, at, ""), digest: digest(text), size: byte_size(text)}

          {:cont, {:ok, found ++ [Map.merge(read, %{at: at, files: others})]}}

        {:error, why} ->
          {:halt, {:error, why}}
      end
    end)
  end

  @doc "The skills that whoever a request is for sees, each as the Skills Extension lists one."
  def list(who), do: for({path, skill, _file} <- found(who), do: listed(path, skill))

  @doc """
  A file of a skill by its address, `skill://<path>/<file>`, if whoever a request is for sees that skill: `{:ok,
  text}`, or `:absent`.
  """
  def text(who, uri) do
    found =
      for {path, skill, file} <- found(who),
          at <- ["SKILL.md" | Enum.map(skill.files, & &1.path)],
          uri == "skill://#{path}/#{at}",
          do: {skill, file, at}

    case found do
      [] -> :absent
      [{_skill, :own, _at}] -> {:ok, Plugin.skill(Plugins.running(who))}
      [{_skill, {:own, text}, _at}] -> {:ok, text}
      [{skill, :ui, at}] -> {:ok, App.library().files[skill.at <> at]}
      [{skill, {artifact, code}, at}] -> Store.file(artifact, code, skill.at <> at)
    end
  end

  # Each skill by its path, with where its files are: One Core's own, then the plugins' by the names they're installed by.
  defp found(who) do
    {:ok, own} = read(@own, Plugin.skill(Plugins.running(who)))
    {:ok, installing} = read("installing-plugins", @installing)

    plugins =
      for {installed, version} <- Enum.sort(Plugins.running(who)), skill <- Version.skills(version) do
        path = Enum.join([installed, Path.basename(version.artifact) | Path.split(skill.at)], "/")
        {path, skill, {version.artifact, version.code}}
      end

    ui = for skill <- App.library().skills, do: {Enum.join(["ui" | Path.split(skill.at)], "/"), skill, :ui}

    [
      {@own, Map.merge(own, %{at: "", files: []}), :own},
      {"installing-plugins", Map.merge(installing, %{at: "", files: []}), {:own, @installing}}
      | ui ++ plugins
    ]
  end

  defp listed(path, skill) do
    uri = "skill://#{path}/SKILL.md"
    files = for file <- skill.files, do: %{uri: "skill://#{path}/#{file.path}", digest: file.digest, size: file.size}

    %{
      uri: uri,
      frontmatter: skill.frontmatter,
      resources: [%{uri: uri, digest: skill.digest, size: skill.size} | files]
    }
  end

  defp digest(text), do: "sha256:" <> Base.encode16(:crypto.hash(:sha256, text), case: :lower)
end
