defmodule GoogleDocs do
  @moduledoc """
  Google Docs: the person's own documents, through Docs' API (v1), read as markdown or text. Signing in and the
  accounts are the `google-account` plugin's, the same for every Google app.
  """
  @behaviour OneCore.Plugin

  @api "https://docs.googleapis.com/v1/"
  @ordered ~r/DECIMAL|ALPHA|ROMAN|ZERO/

  @doc "What Google Docs asks of Google, for the `google-account` plugin."
  def google do
    %{
      title: "Google Docs",
      about: "Sign in to Google, and your agents can read your documents and, when you ask, write in them.",
      scopes: ["https://www.googleapis.com/auth/documents"]
    }
  end

  @impl true
  def uses, do: ["google-account", "vault"]

  @impl true
  def tools do
    string = &%{type: "string", description: &1}
    account = string.("Which account: its address, or a part of it only it has, as google-docs_accounts lists them. The first if left out")

    with_account = fn properties, required ->
      %{type: "object", properties: Map.put(properties, :account, account), required: required}
    end

    document =
      string.(
        "The document's id, or its address, like https://docs.google.com/document/d/1AbC.../edit. google-drive_files_list finds documents (q mimeType = 'application/vnd.google-apps.document' and name contains '...')"
      )

    [
      %{
        name: "accounts",
        read_only: true,
        description:
          "The Google accounts Google Docs can use, the default first, each with ok, and why when it's false (google-docs_sign_in with that account fixes it). Every other tool's account takes one of these.",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "sign_in",
        description:
          "Starts signing a Google account in to Google Docs, or letting it use one another Google app has: gives url, for the person to open in their browser, where Google signs them in and sends them back to finish.",
        inputs: %{type: "object", properties: %{account: string.("The account's address, to suggest it to Google: for one that needs signing in again, as google-docs_accounts lists it")}}
      },
      %{
        name: "sign_out",
        description: "Signs a Google account out of Google Docs. Other Google apps that use it still do, and google-docs_sign_in signs it in again.",
        inputs: with_account.(%{}, ["account"])
      },
      %{
        name: "read",
        read_only: true,
        description:
          "A document (Docs' documents.get) as text: its title, address and body, as markdown (headings, lists, tables, bold, italic, links) or plain text.",
        inputs:
          with_account.(
            %{documentId: document, format: %{type: "string", enum: ["markdown", "text"], description: "markdown, the default, or text"}},
            ["documentId"]
          )
      },
      %{
        name: "create",
        description: "Makes a document in the person's Drive (in My Drive, Docs' documents.create), with text if given. Gives {id, title, url}.",
        inputs:
          with_account.(
            %{title: string.("Its title, like Meeting notes 2026-11-03"), text: string.("Its first text, as plain text: markdown stays as typed")},
            ["title"]
          )
      },
      %{
        name: "append",
        description:
          "Adds text at the end of a document, as a new paragraph (a batchUpdate insertText). This changes the person's document: do it only when they asked.",
        inputs: with_account.(%{documentId: document, text: string.("The text, as plain text: markdown stays as typed")}, ["documentId", "text"])
      },
      %{
        name: "replace",
        description:
          "Replaces every match of some text in a document (a batchUpdate replaceAllText), and says how many changed. Do it only when the person asked.",
        inputs:
          with_account.(
            %{
              documentId: document,
              find: string.("The text to find, exactly as google-docs_read gives it as text"),
              replace: string.("What to put instead, as plain text. Empty removes it"),
              matchCase: %{type: "boolean", description: "Match capitals exactly. False if left out"}
            },
            ["documentId", "find"]
          )
      }
    ]
  end

  @impl true
  def call(tool, inputs, call) when tool in ~w(accounts sign_in sign_out),
    do: google(call).account_call(tool, inputs, call, google())

  def call(tool, inputs, call) do
    run(tool, inputs, %{call: call, email: google(call).account(call, inputs["account"])})
  end

  defp run("read", %{"documentId" => given} = inputs, at) do
    id = id(given)
    document = api(at, "GET", "documents/#{path(at, id)}", missing: no_document(id))
    "#{document["title"]}\n#{url(document["documentId"])}\n\n#{render(document, inputs["format"] != "text")}"
  end

  defp run("create", %{"title" => title} = inputs, at) do
    document = api(at, "POST", "documents", json: %{title: title})

    if text = inputs["text"],
      do: batch(at, document["documentId"], [%{insertText: %{text: text, location: %{index: 1}}}])

    %{id: document["documentId"], title: document["title"], url: url(document["documentId"])}
  end

  defp run("append", %{"documentId" => given, "text" => text}, at) do
    id = id(given)
    batch(at, id, [%{insertText: %{text: "\n" <> text, endOfSegmentLocation: %{}}}])
    "Added it at the end of #{url(id)}"
  end

  defp run("replace", %{"documentId" => given, "find" => find} = inputs, at) do
    replaced =
      batch(at, id(given), [
        %{replaceAllText: %{containsText: %{text: find, matchCase: inputs["matchCase"] == true}, replaceText: inputs["replace"] || ""}}
      ])

    case get_in(replaced, ["replies", Access.at(0), "replaceAllText", "occurrencesChanged"]) || 0 do
      0 -> ~s|Found no "#{find}": nothing changed. google-docs_read with format text shows what's there.|
      1 -> "Replaced 1 match."
      count -> "Replaced #{count} matches."
    end
  end

  defp batch(at, id, requests),
    do: api(at, "POST", "documents/#{path(at, id)}:batchUpdate", json: %{requests: requests}, missing: no_document(id))

  # A document's body as markdown, or text: its paragraphs, lists and tables, in order.
  defp render(document, markdown) do
    {lines, _counters} =
      Enum.reduce(blocks(get_in(document, ["body", "content"]) || [], true), {[], %{}}, fn block, {lines, counters} ->
        case block do
          {:paragraph, paragraph} -> paragraph(document, paragraph, markdown, lines, counters)
          {:table, table} -> {lines ++ table(table, markdown), %{}}
        end
      end)

    lines |> Enum.join("\n") |> String.replace(~r/\n{3,}/, "\n\n") |> String.trim()
  end

  # The paragraphs and tables in a body, and those in its first table of contents.
  defp blocks(content, top) do
    Enum.flat_map(content, fn element ->
      cond do
        element["paragraph"] -> [{:paragraph, element["paragraph"]}]
        element["table"] -> [{:table, element["table"]}]
        element["tableOfContents"] && top -> blocks(element["tableOfContents"]["content"] || [], false)
        true -> []
      end
    end)
  end

  defp paragraph(document, paragraph, markdown, lines, counters) do
    text = runs(paragraph, markdown)

    case paragraph["bullet"] do
      %{"listId" => list} = bullet ->
        level = bullet["nestingLevel"] || 0
        glyph = get_in(document, ["lists", list, "listProperties", "nestingLevels", Access.at(level), "glyphType"]) || ""
        counters = Map.update(counters, {list, level}, 1, &(&1 + 1))
        mark = if glyph =~ @ordered, do: "#{counters[{list, level}]}.", else: "-"
        {lines ++ ["#{String.duplicate("  ", level)}#{mark} #{text}"], counters}

      _ ->
        style = get_in(paragraph, ["paragraphStyle", "namedStyleType"]) || ""

        level =
          case {Regex.run(~r/^HEADING_(\d)$/, style), style} do
            {[_all, level], _} -> String.to_integer(level)
            {nil, "TITLE"} -> 1
            {nil, "SUBTITLE"} -> 2
            _ -> nil
          end

        line = if markdown and level && String.trim(text) != "", do: "#{String.duplicate("#", level)} #{text}", else: text
        {lines ++ [line], %{}}
    end
  end

  defp table(table, markdown) do
    rows =
      for row <- table["tableRows"] || [] do
        for cell <- row["tableCells"] || [] do
          cell["content"]
          |> Enum.filter(& &1["paragraph"])
          |> Enum.map_join(if(markdown, do: "<br>", else: " "), &runs(&1["paragraph"], markdown))
          |> then(&if(markdown, do: String.replace(&1, "|", "\\|"), else: &1))
          |> String.trim()
        end
      end

    case {markdown, rows} do
      {true, [head | rest]} ->
        ["", "| #{Enum.join(head, " | ")} |", "| #{Enum.map_join(head, " | ", fn _ -> "---" end)} |"] ++
          Enum.map(rest, &"| #{Enum.join(&1, " | ")} |") ++ [""]

      _ ->
        Enum.map(rows, &Enum.join(&1, "\t"))
    end
  end

  # A paragraph's text, with its bold, italic, struck-through, code and links in markdown.
  defp runs(paragraph, markdown) do
    Enum.map_join(paragraph["elements"] || [], fn element ->
      cond do
        run = element["textRun"] ->
          text = run["content"] |> String.replace(~r/\n$/, "") |> String.replace("\v", "\n")
          if markdown and String.trim(text) != "", do: styled(text, run["textStyle"] || %{}), else: text

        person = element["person"] ->
          get_in(person, ["personProperties", "name"]) || get_in(person, ["personProperties", "email"]) || ""

        link = element["richLink"] ->
          properties = link["richLinkProperties"] || %{}
          if markdown, do: "[#{properties["title"]}](#{properties["uri"]})", else: properties["uri"] || ""

        true ->
          ""
      end
    end)
  end

  defp styled(text, style) do
    [_all, lead, core, trail] = Regex.run(~r/^(\s*)(.*?)(\s*)$/s, text)
    font = get_in(style, ["weightedFontFamily", "fontFamily"]) || ""

    core =
      if font =~ ~r/mono|courier|consol/i do
        "`#{core}`"
      else
        core
        |> then(&if(style["bold"], do: "**#{&1}**", else: &1))
        |> then(&if(style["italic"], do: "*#{&1}*", else: &1))
        |> then(&if(style["strikethrough"], do: "~~#{&1}~~", else: &1))
      end

    core = if url = get_in(style, ["link", "url"]), do: "[#{core}](#{url})", else: core
    lead <> core <> trail
  end

  defp id(given) do
    given = String.trim(given)
    if given == "", do: raise("Give documentId: the document's id or address.")

    case Regex.run(~r/\/d\/([a-zA-Z0-9_-]+)/, given) do
      [_all, id] -> id
      nil -> given
    end
  end

  defp url(id), do: "https://docs.google.com/document/d/#{id}/edit"

  defp no_document(id),
    do: "No document #{id}: give its id or its address (docs.google.com/document/d/...). google-drive_files_list finds documents."

  defp api(at, method, path, options),
    do: google(at.call).request(at.call, google(), at.email, method, @api <> path, options)

  defp path(at, part), do: google(at.call).path(part)
  defp google(call), do: OneCore.Call.plugin(call, "google-account")
end
