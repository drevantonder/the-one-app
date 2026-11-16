defmodule Gmail.Message do
  @moduledoc """
  A message as Gmail's API gives it (its Message resource, with its payload of MessageParts), said as text: its
  headers, when it came, its labels, its attachments and its body, plain text first, else its HTML made plain.
  """

  # Text it would be cut at, in characters.
  @longest 20_000
  # The invisible padding marketing emails use.
  @filler ~r/[\x{200b}-\x{200d}\x{2060}\x{feff}\x{034f}\x{00ad}]/u
  @entities %{"amp" => "&", "lt" => "<", "gt" => ">", "quot" => "\"", "apos" => "'", "nbsp" => " "}

  @doc "A message in full, as text."
  def show(message, names) do
    payload = message["payload"]

    head =
      for name <- ["Subject", "From", "To", "Cc", "Reply-To"], (value = header(payload, name)) != "", do: "#{name}: #{value}"

    labels = labels(message, names)

    files =
      for {file, at} <- Enum.with_index(attachments(payload), 1),
          do: "Attachment #{at}: #{file.name} (#{file.type}, #{size(file.size)})"

    text = body(payload)
    text = if String.length(text) > @longest, do: String.slice(text, 0, @longest) <> "\n\n[cut: the message goes on]", else: text

    lines = head ++ ["Date: #{time(message)}", "Id: #{message["id"]} (thread #{message["threadId"]})"]
    lines = if labels != "", do: lines ++ ["Labels: #{labels}"], else: lines
    Enum.join(lines ++ files, "\n") <> "\n\n" <> text
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
    |> Enum.map_join(", ", &Map.get(names, &1, &1))
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

  defp size(bytes) when bytes >= 1_048_576, do: "#{Float.round(bytes / 1_048_576, 1)} MB"
  defp size(bytes) when bytes >= 1024, do: "#{div(bytes, 1024)} KB"
  defp size(bytes), do: "#{bytes} bytes"
end
