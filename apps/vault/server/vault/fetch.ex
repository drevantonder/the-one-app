defmodule Vault.Fetch do
  @moduledoc """
  A web request with an item's fields filled in, which is how a secret leaves the Vault: only to one of its item's
  websites, and what the site says back has any secret blotted out. Nothing here raises what it sent or was sent:
  only which site it was, and what the site named as wrong.
  """

  @doc """
  Makes a web request with `{{<field>}}` filled in from `fields` wherever it's written: in the URL (percent-encoded,
  so a value is only ever text within it), a header or the body (escaped, in a JSON one, as a JSON string's text),
  only to one of `hosts`, so what's asked with an item goes nowhere else: where it goes is checked once it's filled
  in, as it's sent. `secret` names the fields that are secret, which are blotted out of what the site answers, which
  it's asked to send uncompressed so they can be. Gives `%{status: ..., headers: ..., body: ...}`. A redirect is
  answered as it is, not followed: it may lead to another site.
  """
  def request(request, fields, secret, hosts) do
    used =
      Enum.uniq(
        Enum.flat_map(
          [request.url, request[:body] || "" | Enum.flat_map(request[:headers] || %{}, &Tuple.to_list/1)],
          &named/1
        )
      )

    for field <- used, not is_map_key(fields, field), do: raise("It has no field named #{field}.")

    fill = fn text, as -> Regex.replace(~r/\{\{([A-Za-z0-9_]+)\}\}/, text, fn _all, field -> as.(fields[field]) end) end
    plain = &fill.(&1, fn value -> value end)
    url = fill.(request.url, &URI.encode(&1, fn char -> URI.char_unreserved?(char) end))
    site = site!(URI.parse(url))
    if URI.parse(url).userinfo, do: raise("A request names nobody before its site, as user@ would.")
    if not to?(url, hosts), do: raise("It's used only with #{listed(hosts)}, and that's #{site}.")

    # Asked for as it is, never compressed: what it says back is looked through for secrets.
    headers =
      for {name, value} <- request[:headers] || %{}, String.downcase(name) != "accept-encoding",
          do: {plain.(name), plain.(value)}

    headers = [{"accept-encoding", "identity"} | headers]
    method = (request[:method] || "get") |> to_string() |> String.downcase() |> method!()

    # In a JSON body a value is written inside a string, so it's escaped as a JSON string's text is.
    json? = Enum.any?(headers, fn {name, value} -> String.downcase(name) == "content-type" and value =~ ~r/json/i end)
    body = request[:body] && if(json?, do: fill.(request.body, &String.slice(JSON.encode!(&1), 1..-2//1)), else: plain.(request.body))
    # A body that isn't text is sent as it is.
    body = request[:bytes] || body

    case Req.request(
           method: method,
           url: url,
           headers: headers,
           body: body,
           redirect: false,
           # Req's own: a GET or a HEAD that met a moment's trouble is sent again; anything else, once.
           retry: :safe_transient,
           max_retries: 2,
           decode_body: false,
           receive_timeout: (request[:timeout] || 15) * 1000
         ) do
      {:ok, %{headers: %{"content-encoding" => [encoding | _]}}} when encoding != "identity" ->
        raise "#{site} answered #{encoding}-compressed, asked not to, so what it said can't be looked through for secrets."

      {:ok, response} ->
        blotted = secret |> Enum.flat_map(&forms(fields[&1])) |> Enum.uniq() |> Enum.sort_by(&(-byte_size(&1)))
        blot = fn text -> Enum.reduce(blotted, text, &String.replace(&2, &1, "[secret]")) end
        headers = Map.new(response.headers, fn {name, values} -> {name, blot.(Enum.join(values, ", "))} end)
        %{status: response.status, headers: headers, body: blot.(response.body)}

      {:error, _unreached} ->
        raise "#{site} couldn't be reached."
    end
  end

  @doc """
  Whether a request to `url` goes to one of `hosts`: its site's name, its host with the port if it isn't its scheme's
  own, and no user before it (`user@`), which would name someone at a site rather than where it goes.
  """
  def to?(url, hosts) do
    uri = URI.parse(url)
    uri.userinfo == nil and site!(uri) in hosts
  end

  # A secret as a site may say it back: as it is, escaped in a JSON string, URL-encoded, or in base64, padded or
  # not, standard or URL-safe.
  defp forms(value) when is_binary(value) and value != "" do
    json = String.slice(JSON.encode!(value), 1..-2//1)
    base64 = [Base.encode64(value), Base.encode64(value, padding: false), Base.url_encode64(value), Base.url_encode64(value, padding: false)]
    [value, json, URI.encode_www_form(value), URI.encode(value, &URI.char_unreserved?/1) | base64]
  end

  defp forms(_value), do: []

  # The fields a text names.
  defp named(text), do: for([_all, field] <- Regex.scan(~r/\{\{([A-Za-z0-9_]+)\}\}/, text), do: field)

  defp listed([]), do: "no site: it names none"
  defp listed(hosts), do: Enum.join(hosts, ", ")

  defp method!(method) when method in ~w(get post put patch delete head), do: String.to_existing_atom(method)
  defp method!(_method), do: raise("A request's method is GET, POST, PUT, PATCH, DELETE or HEAD.")

  @doc false
  # A site as it's named: its host, with its port if that isn't its scheme's own. What's sent travels encrypted,
  # or stays on this machine, where a service being built or tested answers (RFC 8252's loopback).
  def site!(%URI{scheme: scheme, host: host, port: port}) when is_binary(host) do
    cond do
      scheme == "https" -> if port == 443, do: host, else: "#{host}:#{port}"
      scheme == "http" and host in ["127.0.0.1", "localhost", "::1"] -> "#{host}:#{port}"
      true -> raise "A request goes to an https address."
    end
  end

  def site!(_uri), do: raise("That isn't a web address.")
end
