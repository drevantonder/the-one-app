defmodule Gmail.Mime do
  @moduledoc """
  An email as RFC 5322 text, all of it ASCII: its text, or its text and HTML as multipart/alternative, and its
  files after them in multipart/mixed. Gmail adds From, Date and Message-ID.

  Every part is base64, and a header that isn't plain ASCII is an RFC 2047 encoded word, so no line is too long
  and nothing in it says `{{...}}`, which the Vault would fill in on its way to Gmail.
  """

  @types %{
    "pdf" => "application/pdf",
    "png" => "image/png",
    "jpg" => "image/jpeg",
    "jpeg" => "image/jpeg",
    "gif" => "image/gif",
    "webp" => "image/webp",
    "heic" => "image/heic",
    "txt" => "text/plain",
    "csv" => "text/csv",
    "html" => "text/html",
    "htm" => "text/html",
    "md" => "text/markdown",
    "json" => "application/json",
    "zip" => "application/zip",
    "doc" => "application/msword",
    "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "xls" => "application/vnd.ms-excel",
    "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "ppt" => "application/vnd.ms-powerpoint",
    "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "ics" => "text/calendar",
    "mp3" => "audio/mpeg",
    "m4a" => "audio/mp4",
    "mp4" => "video/mp4",
    "mov" => "video/quicktime"
  }

  @doc "A file's MIME type, by its name's extension."
  def type(name), do: Map.get(@types, name |> String.split(".") |> List.last() |> String.downcase(), "application/octet-stream")

  @doc "Who an email goes to, each an address as a header says it: from addresses, or lists of them in one text."
  def recipients(nil), do: []

  def recipients(given) do
    given
    |> List.wrap()
    |> Enum.flat_map(&split/1)
    |> Enum.map(&address/1)
  end

  @doc "A boundary that no part says."
  def boundary(kind), do: "one-#{kind}-#{Base.url_encode64(:crypto.strong_rand_bytes(12), padding: false)}"

  @doc "The email, as RFC 5322 text."
  def rfc822(mail) do
    fold = &Enum.join(&1, ",\r\n ")

    head =
      [
        mail.to != [] && "To: #{fold.(mail.to)}",
        mail.cc != [] && "Cc: #{fold.(mail.cc)}",
        mail.bcc != [] && "Bcc: #{fold.(mail.bcc)}",
        "Subject: #{word(String.replace(mail.subject, ~r/[\r\n]+/, " "))}",
        mail.in_reply_to not in [nil, ""] && "In-Reply-To: #{mail.in_reply_to}",
        mail.references not in [nil, ""] && "References: #{mail.references}",
        "MIME-Version: 1.0"
      ]
      |> Enum.filter(&is_binary/1)

    content =
      if mail.html not in [nil, ""] do
        b = boundary("alt")
        text = mail.text || Gmail.Message.plain_html(mail.html)
        "Content-Type: multipart/alternative; boundary=\"#{b}\"\r\n\r\n--#{b}\r\n#{text("text/plain", text)}--#{b}\r\n#{text("text/html", mail.html)}--#{b}--\r\n"
      else
        text("text/plain", mail.text || "")
      end

    content =
      if mail.files == [] do
        content
      else
        b = boundary("mixed")

        files =
          for file <- mail.files do
            ascii = ascii?(file.name)
            name = if ascii, do: file.name, else: word(file.name)
            star = if ascii, do: "", else: "; filename*=UTF-8''#{URI.encode(file.name, &URI.char_unreserved?/1)}"

            "--#{b}\r\nContent-Type: #{file.type}; name=\"#{name}\"\r\nContent-Disposition: attachment; filename=\"#{name}\"#{star}\r\n" <>
              "Content-Transfer-Encoding: base64\r\n\r\n#{wrapped(file.bytes)}\r\n"
          end

        "Content-Type: multipart/mixed; boundary=\"#{b}\"\r\n\r\n--#{b}\r\n#{content}#{Enum.join(files)}--#{b}--\r\n"
      end

    Enum.join(head, "\r\n") <> "\r\n" <> content
  end

  # A part of text in UTF-8, base64.
  defp text(type, text),
    do: "Content-Type: #{type}; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n#{wrapped(String.replace(text, ~r/\r?\n/, "\r\n"))}\r\n"

  # Base64 in lines of 76, as MIME wants it.
  defp wrapped(bytes), do: bytes |> Base.encode64() |> lines([])

  defp lines(<<line::binary-size(76), rest::binary>>, done) when rest != "", do: lines(rest, [line | done])
  defp lines(last, done), do: Enum.join(Enum.reverse([last | done]), "\r\n")

  defp ascii?(text), do: text =~ ~r/^[\x20-\x7e]*$/ and not String.contains?(text, ["{{", "=?"])

  # An RFC 2047 encoded word, or several folded, for text that isn't plain ASCII or would look like a placeholder.
  defp word(text) do
    if ascii?(text) do
      text
    else
      text
      |> String.codepoints()
      |> Enum.chunk_while(
        "",
        fn char, chunk ->
          if byte_size(chunk <> char) > 45, do: {:cont, chunk, char}, else: {:cont, chunk <> char}
        end,
        fn chunk -> {:cont, chunk, ""} end
      )
      |> Enum.reject(&(&1 == ""))
      |> Enum.map_join("\r\n ", &"=?UTF-8?B?#{Base.encode64(&1)}?=")
    end
  end

  # One text of addresses, split at its commas and semicolons, but not those in quotes or angle brackets.
  defp split(text) when is_binary(text) do
    {parts, last, _quoted, _angle} =
      text
      |> String.graphemes()
      |> Enum.reduce({[], "", false, false}, fn char, {parts, now, quoted, angle} ->
        cond do
          char == "\"" -> {parts, now <> char, not quoted, angle}
          char == "<" and not quoted -> {parts, now <> char, quoted, true}
          char == ">" and not quoted -> {parts, now <> char, quoted, false}
          char in [",", ";"] and not quoted and not angle -> {[now | parts], "", quoted, angle}
          true -> {parts, now <> char, quoted, angle}
        end
      end)

    [last | parts] |> Enum.reverse() |> Enum.map(&String.trim/1) |> Enum.reject(&(&1 == ""))
  end

  defp split(%{} = address), do: [address]

  # An address as a header says it: Name <a@example.org>, its name an encoded word or quoted where it must be.
  defp address(%{} = given), do: address(given["email"] || given["address"] || "", given["name"] || "")

  defp address(text) do
    case Regex.run(~r/^(.*?)\s*<([^<>]+)>$/, String.trim(text)) do
      [_all, name, email] ->
        name = name |> String.trim() |> String.replace(~r/^"(.*)"$/, "\\1") |> String.replace(~r/\\(.)/, "\\1")
        address(String.trim(email), name)

      nil ->
        address(String.trim(text), "")
    end
  end

  defp address(email, name) do
    email = String.replace(email, ~r/[\r\n]/, "")
    if not (email =~ ~r/^[^\s@<>,{}]+@[^\s@<>,{}]+$/), do: raise("\"#{email}\" isn't an email address.")
    name = name |> String.replace(~r/[\r\n]+/, " ") |> String.trim()

    cond do
      name == "" -> email
      not ascii?(name) -> "#{word(name)} <#{email}>"
      name =~ ~r/[()<>\[\]:;@\\,."]/ -> "\"#{String.replace(name, ~r/(["\\])/, "\\\\\\1")}\" <#{email}>"
      true -> "#{name} <#{email}>"
    end
  end
end
