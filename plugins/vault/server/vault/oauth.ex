defmodule Vault.OAuth do
  @moduledoc """
  What the Vault asks of the outside: a request with an item's fields filled in, and the three requests of an
  OAuth sign-in (RFC 6749, with PKCE, RFC 7636): the link its person opens, the exchange of the code they come
  back with for tokens, and a renewal. It's the same for every service, so a plugin that signs in to one names
  only the service's own addresses and scopes.

  Nothing here raises what it sent or was sent: only which site it was, and what the site named as wrong.
  """

  @doc "The link a sign-in begins at: the service's own page, asked for a code that only `verifier` redeems."
  def link(service, client_id, back, state, verifier) do
    challenge = Base.url_encode64(:crypto.hash(:sha256, verifier), padding: false)

    asked =
      Map.merge(Map.get(service, :params, %{}), %{
        "response_type" => "code",
        "client_id" => client_id,
        "redirect_uri" => back,
        "scope" => Enum.join(service.scopes, " "),
        "state" => state,
        "code_challenge" => challenge,
        "code_challenge_method" => "S256"
      })

    service.authorize <> "?" <> URI.encode_query(asked)
  end

  @doc "The tokens a service gives for the code its person came back with."
  def exchange(service, client, code, verifier, back) do
    tokens(service, client, grant_type: "authorization_code", code: code, redirect_uri: back, code_verifier: verifier)
  end

  @doc "The tokens a service gives for a refresh token."
  def renew(service, client, refresh_token) do
    tokens(service, client, grant_type: "refresh_token", refresh_token: refresh_token)
  end

  defp tokens(service, client, asked) do
    url = service["token"]
    site!(URI.parse(url))
    secret = if secret = client["client_secret"], do: [client_secret: secret], else: []
    form = asked ++ [client_id: client["client_id"]] ++ secret

    case Req.post(url, form: form, retry: false, redirect: false) do
      {:ok, %{status: 200, body: %{"access_token" => token} = tokens}} when is_binary(token) ->
        tokens

      {:ok, %{status: status, body: body}} ->
        why =
          if is_map(body) and is_binary(body["error"]) and body["error"] =~ ~r/^[a-z_]{1,40}$/,
            do: ": " <> body["error"],
            else: ""

        raise "#{service["name"]} gave no tokens (#{status}#{why}). Sign in again."

      {:error, _unreached} ->
        raise "#{service["name"]} couldn't be reached for tokens."
    end
  end

  @doc """
  Who an OpenID Connect ID token says signed in: its claims, or none without one. It came straight from the
  service's token address over TLS, which vouches for it in place of its signature (OpenID Connect Core 3.1.3.7).
  """
  def claims(nil), do: %{}

  def claims(id_token) do
    with [_head, payload, _signature] <- String.split(id_token, "."),
         {:ok, json} <- Base.url_decode64(payload, padding: false),
         {:ok, claims} when is_map(claims) <- JSON.decode(json) do
      claims
    else
      _ -> raise "The ID token isn't one."
    end
  end

  @doc """
  Makes a web request with `{{<field>}}` filled in from `fields` wherever it's written: in the URL, a header or
  the body, only to one of `hosts`, so what's asked with an item goes nowhere else. `secret` names the fields that
  are secret, which are blotted out of what the site answers. Gives `%{status: ..., headers: ..., body: ...}`. A
  redirect is answered as it is, not followed: it may lead to another site.
  """
  def request(request, fields, secret, hosts) do
    uri = URI.parse(request.url)
    site = site!(uri)

    used =
      Enum.uniq(
        Enum.flat_map(
          [request.url, request[:body] || "" | Enum.flat_map(request[:headers] || %{}, &Tuple.to_list/1)],
          &named/1
        )
      )

    for field <- used, not is_map_key(fields, field), do: raise("It has no field named #{field}.")

    if not to?(request.url, hosts), do: raise("It's used only with #{said(hosts)}, and that's #{site}.")

    fill = &Regex.replace(~r/\{\{([A-Za-z0-9_]+)\}\}/, &1, fn _all, field -> fields[field] end)
    headers = for {name, value} <- request[:headers] || %{}, do: {fill.(name), fill.(value)}
    method = (request[:method] || "get") |> to_string() |> String.downcase() |> method!()
    body = request[:body] && fill.(request.body)

    case Req.request(
           method: method,
           url: fill.(request.url),
           headers: headers,
           body: body,
           redirect: false,
           retry: false,
           decode_body: false,
           receive_timeout: (request[:timeout] || 15) * 1000
         ) do
      {:ok, answer} ->
        blot = fn text -> Enum.reduce(secret, text, &String.replace(&2, fields[&1] || "\0", "[secret]")) end
        headers = Map.new(answer.headers, fn {name, values} -> {name, blot.(Enum.join(values, ", "))} end)
        %{status: answer.status, headers: headers, body: blot.(answer.body)}

      {:error, _unreached} ->
        raise "#{site} couldn't be reached."
    end
  end

  @doc "Whether a request to `url` goes to one of `hosts`: its site's name, or its host's."
  def to?(url, hosts) do
    uri = URI.parse(url)
    site!(uri) in hosts or uri.host in hosts
  end

  # The fields a text names.
  defp named(text), do: for([_all, field] <- Regex.scan(~r/\{\{([A-Za-z0-9_]+)\}\}/, text), do: field)

  defp said([]), do: "no site: it names none"
  defp said(hosts), do: Enum.join(hosts, ", ")

  defp method!(method) when method in ~w(get post put patch delete head), do: String.to_existing_atom(method)
  defp method!(_method), do: raise("A request's method is GET, POST, PUT, PATCH, DELETE or HEAD.")

  # A site as it's named: its host, with its port if that isn't its scheme's own. What's sent travels encrypted,
  # or stays on this machine, where a service being built or tested answers (RFC 8252's loopback).
  defp site!(%URI{scheme: scheme, host: host, port: port}) when is_binary(host) do
    cond do
      scheme == "https" -> if port == 443, do: host, else: "#{host}:#{port}"
      scheme == "http" and host in ["127.0.0.1", "localhost", "::1"] -> "#{host}:#{port}"
      true -> raise "A request goes to an https address."
    end
  end

  defp site!(_uri), do: raise("That isn't a web address.")
end
