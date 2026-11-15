defmodule OneCore.Skills do
  @moduledoc """
  Skills: text for agents on when and how to use a plugin's tools, which an agent reads when it needs it. Each is
  a `SKILL.md` as Agent Skills has it (https://agentskills.io/specification): the skill's `name` and its
  `description`, which says when to read it, as YAML between two lines of `---`, and then the text.

  A plugin's skill is the `SKILL.md` at the root of its folder, named as the plugin is. A plugin offers more as
  folders of `skills/`, each a skill named as its folder: its `SKILL.md`, and any other files in the folder, which
  the skill points to. One Core has one of its own, `building-plugins`, which is `OneCore.Plugin.skill/0`.

  They reach agents as MCP's Skills Extension says (SEP-2640): each file of a skill is a resource,
  `skill://<path>/<file>`, its `SKILL.md` too, and what's listed of a skill is what its `SKILL.md` says of itself,
  with each file's digest and size. Whoever asks sees One Core's own, and those of the plugins that run for them.

  A skill's path ends with its name, as the extension has it, and what comes before is the plugin's, as it's found
  where it runs: the name it's added by there, then the plugin's own name, as its folder is. So a plugin's skill is
  `skill://<added by>/<own name>/SKILL.md`, and a skill of its `skills/` is beneath that, as in its files:
  `skill://<added by>/<own name>/skills/<name>/SKILL.md`. One Core's own is `skill://building-plugins/SKILL.md`,
  the only skill whose path is its name alone. No two plugins are added by one name, so no skill's address is
  another's.
  """
  alias OneCore.{Plugin, Plugins}
  alias OneCore.Plugins.{Kept, Version}

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
      [{_skill, :own, _at}] -> {:ok, Plugin.skill()}
      [{skill, {artifact, code}, at}] -> Kept.file(artifact, code, skill.at <> at)
    end
  end

  # Each skill by its path, with where its files are: One Core's own, then the plugins' by the names they're added by.
  defp found(who) do
    {:ok, own} = read(@own, Plugin.skill())

    plugins =
      for {added, version} <- Enum.sort(Plugins.running(who)), skill <- Version.skills(version) do
        path = Enum.join([added, Path.basename(version.artifact) | Path.split(skill.at)], "/")
        {path, skill, {version.artifact, version.code}}
      end

    [{@own, Map.merge(own, %{at: "", files: []}), :own} | plugins]
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
