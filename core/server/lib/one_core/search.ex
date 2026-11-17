defmodule OneCore.Search do
  @moduledoc """
  Code mode's `search`, and `one search`: the tools for a task among those whoever asks has, best first, each with
  the exact way to call it, what it does, whether it only reads and whether it runs as a task, then the skill of each
  plugin they're in. It also offers the plugins they could install that match, from what anyone has deployed.

  It ranks as the `search()` inside a code mode script does (OpenCode's, `@opencode/codemode`'s `tool-runtime.js`),
  over the same tools: field-weighted BM25 over each tool's name, its plugin's name, its description, its inputs'
  names and what its plugin says it's for, with the same weights, words and stems, so both give the same tools for
  the same words. Executor's `search` tool does the same.

  The call is written as whoever asked calls tools (`_meta.mode`): a line of a code mode script
  (`tools.gmail.search({ query: string })`), or a command (`one gmail search --query <string>`).
  """
  alias OneCore.Bucket
  alias OneCore.Plugins.Version

  @doc "The tool, as MCP lists it."
  def tool do
    %{
      name: "search",
      description: """
      Finds the tools for a task across every plugin: give a few words, like "unread email" or "add an event". \
      Answers the best matches, each with its exact call and what it does, then each one's plugin's skill, and \
      the plugins you could install that do it. Search before guessing, to learn a plugin, and before saying you \
      can't.\
      """,
      inputSchema: %{
        type: "object",
        properties: %{
          query: %{type: "string", description: "A few words for the task, or a plugin's name"},
          limit: %{type: "integer", description: "How many tools at most, 8 if left out"}
        },
        required: ["query"]
      },
      outputSchema: %{
        type: "object",
        properties: %{text: %{type: "string", description: "The matches, their skills and what you could install"}},
        required: ["text"]
      },
      annotations: %{readOnlyHint: true}
    }
  end

  # About how much of an answer the matches may take, in characters, before their descriptions are cut short, and
  # how much their plugins' skills may, best match's first, before the rest are only named.
  @matches 6_000
  @skills 8_000

  # OpenCode's ranking: BM25 (k1 1.2, b 0.75) with each field weighted, a query word of 4 or more letters matching
  # longer words it starts with at 0.3, and a score raised for a short name.
  @weights [name: 3, namespace: 2, description: 1, namespaceDescription: 0.4, parameters: 0.3]
  @k1 1.2
  @b 0.75
  @brevity 0.5
  @prefix 0.3

  @doc """
  The answer to `query`, at most `limit` of `tools` (as MCP lists them, a plugin's saying whose it is in `_meta`),
  `running` being the plugins they're from and `deployed` what anyone has deployed. Its calls are written for
  `mode`: `:code` or `:cli`.
  """
  def run(query, limit, tools, running, deployed, mode) do
    found = entries(tools, running)
    matches = query |> rank(found) |> Enum.take(max(limit, 1))
    installed = MapSet.new(running, fn {_name, version} -> version.artifact end)
    could = installable(deployed, installed, query, matches == [], mode)

    if matches == [] do
      nothing(could, Enum.sort(Map.keys(running)), mode)
    else
      calls = for entry <- matches, do: call(entry.plugin, entry.tool, mode)
      room = @matches - Enum.sum(for call <- calls, do: String.length(call) + 4)

      described =
        fit(for(entry <- matches, do: String.trim(String.replace(entry.tool.description, ~r/\s+/, " "))), room)

      lines =
        for {entry, call, about} <- Enum.zip([matches, calls, described]) do
          reads = if entry.tool.read_only, do: " It only reads.", else: ""
          task = if entry.tool.task, do: " It runs as a task.", else: ""
          "#{call}\n  #{about}#{reads}#{task}"
        end

      shown = for(entry <- matches, entry.plugin != nil, do: {entry.plugin, running[entry.plugin]}) |> Enum.uniq()
      Enum.join(lines ++ skills(shown, mode) ++ could, "\n\n")
    end
  end

  @doc "What a plugin says it's for: its skill's description, or nothing."
  def described(version) do
    case Enum.find(Version.skills(version), &(&1.at == "")) do
      %{frontmatter: %{"description" => description}} -> description
      _ -> ""
    end
  end

  # Each tool as a script reaches it, by its path there (`gmail.search`, `deploy`), in order of it, with the words
  # each field of it ranks by, as OpenCode takes them from a tool and its namespace.
  defp entries(tools, running) do
    tools
    |> Enum.map(fn listed ->
      plugin = listed[:_meta][:plugin]
      name = if plugin, do: String.replace_prefix(listed.name, plugin <> "_", ""), else: listed.name
      read_only = listed[:annotations][:readOnlyHint] == true
      # Whether its plugin makes it an MCP task, which a tool's listing doesn't say (`OneCore.Tasks`).
      own = if plugin, do: running[plugin].tools, else: OneCore.Tools.tools()

      tool = %{
        name: name,
        description: listed.description || "",
        inputs: listed.inputSchema,
        outputs: listed[:outputSchema],
        read_only: read_only,
        task: Enum.any?(own, &(&1.name == name and &1[:task_support] != nil))
      }

      about = if plugin, do: [described(running[plugin])], else: []

      %{
        path: Enum.join(List.wrap(plugin) ++ [name], "."),
        plugin: plugin,
        tool: tool,
        fields: %{
          name: [name],
          namespace: List.wrap(plugin),
          description: [tool.description],
          namespaceDescription: about,
          parameters: Map.keys(stringed(tool.inputs)["properties"] || %{})
        },
        length: length(split(name))
      }
    end)
    |> Enum.sort_by(& &1.path)
  end

  # The entries for `query`, best first, as OpenCode ranks them: a query that's a tool's path is that tool alone.
  defp rank(query, entries) do
    trimmed = String.trim(query)
    path = String.replace_prefix(trimmed, "tools.", "")

    case Enum.find(entries, &(path != "" and (&1.path == path or path(&1.path) == trimmed))) do
      nil -> bm25(search_words(path), entries)
      exact -> [exact]
    end
  end

  # Field-weighted BM25. Each word's weight in an entry is rounded as OpenCode keeps it, in a Float32Array, so
  # scores, and so ties, are the same.
  defp bm25([], _entries), do: []

  defp bm25(terms, entries) do
    fields =
      for entry <- entries,
          do: Map.new(entry.fields, fn {field, texts} -> {field, Enum.flat_map(texts, &search_words/1)} end)

    count = length(entries)

    average =
      Map.new(@weights, fn {field, _} ->
        total = Enum.sum(for words <- fields, do: length(words[field]))
        {field, if(total == 0, do: 1, else: total / count)}
      end)

    postings =
      fields
      |> Enum.with_index()
      |> Enum.reduce(%{}, fn {words, position}, postings ->
        weighted =
          Enum.reduce(@weights, %{}, fn {field, weight}, weighted ->
            weight = weight / (1 - @b + @b * length(words[field]) / average[field])

            Enum.reduce(words[field], weighted, fn word, weighted ->
              Map.update(weighted, word, weight, &(&1 + weight))
            end)
          end)

        Enum.reduce(weighted, postings, fn {word, weight}, postings ->
          Map.update(postings, word, [{position, float32(weight)}], &[{position, float32(weight)} | &1])
        end)
      end)

    known = Map.keys(postings)

    scores =
      Enum.reduce(terms, %{}, fn term, scores ->
        prefixed =
          if String.length(term) >= 4,
            do: for(word <- known, word != term, String.starts_with?(word, term), do: {word, @prefix}),
            else: []

        # Each entry counts a query word once, through its best match.
        best =
          Enum.reduce([{term, 1} | prefixed], %{}, fn {word, factor}, best ->
            posting = Map.get(postings, word, [])
            found = length(posting)
            idf = :math.log(1 + (count - found + 0.5) / (found + 0.5))

            Enum.reduce(posting, best, fn {position, weight}, best ->
              score = factor * idf * weight * (@k1 + 1) / (weight + @k1)
              if score > Map.get(best, position, 0), do: Map.put(best, position, score), else: best
            end)
          end)

        Enum.reduce(best, scores, fn {position, score}, scores -> Map.update(scores, position, score, &(&1 + score)) end)
      end)

    entries = List.to_tuple(entries)

    scores
    |> Enum.map(fn {position, score} ->
      entry = elem(entries, position)
      {score * (1 + @brevity / max(1, entry.length)), entry}
    end)
    |> Enum.sort(fn {left, a}, {right, b} -> left > right or (left == right and a.path <= b.path) end)
    |> Enum.map(&elem(&1, 1))
  end

  defp float32(number) do
    <<rounded::float-32>> = <<number::float-32>>
    rounded
  end

  # A text's words as OpenCode splits them: at case changes and anything but ASCII letters and digits, with each
  # camelCase word whole too (`CronJob` is cron, job and cronjob), each without a plural's ending.
  defp search_words(text) do
    whole = for [word] <- Regex.scan(~r/[A-Za-z0-9]*[a-z0-9][A-Z][A-Za-z0-9]*/, text), do: String.downcase(word)
    Enum.map(split(text) ++ whole, &stem/1)
  end

  defp split(text) do
    ~r/([a-z0-9])([A-Z])/
    |> Regex.replace(text, "\\1 \\2")
    |> String.downcase()
    |> String.split(~r/[^a-z0-9]+/, trim: true)
  end

  # Plural endings only: settings is setting, statuses status, vms vm.
  defp stem(word) do
    cond do
      String.length(word) > 4 and String.ends_with?(word, "ies") ->
        String.slice(word, 0..-4//1) <> "y"

      String.length(word) > 4 and word =~ ~r/(?:ss|sh|ch|x|z|us)es$/ ->
        String.slice(word, 0..-3//1)

      String.length(word) > 2 and String.ends_with?(word, "s") and not (word =~ ~r/(?:ss|us|is)$/) ->
        String.slice(word, 0..-2//1)

      true ->
        word
    end
  end

  # The plugins whoever asked could install that match, ranked as tools are, by their names, what they're for and
  # their tools' names: those with one of the words in their name, or, when nothing they have matches, any. All
  # there is of one is what its listing says.
  defp installable(deployed, installed, query, none, mode) do
    entries =
      for plugin <- deployed, not MapSet.member?(installed, plugin.plugin) do
        %{
          path: plugin.plugin,
          plugin: plugin,
          fields: %{
            name: [],
            namespace: [Path.basename(plugin.plugin), plugin.title],
            description: [plugin.description],
            namespaceDescription: [],
            parameters: plugin.tools
          },
          length: 1
        }
      end
      |> Enum.sort_by(& &1.path)

    words = MapSet.new(search_words(query))

    named = fn entry ->
      Enum.any?(entry.fields.namespace, &Enum.any?(search_words(&1), fn word -> word in words end))
    end

    case query |> rank(entries) |> Enum.filter(&(none or named.(&1))) |> Enum.take(5) do
      [] ->
        []

      could ->
        lines =
          for %{plugin: plugin} <- could do
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
        :cli ->
          "If no plugin does it yet, make one: `one building-plugins` says how."

        _ ->
          ~s(If no plugin does it yet, make one: the skill building-plugins says how, `skills { name: "building-plugins" }`.)
      end

    case could do
      [] ->
        "Nothing matches. Try other words, or a plugin's name: #{Enum.join(plugins, ", ")}. #{make}"

      could ->
        Enum.join(["Nothing installed for you does that." | could] ++ [make], "\n\n")
    end
  end

  # The skill of each plugin a match is in, read at once, as many as fit, and how to read the rest.
  defp skills(shown, mode) do
    with_skill = Enum.filter(shown, fn {_name, version} -> Enum.any?(Version.skills(version), &(&1.at == "")) end)
    read = Bucket.read_all(for {_name, version} <- with_skill, do: "#{version.artifact}/code/#{version.code}/SKILL.md")

    skills =
      for {{name, version}, {:ok, text, _tag}} <- Enum.zip(with_skill, read) do
        body = text |> String.split(~r/\A---\r?\n.*?\r?\n---\r?\n/s, parts: 2) |> List.last() |> String.trim()
        {name, version, "## #{version.title} (#{name})\n\n#{body}"}
      end

    {fits, rest} =
      skills
      |> Enum.scan({nil, 0}, fn {_, _, text} = skill, {_, used} -> {skill, used + String.length(text)} end)
      |> Enum.with_index()
      |> Enum.split_with(fn {{_skill, used}, at} -> at == 0 or used <= @skills end)

    read =
      for {{{name, version, _}, _}, _} <- rest do
        case mode do
          :cli -> "`one #{name}`"
          _ -> ~s(skills { plugin: "#{name}", name: "#{Path.basename(version.artifact)}" })
        end
      end

    for({{{_, _, text}, _}, _} <- fits, do: text) ++
      if(read == [], do: [], else: ["The other plugins' skills: #{Enum.join(read, ", ")}."])
  end

  @doc """
  A tool as `mode` calls it, `plugin`'s or One Core's own (`nil`), with its inputs as TypeScript has them, and in a
  script what it answers, or with `inputs` given.
  """
  def call(plugin, tool, mode, inputs \\ nil) do
    name = if plugin, do: "#{plugin}_#{tool.name}", else: tool.name
    typed = if inputs, do: given(inputs, mode), else: ts(tool.inputs)

    case mode do
      :cli when inputs != nil -> "one #{String.replace(name, "_", " ", global: false)} #{typed}"
      :cli -> Enum.join(["one #{String.replace(name, "_", " ", global: false)}" | flags(tool.inputs)], " ")
      _ when inputs != nil -> "#{path(Enum.join(List.wrap(plugin) ++ [tool.name], "."))}(#{typed})"
      _ -> "#{path(Enum.join(List.wrap(plugin) ++ [tool.name], "."))}(#{typed}): Promise<#{ts(tool[:outputs])}>"
    end
  end

  defp given(inputs, :cli), do: Enum.map_join(inputs, " ", fn {key, value} -> "--#{key} #{value}" end)

  defp given(inputs, _mode),
    do: "{ " <> Enum.map_join(inputs, ", ", fn {key, value} -> "#{key}: #{inspect(value)}" end) <> " }"

  @doc """
  A tool's path in a code mode script, from its path there, as OpenCode writes it: `tools.gmail.search`,
  `tools["google-calendar"].events`, `tools.deploy`.
  """
  def path(dotted) do
    "tools" <>
      Enum.map_join(String.split(dotted, "."), fn segment ->
        if segment =~ ~r/^[A-Za-z_$][A-Za-z0-9_$]*$/, do: "." <> segment, else: "[#{JSON.encode!(segment)}]"
      end)
  end

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
      {_, [_ | _] = types, _} -> Enum.map_join(types, " | ", &ts(Map.put(schema, "type", &1)))
      {_, "null", _} -> "null"
      {_, nil, nil} when is_list(:erlang.map_get("anyOf", schema)) -> Enum.map_join(schema["anyOf"], " | ", &ts/1)
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

    fields =
      for({name, input} <- needed, do: "#{name}: #{ts(input)}") ++
        for({name, input} <- optional, do: "#{name}?: #{ts(input)}")

    if fields == [], do: "{}", else: "{ #{Enum.join(fields, ", ")} }"
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
end
