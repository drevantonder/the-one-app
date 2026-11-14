defmodule OneCore.Skills do
  @moduledoc """
  Skills: text for agents on when and how to use a plugin's tools, which an agent reads when it needs it. Each is
  a `SKILL.md` as Agent Skills has it (https://agentskills.io/specification): the skill's `name` and its
  `description`, which says when to read it, as YAML between two lines of `---`, and then the text.

  A plugin's skill is the `SKILL.md` at the root of its folder, named as the plugin is. A plugin offers more as
  folders of `skills/`, each a skill named as its folder: its `SKILL.md`, and any other files in the folder, which
  the skill points to. One Core has one of its own, `building-plugins`, which is `OneCore.Plugin.skill/0`.

  They reach agents as MCP's Skills Extension says (SEP-2640): each file of a skill is a resource,
  `skill://<name>/<path>`, its `SKILL.md` too, and what's listed of a skill is what its `SKILL.md` says of itself,
  with each file's digest and size. Whoever asks sees One Core's own, and those of the plugins that run for them.
  """
  alias OneCore.{Plugin, Plugins}

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
  def list(who), do: for({uri, skill, _file} <- found(who), do: listed(uri, skill))

  @doc """
  A file of a skill by its address, `skill://<name>/<path>`, if whoever a request is for sees that skill: `{:ok,
  text}`, or `:absent`.
  """
  def text(who, uri) do
    with "skill://" <> address <- uri,
         [name, path] <- String.split(address, "/", parts: 2),
         {_, skill, file} <- List.keyfind(found(who), "skill://#{name}/SKILL.md", 0),
         true <- path == "SKILL.md" or Enum.any?(skill.files, &(&1.path == path)) do
      case file do
        :own -> {:ok, Plugin.skill()}
        {artifact, code} -> Plugins.file(artifact, code, skill.at <> path)
      end
    else
      _ -> :absent
    end
  end

  # Each skill by its address, with where its files are: One Core's own, then the plugins' by their names. A name
  # is one skill's, the first found.
  defp found(who) do
    {:ok, own} = read(@own, Plugin.skill())

    plugins =
      for {_name, version} <- Enum.sort(Plugins.running(who)),
          skill <- version.skills,
          do: {skill, {version.artifact, version.code}}

    for {skill, file} <-
          Enum.uniq_by([{Map.merge(own, %{at: "", files: []}), :own} | plugins], fn {skill, _} ->
            skill.frontmatter["name"]
          end),
        do: {"skill://#{skill.frontmatter["name"]}/SKILL.md", skill, file}
  end

  defp listed(uri, skill) do
    base = String.replace_suffix(uri, "SKILL.md", "")
    files = for file <- skill.files, do: %{uri: base <> file.path, digest: file.digest, size: file.size}

    %{
      uri: uri,
      frontmatter: skill.frontmatter,
      resources: [%{uri: uri, digest: skill.digest, size: skill.size} | files]
    }
  end

  defp digest(text), do: "sha256:" <> Base.encode16(:crypto.hash(:sha256, text), case: :lower)
end
