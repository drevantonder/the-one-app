defmodule Gmail.Message do
  @moduledoc """
  A message as Gmail's API gives it (its Message resource, with its payload of MessageParts), as Gmail's tools answer
  it: its headers, when it came, its labels, its attachments and its body, plain text first, else its HTML made plain.
  """

  # Text it would be cut at, in characters.
  @longest 20_000
  # The invisible padding marketing emails use.
  @filler ~r/[\x{200b}-\x{200d}\x{2060}\x{feff}\x{034f}\x{00ad}]/u
  @entities %{"amp" => "&", "lt" => "<", "gt" => ">", "quot" => "\"", "apos" => "'", "nbsp" => " "}

  @doc "A message in full: what `message/0` says."
  def message(message, names) do
    payload = message["payload"]
    text = body(payload)
    cut = String.length(text) > @longest

    files =
      for {file, number} <- Enum.with_index(attachments(payload), 1),
          do: %{number: number, name: file.name, type: file.type, size: file.size}

    %{
      id: message["id"],
      threadId: message["threadId"],
      date: time(message),
      subject: header(payload, "Subject"),
      from: header(payload, "From"),
      to: header(payload, "To"),
      cc: header(payload, "Cc"),
      replyTo: header(payload, "Reply-To"),
      labels: labels(message, names),
      attachments: files,
      text: if(cut, do: String.slice(text, 0, @longest), else: text),
      cut: cut
    }
  end

  @doc "A message as a search finds it: what `found/0` says."
  def found(message, names) do
    %{
      id: message["id"],
      threadId: message["threadId"],
      date: time(message),
      from: header(message["payload"], "From"),
      to: header(message["payload"], "To"),
      subject: header(message["payload"], "Subject"),
      labels: labels(message, names),
      snippet: snippet(message["snippet"] || "")
    }
  end

  @fields %{
    id: %{type: "string", description: "The message's id"},
    threadId: %{type: "string", description: "Its thread's id"},
    date: %{type: "string", description: "When it came, to the minute: ISO 8601, in UTC"},
    from: %{type: "string"},
    to: %{type: "string"},
    subject: %{type: "string"},
    labels: %{type: "array", items: %{type: "string"}, description: "Its labels' names, but Gmail's categories"}
  }

  @doc "What `message/2` gives, as JSON Schema: a tool's outputs."
  def message do
    %{
      type: "object",
      properties:
        Map.merge(@fields, %{
          cc: %{type: "string"},
          replyTo: %{type: "string"},
          attachments: %{
            type: "array",
            items: %{
              type: "object",
              properties: %{
                number: %{type: "integer", description: "Which it is, for gmail_attachment: 1 the first"},
                name: %{type: "string"},
                type: %{type: "string", description: "Its MIME type"},
                size: %{type: "integer", description: "In bytes"}
              },
              required: ["number", "name", "type", "size"]
            }
          },
          text: %{type: "string", description: "Its text: its plain text, or its HTML made plain"},
          cut: %{type: "boolean", description: "Whether text was cut short, at 20,000 characters"}
        }),
      required: ~w(id threadId date subject from to cc replyTo labels attachments text cut)
    }
  end

  @doc "What `found/2` gives, as JSON Schema."
  def found do
    %{
      type: "object",
      properties: Map.put(@fields, :snippet, %{type: "string", description: "The start of its text"}),
      required: ~w(id threadId date from to subject labels snippet)
    }
  end

  @doc "A header's value, or empty."
  def header(part, name) do
    wanted = String.downcase(name)
    Enum.find_value((part || %{})["headers"] || [], "", &(String.downcase(&1["name"]) == wanted && &1["value"]))
  end

  @doc "When it came, to the minute, in UTC."
  def time(message) do
    {:ok, at} = DateTime.from_unix(String.to_integer(message["internalDate"]), :millisecond)
    Calendar.strftime(at, "%Y-%m-%dT%H:%MZ")
  end

  @doc "Its labels' names, but Gmail's categories."
  def labels(message, names) do
    (message["labelIds"] || [])
    |> Enum.reject(&String.starts_with?(&1, "CATEGORY_"))
    |> Enum.map(&Map.get(names, &1, &1))
  end

  @doc "A snippet, as text."
  def snippet(text), do: text |> unescape() |> String.replace(@filler, "") |> String.split() |> Enum.join(" ")

  @doc "The files in a message, each its name, type, size, and its attachment's id or its data."
  def attachments(part) do
    own =
      if part["filename"] not in [nil, ""] do
        body = part["body"] || %{}
        [%{name: part["filename"], type: part["mimeType"], size: body["size"] || 0, id: body["attachmentId"], data: body["data"]}]
      else
        []
      end

    own ++ Enum.flat_map(part["parts"] || [], &attachments/1)
  end

  @doc "Bytes from Gmail's base64, which is the URL's kind, padded or not."
  def bytes(nil), do: ""

  def bytes(data) do
    data |> String.replace(~r/[\s=]/, "") |> Base.url_decode64!(padding: false)
  end

  @doc "A message's text: its plain text, or its HTML made plain."
  def body(payload) do
    {plain, html} = texts(payload, {nil, nil})

    if plain && String.trim(plain) != "" do
      plain
      |> String.replace("\r\n", "\n")
      |> String.replace(@filler, "")
      |> String.replace(~r/[ \t\x{a0}]+\n/u, "\n")
      |> String.trim()
    else
      plain_html(html || "")
    end
  end

  @doc "HTML as text: its words, lines and list items, without its markup."
  def plain_html(html) do
    html
    |> String.replace(~r/<(head|style|script|title)\b[\s\S]*?<\/\1\s*>/i, "")
    |> String.replace(~r/<br\s*\/?>|<\/(p|div|tr|h[1-6]|table|ul|ol|blockquote)\s*>/i, "\n")
    |> String.replace(~r/<li\b[^>]*>/i, "\n- ")
    |> String.replace(~r/<[^>]*>/, "")
    |> unescape()
    |> String.replace(@filler, "")
    |> String.replace(~r/[ \t\x{a0}]+/u, " ")
    |> String.split("\n")
    |> Enum.map_join("\n", &String.trim/1)
    |> String.replace(~r/\n\s*\n\s*\n+/, "\n\n")
    |> String.trim()
  end

  # The first plain text and the first HTML that aren't a file.
  defp texts(part, found) do
    cond do
      part["filename"] not in [nil, ""] -> found
      String.starts_with?(part["mimeType"] || "", "multipart/") -> Enum.reduce(part["parts"] || [], found, &texts/2)
      part["mimeType"] == "text/plain" and elem(found, 0) == nil -> put_elem(found, 0, decode(part))
      part["mimeType"] == "text/html" and elem(found, 1) == nil -> put_elem(found, 1, decode(part))
      true -> found
    end
  end

  # A part's text, as UTF-8: it's that, or Latin-1, which every byte is.
  defp decode(part) do
    bytes = bytes((part["body"] || %{})["data"])
    if String.valid?(bytes), do: bytes, else: :unicode.characters_to_binary(bytes, :latin1)
  end

  defp unescape(text) do
    Regex.replace(~r/&(#x[0-9a-f]+|#\d+|[a-z]+);/i, text, fn all, entity ->
      case entity do
        "#x" <> hex -> codepoint(String.to_integer(hex, 16), all)
        "#X" <> hex -> codepoint(String.to_integer(hex, 16), all)
        "#" <> decimal -> codepoint(String.to_integer(decimal), all)
        name -> Map.get(@entities, String.downcase(name), all)
      end
    end)
  end

  defp codepoint(code, all) do
    <<code::utf8>>
  rescue
    _ -> all
  end
end
