defmodule Google do
  @moduledoc """
  What the Google apps (gmail, google-calendar, drive, docs and sheets) share: Google's sign-in and each account
  signed in, kept in the Vault, and their calls to Google's APIs. An app uses this plugin, says what it asks of
  Google in its public `google/0` (`%{title: "Gmail", about: "...", scopes: [...]}`), and passes that here with
  its call.

  Each Google account is one item of its person's Vault, "Google <address>", shared by every Google app signed in
  to it. Signing in to it for another app adds that app's scopes to it (Google's incremental authorization) and
  switches the item on for that app too. Signing out of one app switches it off for that app alone.

  Every Google address is written here and in the apps as Google's own: a test points them at a fake Google by
  rewriting them.
  """
  @behaviour OneCore.Plugin

  # Google's OAuth: https://developers.google.com/identity/protocols/oauth2/web-server. Every API host a Google
  # app calls is the account's, since one account's tokens serve every app signed in to it.
  @service %{
    name: "Google",
    authorize: "https://accounts.google.com/o/oauth2/v2/auth",
    token: "https://oauth2.googleapis.com/token",
    hosts: ~w(gmail.googleapis.com www.googleapis.com docs.googleapis.com sheets.googleapis.com openidconnect.googleapis.com),
    client: "Google client"
  }
  @userinfo "https://openidconnect.googleapis.com/v1/userinfo"

  @impl true
  def uses, do: ["vault"]

  @impl true
  def apps, do: [%{name: "Google", page: "client/index.html", icon: "client/icon.svg", account: true}]

  @impl true
  def tools do
    [
      %{
        name: "apps",
        read_only: true,
        description:
          "The Google apps you have (Gmail, Google Calendar, Drive, Docs, Sheets), each by the name its tools start with. Each one's accounts tool lists its accounts.",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "accounts",
        read_only: true,
        description:
          "Your Google accounts, as every Google app you have signs in to them: each { account, ok, about }, about the apps that use it, with problem when one of them needs it signed in again. Each app's own accounts tool says why.",
        inputs: %{type: "object", properties: %{}}
      }
    ]
  end

  @impl true
  def call("apps", _inputs, call) do
    apps =
      for {name, version} <- OneCore.Plugins.running(call.subject),
          "google" in version.uses,
          function_exported?(version.plugin, :google, 0),
          do: Map.put(Map.take(version.plugin.google(), [:title, :about]), :name, name)

    Enum.sort_by(apps, & &1.title)
  end

  def call("accounts", _inputs, call) do
    using =
      for app <- call("apps", %{}, call),
          account <- OneCore.Call.tool(call, app.name, "accounts", %{}),
          do: {account["account"], app, account["ok"]}

    for {email, uses} <- Enum.group_by(using, &elem(&1, 0), &Tuple.delete_at(&1, 0)) do
      titles = Enum.map_join(uses, ", ", fn {app, _ok} -> app.title end)
      account = %{account: email, ok: Enum.all?(uses, &elem(&1, 1)), about: titles}
      if account.ok, do: account, else: Map.put(account, :problem, "Sign in again")
    end
  end

  @doc """
  An app's `accounts`, `sign_in` and `sign_out` tools, which every Google app has: `call` is the app's, and `app`
  its `google/0`.
  """
  def account_call("accounts", _inputs, call, app) do
    for email <- emails(call) do
      case vault(call).fetch(call, item(email), get(@userinfo)) do
        %{status: 200} -> %{account: email, ok: true}
        answer -> %{account: email, ok: false, why: trouble(call, app, email, answer)}
      end
    end
  end

  def account_call("sign_in", inputs, call, app) do
    params = %{"access_type" => "offline", "prompt" => "consent", "include_granted_scopes" => "true"}
    params = if hint = inputs["account"], do: Map.put(params, "login_hint", hint), else: params
    service = Map.merge(@service, %{scopes: ["openid", "email" | app.scopes], params: params})

    %{
      url: vault(call).sign_in(call, service, "Google {{email}}"),
      then:
        "They open url in their browser and sign in, and Google sends them back to the network's address, which finishes it. #{call.plugin}_accounts then lists the account."
    }
  end

  def account_call("sign_out", %{"account" => name}, call, app) do
    email = account(call, name)
    vault(call).stop_using(call, item(email))
    "Signed #{email} out of #{app.title}. Other Google apps that use it still do."
  end

  @doc "The address of the account a call names, as `accounts` lists them: the whole address, or a part only it has. The first when it names none."
  def account(call, name \\ nil) do
    case {emails(call), name} do
      {[], _} ->
        raise "Sign in to a Google account first: #{call.plugin}_sign_in gives the link."

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

  @doc """
  Calls a Google API as the account `email`, and gives what it answered, decoded from JSON. `options`:

  - `query`: a keyword list or map; a list value is given once for each, and `nil` or `""` not at all.
  - `json`: the body, as JSON; or `body` with its `type`. Neither may say `{{...}}`: the Vault would fill it in.
  - `headers`: more headers.
  - `missing`: what to say when what the URL names isn't there, or the account can't open it.
  - `hint`: what to add to what Google says about an input it can't take.
  """
  def request(call, app, email, method, url, options \\ []) do
    answer = fetch(call, app, email, method, url, options)
    if answer.body == "", do: nil, else: JSON.decode!(answer.body)
  end

  @doc "As `request/6`, and gives the whole answer: its `status`, `headers` and `body`."
  def fetch(call, app, email, method, url, options \\ []) do
    {body, type} =
      if json = options[:json],
        do: {unfilled(JSON.encode!(json)), "application/json"},
        else: {unfilled!(options[:body]), options[:type]}

    query = for {key, values} <- options[:query] || [], value <- List.wrap(values), value not in [nil, ""], do: {key, value}
    url = if query == [], do: url, else: url <> "?" <> URI.encode_query(query)
    headers = if type, do: %{"content-type" => type}, else: %{}

    headers = Map.merge(headers, Map.new(options[:headers] || %{}))
    request = %{url: url, method: method, headers: Map.put(headers, "authorization", "Bearer {{access_token}}"), body: body}
    answer = vault(call).fetch(call, item(email), request)

    if answer.status in 200..299 do
      answer
    else
      raise trouble(call, app, email, answer, Map.new(Keyword.take(options, [:missing, :hint])))
    end
  end

  # The Vault fills {{<field>}} in wherever it's written in a body, so what a person or an agent wrote, as an
  # email or a cell, must never say one. In JSON a { inside a string can be written as \u007b, and a { that
  # starts an object is never followed by another.
  defp unfilled(json), do: String.replace(json, ~r/\{(?=\{)/, fn _brace -> "\\u007b" end)

  defp unfilled!(nil), do: nil

  defp unfilled!(body) do
    if body =~ ~r/\{\{[A-Za-z0-9_]+\}\}/, do: raise("A body sent to Google can't say {{...}}: base64 it.")
    body
  end

  @doc "A part of a URL's path, as Google takes it: an id or a range, with anything but letters and digits escaped."
  def path(part), do: URI.encode(to_string(part), &URI.char_unreserved?/1)

  # What went wrong with a call, said for whoever's reading.
  defp trouble(call, app, email, answer, said \\ %{}) do
    message = message(answer.body) || "#{answer.status}"
    again = "#{email} needs signing in again, to let #{app.title} in: #{call.plugin}_sign_in with account #{email}."

    cond do
      answer.status == 401 ->
        again

      answer.status == 403 and message =~ ~r/insufficient|scope/i ->
        again

      # Gmail answers an id that was made up with 400 "Invalid id value", and Calendar a deleted event with 410.
      said[:missing] && (answer.status in [404, 410] or message =~ ~r/invalid id value/i) ->
        said.missing

      said[:missing] && answer.status == 403 ->
        "#{email} can't open it (#{String.trim_trailing(message, ".")}): use the account it's shared with (#{call.plugin}_accounts lists them), or ask its owner to share it."

      said[:hint] && answer.status == 400 ->
        "#{app.title}: #{String.trim_trailing(message, ".")}: #{said.hint}"

      true ->
        "#{app.title}: #{message}"
    end
  end

  defp message(body) do
    case JSON.decode(body) do
      {:ok, %{"error" => %{"message" => message}}} when is_binary(message) -> message
      {:ok, %{"error_description" => message}} when is_binary(message) -> message
      {:ok, %{"error" => message}} when is_binary(message) -> message
      _ -> nil
    end
  end

  # The addresses of the Google accounts signed in for the app whose call this is, by address.
  defp emails(call) do
    for %{name: "Google " <> email, sign_in: "Google"} <- vault(call).items(call), do: email
  end

  defp item(email), do: "Google #{email}"
  defp get(url), do: %{url: url, headers: %{"authorization" => "Bearer {{access_token}}"}}
  defp vault(call), do: OneCore.Call.plugin(call, "vault")
end
