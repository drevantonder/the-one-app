defmodule GoogleAccount do
  @moduledoc """
  What the Google apps (gmail, google-calendar, google-drive, google-docs and google-sheets) share: their person's
  Google accounts and Google's sign-in, and their calls to Google's APIs. A Google app calls `request`, which acts
  for the app that called it (its `caller`), which it names by its title.

  Each Google account is one item of its person's Vault, "Google <address>", of this app's type (`vault.types`), and
  every Google app uses it through this one: it's switched on for this app alone, which signs it in, with what
  every Google app they have asks of Google (`scopes`), and makes each call to Google with it. So neither this
  app nor a Google app is ever handed a token. Signing in to it again for one app adds that app's scopes
  (Google's incremental authorization). Signing out is deleting the item, which soft delete does; one Google says needs
  signing in again is its person's to act on (`state`), with Sign In.

  Every Google address is written here and in the Google apps as Google's own: a test points them at a fake Google by
  rewriting them.
  """
  @behaviour OneCore.App

  # Google's OAuth: https://developers.google.com/identity/protocols/oauth2/web-server. Every API host a Google
  # app calls is the account's, since one account's tokens serve every app signed in to it.
  @service %{
    "name" => "Google",
    "authorize" => "https://accounts.google.com/o/oauth2/v2/auth",
    "token" => "https://oauth2.googleapis.com/token",
    "hosts" =>
      ~w(gmail.googleapis.com www.googleapis.com docs.googleapis.com sheets.googleapis.com openidconnect.googleapis.com),
    "client" => "Google client"
  }
  @userinfo "https://openidconnect.googleapis.com/v1/userinfo"

  @impl true
  def requires, do: %{"core" => ">= 0.22.18", "vault" => "to keep your Google sign-ins"}

  # It has no app, and its Google accounts are listed in the Vault, so it draws itself.
  @impl true
  def icon, do: "icon.png"

  # A Google account is an item of the person's Vault, listed under Accounts, and added by signing in.
  @impl true
  def hooks, do: ["vault.types"]

  @impl true
  def hook("vault.types", _about, _call),
    do: [%{name: "Google account", section: "Accounts", prefix: "Google ", create: %{tool: "sign_in"}}]

  # What each Google app asks of Google, so one sign-in lets every one in.
  @impl true
  def offers do
    [
      %{
        name: "scopes",
        kind: :collect,
        description:
          "What a Google app asks of Google. It's about nothing. A Google app answers with its list of Google's scopes, which a Google account is signed in with."
      }
    ]
  end

  @impl true
  def tools do
    string = &%{type: "string", description: &1}
    account = string.("Which account: its address, or a part of it only it has. The first if left out")
    object = &%{type: "object", properties: &1, required: &2}
    # What a Google app's code gives as it has it: an input that's null is left out.
    nullable = &Map.update!(&1, :type, fn type -> [type, "null"] end)

    [
      %{
        name: "apps",
        read_only: true,
        description:
          "The Google apps you have (Gmail, Google Calendar, Drive, Docs, Sheets), each by the name its tools start with. Every one uses the accounts google-account_accounts lists.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            apps: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: string.("The name its tools start with: gmail"),
                  title: string.("Its name, as a person reads it: Gmail")
                },
                required: ["name", "title"]
              }
            }
          },
          required: ["apps"]
        }
      },
      %{
        name: "accounts",
        read_only: true,
        description:
          "The Google accounts signed in, which every Google app you have uses, the default first: each with ok, and why when it's false (google-account_sign_in with that account fixes it). Every Google app's account input takes one of these.",
        inputs: object.(%{}, []),
        outputs: %{
          type: "object",
          properties: %{
            accounts: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  account: string.("Its address"),
                  ok: %{type: "boolean", description: "Whether the app can use it now"},
                  why: string.("Why it can't, and what fixes it, when ok is false")
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
          "Starts signing a Google account in, for every Google app you have, or with `account`, signing that one in again, and gives the url to open, where Google signs you in. A Google app's code says its scopes.",
        inputs: %{
          type: "object",
          properties: %{
            scopes: %{type: "array", items: %{type: "string"}, description: "What the app asks of Google"},
            account: string.("The account's address, to suggest it to Google")
          }
        },
        outputs: %{
          type: "object",
          properties: %{url: string.("Where its person signs in"), then: string.("What happens next")},
          required: ["url", "then"]
        }
      },
      %{
        name: "request",
        description: """
        For a Google app's code: calls a Google API as one of the accounts signed in for the app that calls this,
        and gives its status, headers and body, decoded where it's JSON. An answer that isn't a success raises,
        saying what went wrong for whoever reads it. An input that's null is left out.
        """,
        inputs:
          object.(
            %{
              account: nullable.(account),
              method: nullable.(string.("GET if left out")),
              url: string.("The API's address"),
              query:
                nullable.(%{
                  type: "object",
                  description: "The query, by name: a list is given once for each, and null or \"\" not at all"
                }),
              json: %{description: "The body, as JSON"},
              body: nullable.(string.("Or the body as text, with type. Neither may say {{...}}: base64 it")),
              type: nullable.(string.("The body's MIME type")),
              headers: nullable.(%{type: "object", additionalProperties: %{type: "string"}, description: "More headers"}),
              missing: nullable.(string.("What to say when what the URL names isn't there, or the account can't open it")),
              hint: nullable.(string.("What to add to what Google says about an input it can't take"))
            },
            ["url"]
          ),
        outputs: %{
          type: "object",
          properties: %{
            status: %{type: "integer"},
            headers: %{type: "object", additionalProperties: %{type: "string"}},
            body: %{description: "The body: JSON decoded, other text as it is, or null for none"}
          },
          required: ["status", "headers", "body"]
        }
      }
    ]
  end

  @impl true
  def call("apps", _inputs, call) do
    apps =
      for {name, version} <- OneCore.Apps.running(call.subject),
          is_map_key(version.requires, "google-account"),
          do: %{name: name, title: version.title}

    %{apps: Enum.sort_by(apps, & &1.title)}
  end

  def call("request", _inputs, %OneCore.Call{caller: nil}),
    do: raise("google-account_request is for the Google apps' code. Each has its own tools.")

  def call("accounts", _inputs, call) do
    accounts =
      for email <- emails(call) do
        case vault(call, "fetch", Map.put(get(@userinfo), "item", item(email))) do
          %{"status" => 200} ->
            state(call, email, :ready)
            %{account: email, ok: true}

          response ->
            why = trouble(call, email, response)
            again(call, email, response)
            %{account: email, ok: false, why: why}
        end
      end

    %{accounts: accounts}
  end

  # Signed in for a Google app, with what it asks of Google, or for its person, with what every Google app they
  # have asks: so adding an account in the Vault lets every one in at once.
  def call("sign_in", inputs, call) do
    {scopes, apps} =
      if call.caller do
        {inputs["scopes"] || raise("A Google app signs in with its scopes."), [call.caller]}
      else
        {:ok, parts, _failed} = OneCore.Call.run_hook(call, "scopes", %{})
        {parts |> Map.values() |> List.flatten() |> Enum.uniq(), Map.keys(parts)}
      end

    # The apps it's for, as people know them, which the page Google sends them back to names.
    running = OneCore.Apps.running(call.subject)
    titles = Map.new(running, &{elem(&1, 0), elem(&1, 1).title}) |> Map.merge(Map.new(running, &{elem(&1, 1).artifact, elem(&1, 1).title}))
    apps = apps |> Enum.map(&titles[&1]) |> Enum.reject(&is_nil/1) |> Enum.sort()

    params = %{"access_type" => "offline", "prompt" => "consent", "include_granted_scopes" => "true"}
    params = if hint = inputs["account"], do: Map.put(params, "login_hint", hint), else: params
    service = Map.merge(@service, %{"scopes" => Enum.uniq(["openid", "email" | scopes]), "params" => params})

    %{
      url: vault(call, "sign_in", %{"service" => service, "item" => "Google {{email}}", "apps" => apps})["url"],
      then:
        "They open url in their browser and sign in, and Google sends them back to the network's address, which finishes it. The account is then in their Vault, under Accounts."
    }
  end

  def call("request", inputs, call) do
    %{"url" => url} = inputs = Map.reject(inputs, fn {_input, value} -> value == nil end)
    email = account(call, inputs["account"])

    {body, type} =
      if Map.has_key?(inputs, "json"),
        do: {unfilled(JSON.encode!(inputs["json"])), "application/json"},
        else: {unfilled!(inputs["body"]), inputs["type"]}

    query =
      for {key, values} <- inputs["query"] || %{},
          value <- List.wrap(values),
          value not in [nil, ""],
          do: {key, value}

    url = if query == [], do: url, else: url <> "?" <> URI.encode_query(query)
    headers = Map.merge(if(type, do: %{"content-type" => type}, else: %{}), inputs["headers"] || %{})

    request = %{
      "item" => item(email),
      "url" => url,
      "method" => inputs["method"] || "GET",
      "headers" => Map.put(headers, "authorization", "Bearer {{access_token}}")
    }

    request = if body, do: Map.put(request, "body", body), else: request

    response = vault(call, "fetch", request)

    if response["status"] not in 200..299 do
      again(call, email, response)
      raise trouble(call, email, response, Map.take(inputs, ["missing", "hint"]))
    end

    %{status: response["status"], headers: response["headers"], body: decoded(response["body"])}
  end

  defp decoded(nil), do: nil
  defp decoded(""), do: nil

  defp decoded(body) do
    case JSON.decode(body) do
      {:ok, json} -> json
      {:error, _} -> body
    end
  end

  # The address of the account a call names, as `accounts` lists them: the whole address, or a part only it has.
  # The first when it names none.
  defp account(call, name) do
    case {emails(call), name} do
      {[], _} ->
        raise "Sign in to a Google account first: google-account_sign_in gives the link."

      {[first | _], blank} when blank in [nil, ""] ->
        first

      {emails, name} ->
        wanted = String.downcase(name)

        case Enum.find(emails, &(String.downcase(&1) == wanted)) ||
               Enum.filter(emails, &String.contains?(String.downcase(&1), wanted)) do
          email when is_binary(email) -> email
          [email] -> email
          _ -> raise "No one account matches \"#{name}\". They are: #{Enum.join(emails, ", ")}."
        end
    end
  end

  # The Vault fills {{<field>}} in wherever it's written in a body, so what a person or an agent wrote, as an
  # email or a cell, must never say one. In JSON a { inside a string can be written as {, and a { that
  # starts an object is never followed by another.
  defp unfilled(json), do: String.replace(json, ~r/\{(?=\{)/, fn _brace -> "\\u007b" end)

  defp unfilled!(nil), do: nil

  defp unfilled!(body) do
    if body =~ ~r/\{\{[A-Za-z0-9_]+\}\}/, do: raise("A body sent to Google can't say {{...}}: base64 it.")
    body
  end

  # What went wrong with a call, said for whoever's reading.
  defp trouble(call, email, response, hints \\ %{}) do
    title = title(call)
    status = response["status"]
    message = message(response["body"]) || "#{status}"
    again = "#{email} needs signing in again#{if call.caller, do: ", to let #{title} in"}: google-account_sign_in with account #{email}."

    cond do
      status == 401 ->
        again

      status == 403 and message =~ ~r/insufficient|scope/i ->
        again

      # Gmail answers an id that was made up with 400 "Invalid id value", and Calendar a deleted event with 410.
      hints["missing"] && (status in [404, 410] or message =~ ~r/invalid id value/i) ->
        hints["missing"]

      hints["missing"] && status == 403 ->
        "#{email} can't open it (#{String.trim_trailing(message, ".")}): use the account it's shared with (google-account_accounts lists them), or ask its owner to share it."

      hints["hint"] && status == 400 ->
        "#{title}: #{String.trim_trailing(message, ".")}: #{hints["hint"]}"

      true ->
        "#{title}: #{message}"
    end
  end

  defp message(body) when is_binary(body) do
    case JSON.decode(body) do
      {:ok, %{"error" => %{"message" => message}}} when is_binary(message) -> message
      {:ok, %{"error_description" => message}} when is_binary(message) -> message
      {:ok, %{"error" => message}} when is_binary(message) -> message
      _ -> nil
    end
  end

  defp message(_body), do: nil

  # The addresses of the Google accounts signed in, by address.
  defp emails(call) do
    for %{"name" => "Google " <> email, "sign_in" => "Google"} <- vault(call, "items", %{})["items"], do: email
  end

  # A Google account that Google says needs signing in again, or more allowed of it, is its person's to act on, in
  # the Vault: with Sign In, which signs it in again for every Google app they have. They aren't notified: the
  # call that found it says so to whoever made it, and the Vault shows the dot.
  defp again(call, email, response) do
    if response["status"] == 401 or (response["status"] == 403 and (message(response["body"]) || "") =~ ~r/insufficient|scope/i),
      do: state(call, email, :needs_action, "Sign in again", %{label: "Sign In", tool: "sign_in", inputs: %{account: email}})
  end

  defp state(call, email, state, message \\ nil, action \\ nil) do
    vault(call, "state", %{"item" => item(email), "state" => to_string(state), "message" => message, "action" => action} |> Map.reject(fn {_, value} -> value == nil end))
  end

  # What a person calls the app that called, or Google when a person asked.
  defp title(%OneCore.Call{caller: nil}), do: "Google"

  defp title(call) do
    Enum.find_value(OneCore.Apps.running(call.subject), fn {_name, version} ->
      version.artifact == call.caller && version.title
    end)
  end

  defp item(email), do: "Google #{email}"
  defp get(url), do: %{"url" => url, "headers" => %{"authorization" => "Bearer {{access_token}}"}}

  # The Vault's tools, as this app, which every Google account is switched on for.
  defp vault(call, tool, inputs), do: OneCore.Call.tool(call, "vault", tool, inputs)
end
