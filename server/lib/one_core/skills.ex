defmodule OneCore.Skills do
  @moduledoc """
  Skills: text for agents on when and how to use a plugin's tools, which an agent reads when it needs it. Each is
  a `SKILL.md` as Agent Skills has it (https://agentskills.io/specification): the skill's `name` and its
  `description`, which says when to read it, as YAML between two lines of `---`, and then the text.

  A plugin's skill is the `SKILL.md` at the root of its folder, named as the plugin is, and a plugin with no such
  file has none. One Core has one of its own, `building-plugins`, which is `OneCore.Plugin.skill/0`.

  They reach agents as MCP's Skills Extension says (SEP-2640): a skill is the resource `skill://<name>/SKILL.md`,
  and what's listed of each is what its file says of itself, with the file's digest and size. Whoever asks sees
  One Core's own, and those of the plugins that run for them.
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
           "SKILL.md names its skill #{inspect(named)}. A plugin's skill has the plugin's name: `name: #{name}`."}

        not (name =~ ~r/^[a-z0-9]+(-[a-z0-9]+)*$/ and byte_size(name) <= 64) ->
          {:error,
           "A skill's name is at most 64 lowercase letters, digits and single hyphens, which #{name} isn't. " <>
             "A plugin by that name has no SKILL.md."}

        not (is_binary(description) and String.trim(description) != "" and String.length(description) <= 1024) ->
          {:error, "SKILL.md's `description` says when an agent should read it, in at most 1024 characters."}

        true ->
          digest = "sha256:" <> Base.encode16(:crypto.hash(:sha256, text), case: :lower)
          {:ok, %{frontmatter: frontmatter, digest: digest, size: byte_size(text)}}
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

  @doc "The skills that whoever a request is for sees, each as the Skills Extension lists one."
  def list(who), do: for({uri, skill, _file} <- found(who), do: listed(uri, skill))

  @doc "The skill at `uri`, as its whole `SKILL.md`, if whoever a request is for sees it: `{:ok, text}`, or `:absent`."
  def text(who, uri) do
    case List.keyfind(found(who), uri, 0) do
      {^uri, _skill, :own} -> {:ok, Plugin.skill()}
      {^uri, _skill, {artifact, code}} -> Plugins.file(artifact, code, "SKILL.md")
      nil -> :absent
    end
  end

  # Each skill by its address, with where its file is: One Core's own, then the plugins' by their names. A name
  # is one skill's, the first found.
  defp found(who) do
    {:ok, own} = read(@own, Plugin.skill())

    plugins =
      for {_name, %{skill: %{} = skill} = version} <- Enum.sort(Plugins.running(who)),
          do: {skill, {version.artifact, version.code}}

    for {skill, file} <- Enum.uniq_by([{own, :own} | plugins], fn {skill, _file} -> skill.frontmatter["name"] end),
        do: {"skill://#{skill.frontmatter["name"]}/SKILL.md", skill, file}
  end

  # A skill here is its one file.
  defp listed(uri, skill) do
    %{uri: uri, frontmatter: skill.frontmatter, resources: [%{uri: uri, digest: skill.digest, size: skill.size}]}
  end
end
