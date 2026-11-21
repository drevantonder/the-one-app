defmodule Vault.SignIns do
  @moduledoc """
  A sign-in to an outside service for an app, by OAuth 2 (RFC 6749) with PKCE (RFC 7636), whose tokens become an item
  of its person's, and their renewal.

  A sign-in moves once (`@moves`):

      started --tokens kept------------------> ok
      started --they said no at the service--> cancelled
      started --anything else----------------> failed

  It's `started` as its link is given, and kept by its `state` with where it began (`began/1`): an app's screen, an
  agent, or `one` in a terminal. Its service sends its person's browser back, which moves it, written once from
  `started` (`move!/4`), so its page shows the same ending however often it's opened, until it's let go with the rest
  that waited too long. Try Again on one that didn't sign in starts another as it began.

  The protocol is ours, not a library's: what it needs (a link, a code swapped for tokens, a renewal, an ID token's
  claims) is four small requests over Req, which One Core already has, where a library would be a new dependency of
  every server for one app, and none keeps tokens as an item does. An ID token's claims aren't checked against its
  signature, as it came straight from the service's token address over TLS (OpenID Connect Core 3.1.3.7).
  """

  alias OneCore.{Call, User}
  alias Vault.{Access, Items, Store}

  @moves %{"started" => ~w(ok cancelled failed)}

  # How long a sign-in waits for its browser, in seconds.
  @waits 600

  # What a sign-in's page says in a browser its link wasn't opened in.
  @elsewhere "Finish signing in in the browser you began in, or start it again."

  @doc "The states a sign-in is in."
  def states, do: ["started" | @moves["started"]]

  @doc "What a sign-in is to start with, from what an app gave, or what was kept of that."
  def service(given) do
    %{
      name: given["name"],
      authorize: given["authorize"],
      token: given["token"],
      scopes: given["scopes"],
      hosts: Items.hosts!(given["hosts"]),
      client: given["client"],
      params: given["params"] || %{}
    }
  end

  @doc "Starts a sign-in for `app`, `started`, and gives the link its person opens."
  def start(call, db, service, app, item, began, apps) do
    address = address()

    Store.quietly("Starting the sign-in", fn ->
      {state, verifier} = {"#{Base.url_encode64(call.subject, padding: false)}.#{Store.random(24)}", Store.random(48)}
      # An app signs in with a client its person switched on for it, and nothing else of theirs.
      found = Access.may!(Items.find!(call, db, service.client), {:app, app})
      client = Items.fields(found)

      client["client_id"] ||
        raise "#{service.client} has no client_id. Keep the one #{service.name} gave you there."

      # The code and its verifier go to the token address, which redeems them for tokens, as its client_secret does:
      # only to one of the client's sites, which its person chose.
      if not Vault.Fetch.to?(service.token, found["hosts"]),
        do: raise("#{service.client}'s sign-ins go only to its sites, and #{service.name}'s token address isn't one. Its person adds it, in the Vault.")

      own!(service, found["hosts"])
      service = %{service | authorize: authorize!(call, db, service, found, client, app)}
      if not String.contains?(item, "{{"), do: ours!(Items.find(call, db, item), app, service.name)

      Store.transaction(call, db, fn d ->
        Store.query(d, "delete from sign_ins where at < ?", [Store.now() - @waits])

        Store.query(
          d,
          "insert into sign_ins (state, verifier, service, app, item, at, began, apps) values (?, ?, ?, ?, ?, ?, ?, ?)",
          [state, verifier, JSON.encode!(service), app, item, Store.now(), JSON.encode!(began), JSON.encode!(apps)]
        )
      end)

      # Its person opens it at the network's edge, which keeps the browser that did (`opened/4`).
      address <> "/vault/start?" <> URI.encode_query(%{"state" => state})
    end)
  end

  @doc """
  Where a sign-in began, which says how its page ends: on an agent, named as its person let it in (an assistant
  somewhere else, which comes in over the web, among their devices as `sign-in` lists them) or as it says it's
  called (an MCP client on a device of theirs, through `one mcp`); in a terminal, by `one`; or on an app's screen,
  the Vault's.
  """
  def began(call) do
    client = Call.client(call) || %{}
    said = client["title"] || client["name"]
    sign_in = Call.app(call, "sign-in")
    device = if client["web"], do: sign_in.device(call)

    cond do
      device["kind"] == "assistant" -> %{agent: device["name"]}
      client["name"] == "one-cli" -> %{cli: true}
      sign_in.screen?(call) -> %{screen: "settings#vault/accounts"}
      said -> %{agent: said}
      true -> %{screen: "settings#vault/accounts"}
    end
  end

  @doc """
  The service sent its person back: what its page shows. Whoever came back is nobody the network knows: what says
  whose sign-in this finishes is its `state`, which only the browser that began it was given, and which starts with
  whose it is. A server that passed this on has checked that, and asks for whoever's it is, by nobody.
  """
  def callback(%Call{actor: actor} = call, %{"state" => state} = inputs) do
    with [whose, _random] <- String.split(state, "."),
         {:ok, owner} <- Base.url_decode64(whose, padding: false),
         true <- User.owner?(owner),
         true <- actor == User.link("#{owner}/#{state}") or (actor == nil and call.subject == owner) do
      call = %{call | subject: owner, actor: nil}
      db = Store.own(call)

      sign_in =
        case Store.transaction(call, db, &Store.query(&1, "select * from sign_ins where state = ? and at > ?", [state, Store.now() - @waits])) do
          [sign_in] -> sign_in
          [] -> raise "This sign-in isn't waiting any more. Start it again."
        end

      cond do
        inputs["open"] -> opened(call, db, sign_in, inputs["open"])
        not browser?(sign_in, inputs["browser"]) -> raise(@elsewhere)
        inputs["again"] -> again(call, db, sign_in)
        sign_in["ending"] -> ending(sign_in, JSON.decode!(sign_in["ending"]))
        true -> returned(call, db, sign_in, inputs)
      end
    else
      _ -> raise "This sign-in isn't one this network began."
    end
  end

  # It ends as the service said, written once.
  defp returned(call, db, sign_in, inputs) do
    service = JSON.decode!(sign_in["service"])

    ending =
      try do
        Store.quietly("Finishing the sign-in", fn ->
          cond do
            # RFC 6749 4.1.2.1: they said no.
            inputs["error"] == "access_denied" -> %{"status" => "cancelled"}
            why = inputs["error"] -> raise "#{service["name"]} didn't sign you in: #{why}."
            true -> signed!(call, db, sign_in, service, inputs["code"])
          end
        end)
      rescue
        error in RuntimeError -> %{"status" => "failed", "problem" => error.message}
      end

    move!(call, db, sign_in, ending)
  end

  # Swaps the code for tokens, which the service gives once, and keeps them in the item, for the app that began it.
  defp signed!(call, db, sign_in, service, code) do
    code || raise "#{service["name"]} sent no code back."
    client = Items.fields(Items.find!(call, db, service["client"]))
    tokens = exchange(service, client, code, sign_in["verifier"], address() <> "/vault/callback")
    name = signed_in(sign_in["item"], service, tokens)
    app = sign_in["app"]
    ours!(Items.find(call, db, name), app, service["name"])

    # Signed in, it's ready: whatever it needed of its person is done, and the app they signed in for has access, as
    # their signing in says.
    item =
      Items.write!(call, db, name, Items.kept_by(call, app, name, service["hosts"]), :person, fn record ->
        record |> kept_tokens(service, tokens) |> Map.put("status", nil) |> Map.update!("access", &Enum.uniq(&1 ++ [app]))
      end)

    Access.in_step(call, db, app, true)
    Call.changed(call)
    account = claims(tokens["id_token"])["email"]
    Map.merge(%{"status" => "ok", "item" => item["name"]}, if(is_binary(account), do: %{"account" => account}, else: %{}))
  end

  # Moves a sign-in from started to how it ended (`@moves`), once: a second return finds the first's ending, which
  # is what it answers, never one it didn't write.
  defp move!(call, db, sign_in, %{"status" => to} = ending) do
    if to not in @moves["started"], do: raise("A sign-in doesn't go from started to #{to}.")

    kept =
      Store.transaction(call, db, fn d ->
        case Store.query(d, "update sign_ins set ended = ?, ending = ? where state = ? and ended is null returning ending", [to, JSON.encode!(ending), sign_in["state"]]) do
          [%{"ending" => written}] -> written
          [] -> Store.query(d, "select ending from sign_ins where state = ?", [sign_in["state"]]) |> List.first() |> Map.fetch!("ending")
        end
      end)

    ending(sign_in, JSON.decode!(kept))
  end

  # What its page shows: how it ended, and where it began.
  defp ending(sign_in, ending) do
    %{
      service: JSON.decode!(sign_in["service"])["name"],
      began: JSON.decode!(sign_in["began"]),
      apps: JSON.decode!(sign_in["apps"])
    }
    |> Map.merge(Map.new(ending, fn {key, value} -> {String.to_existing_atom(key), value} end))
  end

  # Try Again: one that ended without signing in starts another, as it began, for the same app and item.
  defp again(call, db, %{"ended" => ended} = sign_in) when ended in ["cancelled", "failed"] do
    service = service(JSON.decode!(sign_in["service"]))
    began = JSON.decode!(sign_in["began"])
    apps = JSON.decode!(sign_in["apps"])
    url = start(call, db, service, sign_in["app"], sign_in["item"], began, apps)
    %{service: service.name, status: "started", began: began, apps: apps, url: url}
  end

  defp again(_call, _db, _sign_in), do: raise("Only a sign-in that didn't sign you in starts again.")

  # A sign-in's tokens go only to the service's own sites, where the app that asks says they go: its token address's
  # site, which its person chose (`start/7`), or a site under the same registrable domain (`Vault.PublicSuffix`), as
  # a browser decides whose cookie a site may set, or one of its client's sites. A token address with none, an IP
  # address or a public suffix itself, has no other.
  defp own!(service, client_hosts) do
    token = URI.parse(service.token)
    domain = Vault.PublicSuffix.registrable(token.host)

    for site <- service.hosts,
        site not in client_hosts,
        site != Vault.Fetch.site!(token),
        host = hd(String.split(site, ":")),
        domain == nil or Vault.PublicSuffix.registrable(host) != domain do
      raise "#{service.name}'s tokens go only to its own sites, or #{service.client}'s, and #{site} isn't one."
    end
  end

  # Where its person signs in to the service: the client's own sign-in page, which the client item keeps. A client
  # kept before items kept it takes the first page an app it was switched on for names, and keeps it from then on, so
  # no other app sends its person anywhere else with it. It's a web page over https that names nobody before its
  # site: never a script (`javascript:`), a file or a page sent in the clear.
  defp authorize!(call, db, service, found, client, app) do
    authorize = client["authorize"] || service.authorize || ""
    uri = URI.parse(authorize)

    if not (uri.scheme == "https" and is_binary(uri.host) and uri.userinfo == nil),
      do: raise("#{service.client}'s sign-in page is an https address.")

    if client["authorize"] == nil,
      do: Items.write!(call, db, found["name"], nil, {:app, app}, &Items.put_field(&1, "authorize", authorize, false))

    authorize
  end

  # Its link opened, in the browser its person's edge says `browser` (a random value it keeps in a cookie): the first
  # to open it is the only one it finishes in, as OAuth 2.1 binds a request's state to its user agent (RFC 9700
  # 4.7.1). Gives where the service's page is, asked for a code that only its verifier redeems.
  defp opened(call, db, sign_in, browser) when is_binary(browser) and byte_size(browser) >= 32 do
    opened =
      Store.transaction(call, db, fn d ->
        Store.query(d, "update sign_ins set browser = ? where state = ? and browser is null and ended is null returning state", [hashed(browser), sign_in["state"]])
      end)

    if opened == [], do: raise("This sign-in was opened already, in another browser. Start it again.")
    service = service(JSON.decode!(sign_in["service"]))
    client = Items.fields(Items.find!(call, db, service.client))
    url = link(service, client["client_id"], address() <> "/vault/callback", sign_in["state"], sign_in["verifier"])
    %{service: service.name, status: "started", began: JSON.decode!(sign_in["began"]), apps: JSON.decode!(sign_in["apps"]), url: url}
  end

  defp opened(_call, _db, _sign_in, _browser), do: raise(@elsewhere)

  defp browser?(%{"browser" => kept}, browser) when is_binary(kept) and is_binary(browser),
    do: :crypto.hash_equals(kept, hashed(browser))

  defp browser?(_sign_in, _browser), do: false

  defp hashed(browser), do: Base.encode16(:crypto.hash(:sha256, browser), case: :lower)

  # A sign-in writes only into an item a sign-in to the same service made, which the app keeps or uses, as signing in
  # again does, never into another of its person's.
  defp ours!(nil, _app, _service), do: :ok

  defp ours!(found, app, service) do
    if not (found["oauth"]["name"] == service and app in [found["app"] | found["access"]]),
      do: raise("#{found["name"]} is in the Vault already, and no sign-in to #{service} for this app made it.")
  end

  # The item a sign-in's tokens become: its name, with any {{<claim>}} filled in from who the service says signed in.
  defp signed_in(name, service, tokens) do
    claims = claims(tokens["id_token"])

    Regex.replace(~r/\{\{([A-Za-z0-9_]+)\}\}/, name, fn _all, claim ->
      case claims[claim] do
        value when is_binary(value) and value != "" -> value
        _ -> raise "#{service["name"]} didn't say who signed in. Sign in again."
      end
    end)
  end

  @doc """
  An item's fields with its tokens renewed, unless another call renewed them meanwhile. One renewal at a time, in a
  transaction on its person's Vault's database, which one server at a time runs, so two calls on two servers don't
  both spend a refresh token a service gives once.
  """
  def renewed(call, db, item, had) do
    client = Items.fields(Items.find!(call, db, item["oauth"]["client"]))

    Store.transaction(call, db, fn _d ->
      latest = Store.record(db, item["id"]) || raise "There's no item named #{item["name"]} in the Vault."

      if Items.fields(latest)["access_token"] != had["access_token"] do
        Items.fields(latest)
      else
        refresh = Items.fields(latest)["refresh_token"] || raise "#{item["name"]} has no way to renew its sign-in. Sign in again."
        tokens = renew(latest["oauth"], client, refresh)
        Items.fields(Store.change!(db, latest, &kept_tokens(&1, latest["oauth"], tokens)))
      end
    end)
  end

  defp kept_tokens(record, service, tokens) do
    record =
      Enum.reduce(~w(access_token refresh_token), record, fn field, record ->
        if value = tokens[field], do: Items.put_field(record, field, value, true), else: record
      end)

    oauth = %{
      "name" => service["name"],
      "token" => service["token"],
      "client" => service["client"],
      "expires" => Store.now() + (tokens["expires_in"] || 3600)
    }

    Map.put(record, "oauth", oauth)
  end

  # The link a sign-in begins at: the service's own page, asked for a code that only `verifier` redeems.
  defp link(service, client_id, back, state, verifier) do
    challenge = Base.url_encode64(:crypto.hash(:sha256, verifier), padding: false)

    query =
      Map.merge(service.params, %{
        "response_type" => "code",
        "client_id" => client_id,
        "redirect_uri" => back,
        "scope" => Enum.join(service.scopes, " "),
        "state" => state,
        "code_challenge" => challenge,
        "code_challenge_method" => "S256"
      })

    service.authorize <> "?" <> URI.encode_query(query)
  end

  defp exchange(service, client, code, verifier, back),
    do: tokens(service, client, grant_type: "authorization_code", code: code, redirect_uri: back, code_verifier: verifier)

  defp renew(service, client, refresh_token), do: tokens(service, client, grant_type: "refresh_token", refresh_token: refresh_token)

  # A code or a refresh token is spent as it's sent, so a token request isn't sent again on its own.
  defp tokens(service, client, params) do
    url = service["token"]
    Vault.Fetch.site!(URI.parse(url))
    secret = if secret = client["client_secret"], do: [client_secret: secret], else: []
    form = params ++ [client_id: client["client_id"]] ++ secret

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

  # Who an OpenID Connect ID token says signed in: its claims, or none without one.
  defp claims(nil), do: %{}

  defp claims(id_token) do
    with [_head, payload, _signature] <- String.split(id_token, "."),
         {:ok, json} <- Base.url_decode64(payload, padding: false),
         {:ok, claims} when is_map(claims) <- JSON.decode(json) do
      claims
    else
      _ -> raise "The ID token isn't one."
    end
  end

  defp address do
    Application.get_env(:one_core, :address) ||
      raise "A sign-in to an outside service finishes at the network's address, and this network has none."
  end
end
