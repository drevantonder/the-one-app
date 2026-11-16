defmodule OneCore.Search do
  @moduledoc """
  MCP's `search`, and `one search`: the tools for a task across One Core's own and every plugin whoever asks has,
  best first, each with the exact way to call it, what it does and whether it only reads, then the skill of each
  plugin they're in. So the long skills come with a search rather than in MCP's instructions. It also offers the
  plugins they could install that match, from what anyone has deployed, and gives a skill's text by its name, for a
  client that doesn't read MCP's resources.

  The call is written as whoever asked calls tools (`_meta.mode`): an MCP tool (`gmail_search { query: string }`),
  a line of a code mode script (`tools.gmail.search({ query: string })`), or a command (`one gmail search --query
  <string>`).
  """
  alias OneCore.{Bucket, Plugins, Skills, Tools}
  alias OneCore.Plugins.{Deploy, Version}

  @doc "The tool, as MCP lists it."
  def tool do
    %{
      name: "search",
      description: """
      Finds the tools for a task across every plugin: give a few words, like "unread email" or "add an event". \
      Answers the best matches, each with its exact call and what it does, then each one's plugin's skill, and \
      the plugins you could install that do it. Search before guessing, to learn a plugin, and before saying you \
      can't. A skill's name, like building-plugins, gives its text.\
      """,
      inputSchema: %{
        type: "object",
        properties: %{
          query: %{type: "string", description: "A few words for the task, a plugin's name, or a skill's name"},
          limit: %{type: "integer", description: "How many tools at most, 8 if left out"}
        },
        required: ["query"]
      },
      annotations: %{readOnlyHint: true}
    }
  end

  # About how much of an answer the matches may take, in characters, before their descriptions are cut short.
  @matches 6_000

  @filler ~w(a an the my me i to for of in on at and or with from it is are all some how do can what this that please
             using use here there else which any your you be was has have just also)

  @doc """
  The answer to `query` for `who`, at most `limit` tools, its calls written for `mode`: `:mcp`, `:code` or `:cli`.
  """
  def answer(query, limit, who, mode) do
    running = Plugins.running(who)
    # What anyone has deployed is read from the bucket, meanwhile.
    deployed = Task.async(&Deploy.deployed/0)

    case skill(String.trim(query), who, running) do
      {:ok, text} ->
        Task.shutdown(deployed, :brutal_kill)
        text

      :none ->
        found(words(query), limit, running, Task.await(deployed, 30_000), mode)
    end
  end

  # A skill by its name, as its SKILL.md names it, or by its address: One Core's own and those of a plugin's
  # `skills/`. A plugin's own skill is named as the plugin, which a search for its tools gives with them.
  defp skill(query, who, running) do
    own = for {name, version} <- running, do: [name, Path.basename(version.artifact)]

    found =
      Enum.find(Skills.list(who), fn %{uri: uri, frontmatter: %{"name" => name}} ->
        uri == query or (name == query and query not in List.flatten(own))
      end)

    case found && Skills.text(who, found.uri) do
      {:ok, text} -> {:ok, text}
      _ -> :none
    end
  end

  defp found(query, limit, running, deployed, mode) do
    plugins = Enum.sort(running)
    tools = for tool <- Tools.tools(), do: {nil, nil, tool}
    tools = tools ++ for {name, version} <- plugins, tool <- version.tools, do: {name, version, tool}
    about = Map.new(plugins, fn {name, version} -> {name, words(described(version))} end)
    enough = fn covers -> covers > 0 and covers * 2 > max(length(query), 1) end

    scored =
      for {{plugin, version, tool}, at} <- Enum.with_index(tools),
          {score, covers, by_name} = score(plugin, version, tool, about[plugin] || [], query),
          enough.(covers),
          do: {score, covers, by_name, at, plugin, version, tool}

    # Those with as many of the words as the best, or one fewer with one in their name, best first: fewer is a guess.
    best = scored |> Enum.map(&elem(&1, 1)) |> Enum.max(fn -> 0 end)

    matches =
      scored
      |> Enum.filter(fn {_, covers, by_name, _, _, _, _} -> covers == best or (by_name and covers + 1 == best) end)
      |> Enum.sort_by(fn {score, _, _, at, _, _, _} -> {-score, at} end)
      |> Enum.take(max(limit, 1))

    installed = MapSet.new(running, fn {_name, version} -> version.artifact end)
    could = installable(deployed, installed, query, matches == [], best, enough, mode)

    if matches == [] do
      nothing(could, plugins, mode)
    else
      calls = for {_, _, _, _, plugin, _, tool} <- matches, do: call(plugin, tool, mode)
      room = @matches - Enum.sum(for call <- calls, do: String.length(call) + 4)

      described =
        fit(
          for({_, _, _, _, _, _, tool} <- matches, do: String.trim(String.replace(tool.description, ~r/\s+/, " "))),
          room
        )

      said =
        for {{_, _, _, _, _, _, tool}, call, about} <- Enum.zip([matches, calls, described]) do
          reads = if tool[:read_only] == true, do: " It only reads.", else: ""
          "#{call}\n  #{about}#{reads}"
        end

      shown = matches |> Enum.map(&{elem(&1, 4), elem(&1, 5)}) |> Enum.reject(&(elem(&1, 0) == nil)) |> Enum.uniq()
      Enum.join(said ++ skills(shown) ++ could, "\n\n")
    end
  end

  @doc "What a plugin says it's for: its skill's description, or nothing."
  def described(version) do
    case Enum.find(Version.skills(version), &(&1.at == "")) do
      %{frontmatter: %{"description" => description}} -> description
      _ -> ""
    end
  end

  # How well a tool matches: how many of the query's words its name has, its plugin's name or title, its own
  # description and what its plugin says it's for, and whether its name has one. Its name counts most, then its
  # plugin's, its own description, and last what its plugin is for, which alone never makes a match: so a word in a
  # plugin's one line doesn't put every tool of it above the one whose own description has it.
  defp score(plugin, version, tool, plugin_about, query) do
    name = words(tool.name)
    plugin_name = words("#{plugin} #{version && version.title}")
    # Its inputs' descriptions are what it says it does too: gmail_search's says it finds what's unread.
    inputs = for {_, input} <- stringed(tool.inputs)["properties"] || %{}, do: input["description"] || ""
    about = words(Enum.join([tool.description | inputs], " "))

    score =
      Enum.sum(
        for word <- query,
            do:
              if(has(name, word), do: 6, else: 0) + if(has(plugin_name, word), do: 4, else: 0) +
                if(has(about, word), do: 3, else: 0) + if(has(plugin_about, word), do: 1, else: 0)
      )

    covers = Enum.count(query, &(has(name, &1) or has(plugin_name, &1) or has(about, &1) or has(plugin_about, &1)))
    {score, covers, Enum.any?(query, &has(name, &1))}
  end

  # The plugins whoever asked could install that match: as well as what they have, or, when nothing they have
  # does, any with one of the words, since all there is of them is their name, what their skill says they're for
  # and their tools' names.
  defp installable(deployed, installed, query, none, best, enough, mode) do
    could =
      for plugin <- deployed, not MapSet.member?(installed, plugin.plugin) do
        name = words("#{Path.basename(plugin.plugin)} #{plugin.title}")
        about = words("#{plugin.description} #{Enum.join(plugin.tools, " ")}")

        score =
          Enum.sum(for word <- query, do: if(has(name, word), do: 4, else: 0) + if(has(about, word), do: 2, else: 0))

        {score, Enum.count(query, &(has(name, &1) or has(about, &1))), plugin}
      end

    fits = fn covers -> if none, do: covers > 0, else: enough.(covers) and covers >= best end

    case could |> Enum.filter(&fits.(elem(&1, 1))) |> Enum.sort_by(&(-elem(&1, 0))) |> Enum.take(5) do
      [] ->
        []

      could ->
        lines =
          for {_, _, plugin} <- could do
            install = call(nil, %{name: "install", inputs: nil}, mode, plugin: plugin.plugin)
            about = if plugin.description == "", do: "", else: ": " <> String.trim_trailing(plugin.description, ".")
            "- #{plugin.title} (#{plugin.plugin})#{about}. #{install} installs it for you."
          end

        ["Not installed for you yet, and may do it:\n" <> Enum.join(lines, "\n")]
    end
  end

  defp nothing(could, plugins, mode) do
    make =
      case mode do
        :cli -> "If no plugin does it yet, make one: `one building-plugins` says how."
        _ -> "If no plugin does it yet, make one: search for building-plugins says how."
      end

    case could do
      [] ->
        names = Enum.map_join(plugins, ", ", &elem(&1, 0))
        "Nothing matches. Try other words, or a plugin's name: #{names}. #{make}"

      could ->
        Enum.join(["Nothing installed for you does that." | could] ++ [make], "\n\n")
    end
  end

  # The skill of each plugin a match is in, read at once.
  defp skills(shown) do
    with_skill = Enum.filter(shown, fn {_name, version} -> Enum.any?(Version.skills(version), &(&1.at == "")) end)
    read = Bucket.read_all(for {_name, version} <- with_skill, do: "#{version.artifact}/code/#{version.code}/SKILL.md")

    for {{name, version}, {:ok, text, _tag}} <- Enum.zip(with_skill, read) do
      body = text |> String.split(~r/\A---\r?\n.*?\r?\n---\r?\n/s, parts: 2) |> List.last() |> String.trim()
      "## #{version.title} (#{name})\n\n#{body}"
    end
  end

  @doc """
  A tool as `mode` calls it, `plugin`'s or One Core's own (`nil`), with its inputs as TypeScript has them, or
  with `inputs` given.
  """
  def call(plugin, tool, mode, inputs \\ nil) do
    name = if plugin, do: "#{plugin}_#{tool.name}", else: tool.name
    typed = if inputs, do: given(inputs, mode), else: ts(tool.inputs)

    case mode do
      :mcp -> "#{name} #{typed}"
      :code -> "#{path(plugin, tool.name)}(#{typed})"
      :cli when inputs != nil -> "one #{String.replace(name, "_", " ", global: false)} #{typed}"
      :cli -> Enum.join(["one #{String.replace(name, "_", " ", global: false)}" | flags(tool.inputs)], " ")
    end
  end

  defp given(inputs, :cli), do: Enum.map_join(inputs, " ", fn {key, value} -> "--#{key} #{value}" end)

  defp given(inputs, _mode),
    do: "{ " <> Enum.map_join(inputs, ", ", fn {key, value} -> "#{key}: #{inspect(value)}" end) <> " }"

  @doc "A tool's path in a code mode script: `tools.gmail.search`, `tools[\"google-calendar\"].events`, `tools.deploy`."
  def path(nil, name), do: "tools" <> key(name)
  def path(plugin, name), do: "tools" <> key(plugin) <> key(name)

  defp key(name), do: if(name =~ ~r/^[A-Za-z_$][A-Za-z0-9_$]*$/, do: "." <> name, else: "[#{inspect(name)}]")

  # A tool's inputs as the command line takes them, those it needs first.
  defp flags(schema) do
    {needed, optional} = inputs(schema)

    for({name, input} <- needed, do: "--#{name} <#{ts(input)}>") ++
      for {name, input} <- optional do
        if input["type"] == "boolean", do: "[--#{name}]", else: "[--#{name} <#{ts(input)}>]"
      end
  end

  defp inputs(schema) do
    schema = stringed(schema)
    required = List.wrap(schema["required"])
    properties = Enum.sort(schema["properties"] || %{})

    {for(name <- required, input = schema["properties"][name], do: {name, input}),
     Enum.reject(properties, &(elem(&1, 0) in required))}
  end

  defp stringed(schema), do: JSON.decode!(JSON.encode!(schema || %{}))

  @doc "A JSON Schema as a TypeScript type, as a code mode signature shows it."
  def ts(schema) do
    schema = stringed(schema)

    case {schema["enum"], schema["type"], schema["properties"]} do
      {[_ | _] = one_of, _, _} -> Enum.map_join(one_of, " | ", &JSON.encode!/1)
      {_, "string", _} -> "string"
      {_, type, _} when type in ["integer", "number"] -> "number"
      {_, "boolean", _} -> "boolean"
      {_, "array", _} -> array(ts(schema["items"]))
      {_, type, %{}} when type in ["object", nil] -> object(schema)
      {_, "object", _} -> "object"
      _ -> "unknown"
    end
  end

  defp array(type), do: if(String.contains?(type, " "), do: "(#{type})[]", else: "#{type}[]")

  defp object(schema) do
    {needed, optional} = inputs(schema)

    said =
      for({name, input} <- needed, do: "#{name}: #{ts(input)}") ++
        for({name, input} <- optional, do: "#{name}?: #{ts(input)}")

    if said == [], do: "{}", else: "{ #{Enum.join(said, ", ")} }"
  end

  @doc """
  Descriptions that fit in `room` characters between them: as written when they do, else each cut to its first
  sentence, else to an equal share of the room.
  """
  def fit(described, room) do
    total = fn texts -> Enum.sum(Enum.map(texts, &String.length/1)) end

    if total.(described) <= room do
      described
    else
      first =
        for text <- described, do: (text |> String.split(". ", parts: 2) |> hd() |> String.trim_trailing(".")) <> "."

      share = div(room, max(length(first), 1))

      if total.(first) <= room,
        do: first,
        else: for(text <- first, do: if(String.length(text) > share, do: cut(text, share), else: text))
    end
  end

  defp cut(text, share), do: String.trim_trailing(String.slice(text, 0, max(share - 3, 0))) <> "..."

  # The words of `text` that count, each by its stem, with what people call a thing as the tools here call it.
  defp words(text) do
    text
    |> to_string()
    |> String.split(~r/[^[:alnum:]]+/u, trim: true)
    |> Enum.map(&String.downcase/1)
    |> Enum.reject(&(String.length(&1) < 2 or &1 in @filler))
    |> Enum.map(fn
      word when word in ["machine", "box"] -> "device"
      word when word in ["save", "remember"] -> "keep"
      word -> word
    end)
    |> Enum.map(&stem/1)
  end

  # A plural's s even on a short word: keys is key.
  defp stem(word) do
    Enum.find_value(["ing", "es", "s", "ed"], word, fn ending ->
      if String.length(word) > String.length(ending) + if(ending == "s", do: 2, else: 3) and
           String.ends_with?(word, ending),
         do: String.replace_suffix(word, ending, "")
    end)
  end

  # Whether `word`, a query's, is among `list`: either may be the other's stem (devices is devic, device is device).
  defp has(list, word) do
    Enum.any?(list, fn other ->
      other == word or (String.length(word) > 3 and String.starts_with?(other, word)) or
        (String.length(other) > 3 and String.starts_with?(word, other))
    end)
  end
end
