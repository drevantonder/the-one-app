defmodule GoogleAccount do
  @moduledoc """
  What the Google apps (gmail, google-calendar, google-drive, google-docs and google-sheets) share: Google's
  sign-in and each account signed in, kept in the Vault, and their calls to Google's APIs. An app calls this
  plugin's tools, which act for the app that called them (its `caller`), with the app's `title`, and `scopes` to
  sign in with.

  Each Google account is one item of its person's Vault, "Google <address>", shared by every Google app signed in
  to it. Signing in to it for another app adds that app's scopes to it (Google's incremental authorization) and
  switches the item on for that app too. Signing out of one app switches it off for that app alone. This plugin
  uses the Vault's tools on behalf of the app, so the Vault ties each account to the apps it's switched on for, and
  neither this plugin nor an app is ever handed a token: one in sandbox mode uses an account as safely as one
  that isn't.

  Every Google address is written here and in the apps as Google's own: a test points them at a fake Google by
  rewriting them.
  """
  @behaviour OneCore.Plugin

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
  def permissions, do: ~w(vault_fetch vault_items vault_sign_in vault_stop_using)

  @impl true
  def tools do
    string = &%{type: "string", description: &1}
    title = string.("The app's name, as a person reads it, like Gmail")
    account = string.("Which account: its address, or a part of it only it has. The first if left out")
    app = &%{type: "object", properties: Map.put(&1, :title, title), required: ["title" | &2]}

    [
      %{
        name: "apps",
        read_only: true,
        description:
          "The Google apps you have (Gmail, Google Calendar, Drive, Docs, Sheets), each by the name its tools start with. Each one's accounts tool lists its accounts.",
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
          "For a Google app's code: the Google accounts signed in for the app that calls this, each with ok, and why when it's false.",
        inputs: app.(%{}, []),
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
          "For a Google app's code: starts signing a Google account in to the app that calls this, and gives the url its person opens.",
        inputs:
          app.(
            %{
              scopes: %{type: "array", items: %{type: "string"}, description: "What the app asks of Google"},
              account: string.("The account's address, to suggest it to Google")
            },
            ["scopes"]
          ),
        outputs: %{
          type: "object",
          properties: %{url: string.("Where its person signs in"), then: string.("What happens next")},
          required: ["url", "then"]
        }
      },
      %{
        name: "sign_out",
        description: "For a Google app's code: signs a Google account out of the app that calls this.",
        inputs: app.(%{account: account}, ["account"]),
        outputs: %{type: "object", properties: %{account: string.("The account signed out")}, required: ["account"]}
      },
      %{
        name: "request",
        description: """
        For a Google app's code: calls a Google API as one of the accounts signed in for the app that calls this,
        and gives its status, headers and body, decoded where it's JSON. An answer that isn't a success raises,
        saying what went wrong for whoever reads it.
        """,
        inputs:
          app.(
            %{
              account: account,
              method: string.("GET if left out"),
              url: string.("The API's address"),
              query: %{
                type: "object",
                description: "The query, by name: a list is given once for each, and null or \"\" not at all"
              },
              json: %{description: "The body, as JSON"},
              body: string.("Or the body as text, with type. Neither may say {{...}}: base64 it"),
              type: string.("The body's MIME type"),
              headers: %{type: "object", additionalProperties: %{type: "string"}, description: "More headers"},
              missing: string.("What to say when what the URL names isn't there, or the account can't open it"),
              hint: string.("What to add to what Google says about an input it can't take")
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
      for {name, version} <- OneCore.Plugins.running(call.subject),
          "google-account" in version.permissions,
          do: %{name: name, title: version.title}

    %{apps: Enum.sort_by(apps, & &1.title)}
  end

  def call(_tool, _inputs, %OneCore.Call{caller: nil}),
    do:
      raise(
        "google-account's tools are for the Google apps' code. Each app has its own accounts, sign_in and sign_out."
      )

  def call("accounts", %{"title" => title}, call) do
    accounts =
      for email <- emails(call) do
        case vault(call, "fetch", Map.put(get(@userinfo), "item", item(email))) do
          %{"status" => 200} -> %{account: email, ok: true}
          response -> %{account: email, ok: false, why: trouble(call, title, email, response)}
        end
      end

    %{accounts: accounts}
  end

  def call("sign_in", %{"scopes" => scopes} = inputs, call) do
    params = %{"access_type" => "offline", "prompt" => "consent", "include_granted_scopes" => "true"}
    params = if hint = inputs["account"], do: Map.put(params, "login_hint", hint), else: params
    service = Map.merge(@service, %{"scopes" => ["openid", "email" | scopes], "params" => params})

    %{
      url: vault(call, "sign_in", %{"service" => service, "item" => "Google {{email}}"})["url"],
      then:
        "They open url in their browser and sign in, and Google sends them back to the network's address, which finishes it. #{app(call)}_accounts then lists the account."
    }
  end

  def call("sign_out", %{"account" => name}, call) do
    email = account(call, name)
    vault(call, "stop_using", %{"item" => item(email)})
    %{account: email}
  end

  def call("request", %{"url" => url, "title" => title} = inputs, call) do
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

    if response["status"] not in 200..299,
      do: raise(trouble(call, title, email, response, Map.take(inputs, ["missing", "hint"])))

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
        raise "Sign in to a Google account first: #{app(call)}_sign_in gives the link."

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
  defp trouble(call, title, email, response, hints \\ %{}) do
    status = response["status"]
    message = message(response["body"]) || "#{status}"
    again = "#{email} needs signing in again, to let #{title} in: #{app(call)}_sign_in with account #{email}."

    cond do
      status == 401 ->
        again

      status == 403 and message =~ ~r/insufficient|scope/i ->
        again

      # Gmail answers an id that was made up with 400 "Invalid id value", and Calendar a deleted event with 410.
      hints["missing"] && (status in [404, 410] or message =~ ~r/invalid id value/i) ->
        hints["missing"]

      hints["missing"] && status == 403 ->
        "#{email} can't open it (#{String.trim_trailing(message, ".")}): use the account it's shared with (#{app(call)}_accounts lists them), or ask its owner to share it."

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

  # The addresses of the Google accounts signed in for the app that called, by address.
  defp emails(call) do
    for %{"name" => "Google " <> email, "sign_in" => "Google"} <- vault(call, "items", %{})["items"], do: email
  end

  # The app that called, by the name its tools start with.
  defp app(call), do: Path.basename(call.caller)

  defp item(email), do: "Google #{email}"
  defp get(url), do: %{"url" => url, "headers" => %{"authorization" => "Bearer {{access_token}}"}}

  # The Vault's tools, for the app that called.
  defp vault(call, tool, inputs), do: OneCore.Call.tool(call, "vault", tool, inputs, on_behalf_of: call.caller)
end
