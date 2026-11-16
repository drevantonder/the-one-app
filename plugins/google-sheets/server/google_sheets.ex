defmodule GoogleSheets do
  @moduledoc """
  Google Sheets: the person's own spreadsheets, through Sheets' API (v4). Signing in and the accounts are the
  `google-account` plugin's, the same for every Google plugin.
  """
  @behaviour OneCore.Plugin

  @api "https://sheets.googleapis.com/v4/"
  @range_hint "ranges are A1 notation, like Budget!A1:F20, Budget!A:C or Budget; google-sheets_spreadsheet lists the sheets"

  # It has accounts and no app, so it names and draws itself, in Accounts and the App Store.
  @impl true
  def title, do: google().title

  @impl true
  def icon, do: "icon.png"

  @doc "What Google Sheets asks of Google, for the `google-account` plugin."
  def google do
    %{
      title: "Google Sheets",
      scopes: ["https://www.googleapis.com/auth/spreadsheets"]
    }
  end

  @impl true
  def permissions, do: ["google-account"]

  @impl true
  def tools do
    string = &%{type: "string", description: &1}
    account = string.("Which account: its address, or a part of it only it has, as google-sheets_accounts lists them. The first if left out")

    with_account = fn properties, required ->
      %{type: "object", properties: Map.put(properties, :account, account), required: required}
    end

    spreadsheet =
      string.(
        "The spreadsheet's id, or its address, like https://docs.google.com/spreadsheets/d/1AbC.../edit. google-drive_files_list finds spreadsheets (q mimeType = 'application/vnd.google-apps.spreadsheet' and name contains '...')"
      )

    rows = "rows, each an array of values, like [[\"Date\", \"Amount\"], [\"2026-11-03\", 42]]. Text is read as if typed (valueInputOption USER_ENTERED), so =SUM(A1:A3) is a formula and 3/11 a date"
    values = %{type: "array", items: %{type: "array", items: %{}}, description: String.capitalize(rows)}

    [
      %{
        name: "accounts",
        read_only: true,
        description:
          "The Google accounts Google Sheets can use, the default first, each with ok, and why when it's false (google-sheets_sign_in with that account fixes it). Every other tool's account takes one of these.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            accounts: %{
              type: "array",
              description: "The default first",
              items: %{
                type: "object",
                properties: %{
                  account: string.("Its address"),
                  ok: %{type: "boolean", description: "Whether it can be used now"},
                  why: string.("Why not, when ok is false: sign_in with that account fixes it")
                },
                required: ["account", "ok"]
              }
            }
          },
          required: ["accounts"]
        }
      },
      %{
        name: "sign_in",
        description:
          "Starts signing a Google account in to Google Sheets, or letting it use one another Google plugin has: gives url, for the person to open in their browser, where Google signs them in and sends them back to finish.",
        inputs: %{type: "object", properties: %{account: string.("The account's address, to suggest it to Google: for one that needs signing in again, as google-sheets_accounts lists it")}},
        outputs: %{
          type: "object",
          properties: %{url: string.("Where the person signs in, in their browser"), then: string.("What happens next")},
          required: ["url", "then"]
        }
      },
      %{
        name: "sign_out",
        description: "Signs a Google account out of Google Sheets. Other Google plugins that use it still do, and google-sheets_sign_in signs it in again.",
        inputs: with_account.(%{}, ["account"]),
        outputs: %{type: "object", properties: %{account: string.("The account signed out")}, required: ["account"]}
      },
      %{
        name: "spreadsheet",
        read_only: true,
        description:
          "A spreadsheet's title, address, time zone and sheets (tabs), each with its size (Sheets' spreadsheets.get, trimmed): {id, title, url, timeZone, sheets: [{title, id, rows, columns}]}. Sheet titles are what ranges start with.",
        inputs: with_account.(%{spreadsheetId: spreadsheet}, ["spreadsheetId"]),
        outputs: %{
          type: "object",
          properties: %{
            id: string.("The spreadsheet's id"),
            title: %{type: "string"},
            url: string.("Its address"),
            timeZone: string.("An IANA time zone"),
            sheets: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  title: string.("What ranges start with"),
                  id: %{type: "integer"},
                  rows: %{type: ["integer", "null"]},
                  columns: %{type: ["integer", "null"]}
                },
                required: ["title", "id", "rows", "columns"]
              }
            }
          },
          required: ["id", "title", "url", "timeZone", "sheets"]
        }
      },
      %{
        name: "read",
        read_only: true,
        description:
          "The values in a range (Sheets' values.get): {range, values, more}, values a row per array, with the range they came from, and more how many rows past limit were left out.",
        inputs:
          with_account.(
            %{
              spreadsheetId: spreadsheet,
              range: string.("A1 notation, like Budget!A1:F20, Budget!A:C or Budget. The whole first sheet if left out"),
              valueRenderOption: %{
                type: "string",
                enum: ["FORMATTED_VALUE", "UNFORMATTED_VALUE", "FORMULA"],
                description: "FORMATTED_VALUE: as shown, the default. UNFORMATTED_VALUE: numbers unformatted. FORMULA: formulas, like =SUM(B2:B9)"
              },
              limit: %{type: "integer", description: "At most this many rows. 500 if left out, and more says how many were left out"}
            },
            ["spreadsheetId"]
          ),
        outputs: %{
          type: "object",
          properties: %{
            range: string.("The range they came from, in A1 notation"),
            values: %{type: "array", items: %{type: "array"}, description: "A row per array, as valueRenderOption asked"},
            more: %{type: "integer", description: "How many rows were left out past limit: read a smaller range for them"}
          },
          required: ["range", "values", "more"]
        }
      },
      %{
        name: "append",
        description:
          "Adds rows after the table in a range, or the first sheet (Sheets' values.append), and says where they went. This changes the person's spreadsheet: do it only when they asked.",
        inputs:
          with_account.(
            %{
              spreadsheetId: spreadsheet,
              range: string.("The table's range or sheet, like Expenses or Expenses!A:E. The first sheet if left out"),
              values: values
            },
            ["spreadsheetId", "values"]
          ),
        outputs: %{
          type: "object",
          properties: %{updatedRange: string.("Where they went, in A1 notation"), updatedRows: %{type: "integer"}},
          required: ["updatedRange", "updatedRows"]
        }
      },
      %{
        name: "update",
        description:
          "Writes rows over a range, from its top-left cell, replacing what was there (Sheets' values.update), and says which cells changed. Do it only when the person asked.",
        inputs:
          with_account.(
            %{spreadsheetId: spreadsheet, range: string.("Where to write, in A1 notation, like Budget!B2 or Budget!B2:D5"), values: values},
            ["spreadsheetId", "range", "values"]
          ),
        outputs: %{
          type: "object",
          properties: %{updatedRange: string.("The cells written, in A1 notation"), updatedCells: %{type: "integer"}},
          required: ["updatedRange", "updatedCells"]
        }
      },
      %{
        name: "create",
        description:
          "Makes a spreadsheet in the person's Drive (in My Drive, Sheets' spreadsheets.create), with sheets and first rows if given. Gives {id, title, url}.",
        inputs:
          with_account.(
            %{
              title: string.("Its title, like Budget 2026"),
              sheets: %{type: "array", items: %{type: "string"}, description: ~s|Its sheets' (tabs') titles, like ["Income", "Expenses"]. One, Sheet1, if left out|},
              values: %{values | description: "Rows to put in the first sheet, from A1: #{rows}"}
            },
            ["title"]
          ),
        outputs: %{
          type: "object",
          properties: %{id: string.("The spreadsheet's id"), title: %{type: "string"}, url: string.("Its address")},
          required: ["id", "title", "url"]
        }
      }
    ]
  end

  @impl true
  def call(tool, inputs, call) when tool in ~w(accounts sign_in sign_out),
    do: google(call, tool, Map.put(inputs, "scopes", google().scopes))

  def call(tool, inputs, call) do
    run(tool, inputs, %{call: call, account: inputs["account"]})
  end

  defp run("spreadsheet", %{"spreadsheetId" => given}, at) do
    sheet =
      api(at, "GET", spreadsheet(at, given),
        query: [fields: "spreadsheetId,spreadsheetUrl,properties(title,timeZone,locale),sheets.properties(sheetId,title,index,gridProperties)"],
        missing: no_spreadsheet(given)
      )

    %{
      id: sheet["spreadsheetId"],
      title: sheet["properties"]["title"],
      url: sheet["spreadsheetUrl"],
      timeZone: sheet["properties"]["timeZone"],
      sheets:
        for %{"properties" => properties} <- sheet["sheets"] || [] do
          %{
            title: properties["title"],
            id: properties["sheetId"],
            rows: get_in(properties, ["gridProperties", "rowCount"]),
            columns: get_in(properties, ["gridProperties", "columnCount"])
          }
        end
    }
  end

  defp run("read", %{"spreadsheetId" => given} = inputs, at) do
    range = range(at, given, inputs["range"])

    read =
      api(at, "GET", "#{spreadsheet(at, given)}/values/#{path(at, range)}",
        query: [valueRenderOption: inputs["valueRenderOption"] || "FORMATTED_VALUE"],
        missing: no_spreadsheet(given),
        hint: @range_hint
      )

    rows = read["values"] || []
    most = inputs["limit"] || 500
    %{range: read["range"], values: Enum.take(rows, most), more: max(length(rows) - most, 0)}
  end

  defp run("append", %{"spreadsheetId" => given, "values" => rows} = inputs, at) do
    range = range(at, given, inputs["range"])

    appended =
      api(at, "POST", "#{spreadsheet(at, given)}/values/#{path(at, range)}:append",
        query: [valueInputOption: "USER_ENTERED", insertDataOption: "INSERT_ROWS"],
        json: %{values: rows},
        missing: no_spreadsheet(given),
        hint: @range_hint
      )

    %{
      updatedRange: get_in(appended, ["updates", "updatedRange"]),
      updatedRows: get_in(appended, ["updates", "updatedRows"]) || length(rows)
    }
  end

  defp run("update", %{"spreadsheetId" => given, "range" => range, "values" => rows}, at) do
    updated =
      api(at, "PUT", "#{spreadsheet(at, given)}/values/#{path(at, range)}",
        query: [valueInputOption: "USER_ENTERED"],
        json: %{values: rows},
        missing: no_spreadsheet(given),
        hint: @range_hint
      )

    Map.take(updated, ["updatedRange", "updatedCells"])
  end

  defp run("create", %{"title" => title} = inputs, at) do
    tabs = for tab <- inputs["sheets"] || [], do: %{properties: %{title: tab}}
    body = if tabs == [], do: %{properties: %{title: title}}, else: %{properties: %{title: title}, sheets: tabs}
    sheet = api(at, "POST", "spreadsheets", json: body)

    if (rows = inputs["values"]) not in [nil, []] do
      first = "#{quoted(hd(sheet["sheets"])["properties"]["title"])}!A1"

      api(at, "PUT", "#{spreadsheet(at, sheet["spreadsheetId"])}/values/#{path(at, first)}",
        query: [valueInputOption: "USER_ENTERED"],
        json: %{values: rows}
      )
    end

    %{id: sheet["spreadsheetId"], title: sheet["properties"]["title"], url: sheet["spreadsheetUrl"]}
  end

  # The range given, or the whole first sheet.
  defp range(_at, _given, range) when is_binary(range) and range != "", do: range

  defp range(at, given, _range) do
    sheet = api(at, "GET", spreadsheet(at, given), query: [fields: "sheets.properties.title"], missing: no_spreadsheet(given))
    quoted(hd(sheet["sheets"])["properties"]["title"])
  end

  defp quoted(title), do: "'#{String.replace(title, "'", "''")}'"

  defp spreadsheet(at, given), do: "spreadsheets/#{path(at, id(given))}"

  defp id(given) do
    given = String.trim(given)
    if given == "", do: raise("Give spreadsheetId: the spreadsheet's id or address.")

    case Regex.run(~r/\/d\/([a-zA-Z0-9_-]+)/, given) do
      [_all, id] -> id
      nil -> given
    end
  end

  defp no_spreadsheet(given),
    do:
      "No spreadsheet #{id(given)}: give its id or its address (docs.google.com/spreadsheets/d/...). google-drive_files_list finds spreadsheets."

  defp api(at, method, path, options), do: request(at, method, @api <> path, options)["body"]

  # A part of a URL's path, as Google takes it: an id or a range, with anything but letters and digits escaped.
  defp path(_at, part), do: URI.encode(to_string(part), &URI.char_unreserved?/1)

  # Google's API as the account `at` names, through google-account, which gives the answer's status, headers and
  # body, or raises with what went wrong.
  defp request(at, method, url, options) do
    given = for {option, value} <- options, value != nil, into: %{}, do: {to_string(option), value}
    inputs = Map.merge(given, %{"method" => method, "url" => url})
    google(at.call, "request", if(at.account, do: Map.put(inputs, "account", at.account), else: inputs))
  end

  # Google's sign-in, the accounts and Google's API are google-account's, whose tools act for this plugin.
  defp google(call, tool, inputs) do
    inputs = Map.new(Map.put(inputs, "title", google().title), &query/1)
    OneCore.Call.tool(call, "google-account", tool, JSON.decode!(JSON.encode!(inputs)))
  end

  defp query({"query", query}), do: {"query", Map.new(query)}
  defp query(input), do: input
end
