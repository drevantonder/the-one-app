---
name: vault
description: Read whenever someone mentions or pastes a key, token or password, before asking for one or using one, and before a plugin signs in to an outside service or uses an account there.
metadata:
  subtitle: "Passwords and keys, kept safe"
  about: "Keeps your passwords, keys and sign-ins, and lets your apps and your AI use them without ever seeing them. You choose who may use each one."
---

The Vault keeps keys, sign-ins and accounts as items, used without being seen. An item has fields (a secret one is
never read back, by anyone), the sites its secrets are sent to, the plugins switched on for it, and whether agents
may use it.

Never ask for a password, key or token in the chat, and never paste one. `request` gives the person a link to
their own device, where they pick or type it; `fetch` makes a web request with an item they let agents use, and
you never see the secret. If someone pastes one anyway, keep it with `set` and don't repeat it.

- `list` shows what's there: each item's fields, a secret one without its value, its sites, the plugins switched
  on for it, whether agents may use it, the requests still waiting, and the plugins that use the Vault.
- `request` asks for an item by its `title` and the `fields` it needs, for a `plugin` or, with none, for you.
  Give the person the link it answers with. Today the link's page tells them to open Vault on a device of
  theirs, where the request waits at the top; `status` says when they've answered. Allowing it switches the
  item on for whoever asked.
- `fetch` fills `{{<field>}}` into a request's url, headers or body, for an item agents may use, and answers
  the status, headers and body. It goes only to one of the item's sites, and a secret the site says back is
  blotted out.
- `set` keeps one field of an item, making the item if there's none. Say `secret: false` for what may be
  shown, as a username, and `hosts` for the sites its secrets go to. The value is encrypted on the device you
  run on before it's sent, so `set` is for `one` or MCP on a device. An assistant that reaches the network
  over the web has no device: it asks with `request` instead.
- `use` switches a plugin on for an item, or off. A plugin that's switched on uses it through the Vault's tools
  without seeing it, or, one with a service, is handed the secret itself, which trusts its code with it. `agents` lets agents use an item with `fetch`: that's the person's to
  decide, in the Vault's app or by allowing a request, so ask with `request` and don't call it for yourself.
- `grant` chooses the servers that open the Vault, from a device that holds its person's key (the skill
  `encryption` says how one comes to). A Vault no server opens keeps no secret, so that comes first.
- Someone's Vault is theirs alone: nobody asks for another user's, and the network's owner reads none. A group's
  or the network's is its members' to use (`ONE_SUBJECT=groups/family`), and its owner's to choose servers for.
- Removing an item, switching a plugin on and choosing servers are a person's decisions. Do them when asked
  to, and say what you did.

A plugin uses an account through this plugin's tools, which act for the plugin that calls them (its `caller`),
and never sees a secret, so a plugin in sandbox mode can use one too. It names them in its `permissions/0`:

    def permissions, do: ~w(vault_fetch vault_items vault_sign_in vault_stop_using)

    @github %{
      "name" => "GitHub",
      "authorize" => "https://github.com/login/oauth/authorize",
      "token" => "https://github.com/login/oauth/access_token",
      "scopes" => ["repo"],
      "hosts" => ["api.github.com"],
      "client" => "GitHub client"
    }

    def call("sign_in", _inputs, call), do: OneCore.Call.tool(call, "vault", "sign_in", %{"service" => @github})

    def call("me", _inputs, call) do
      request = %{"item" => "GitHub", "url" => "https://api.github.com/user", "headers" => %{"authorization" => "Bearer {{access_token}}"}}
      OneCore.Call.tool(call, "vault", "fetch", request)["body"]
    end

- `sign_in` starts an OAuth sign-in and gives the link its person opens in their browser. The service sends
  them back to the network's address, and their tokens become the item named as the service is, or as `item`
  says, for a second account, switched on for the plugin. All the plugin says is the service's own: where it
  signs in (`authorize`), where it gives tokens (`token`), its `scopes`, the sites its tokens are sent to
  (`hosts`), anything more its sign-in page is asked with (`params`), and `client`, the item that holds the
  `client_id` the service gave whoever registered the plugin with it, and its `client_secret` if it has one,
  which goes only to that item's sites: its person keeps the service's token address among them. A network
  without an address has nowhere to be sent back to. An item's name that says `{{<claim>}}` is filled in from
  who signed in, as the service's OpenID Connect ID token says: `"Google {{email}}"`, with `openid` and `email`
  among the scopes, is an item for each account.
- `fetch`, called by a plugin, makes a web request with an item switched on for it, to one of the item's sites,
  with `{{<field>}}` filled in, and answers the whole of what the site said. It waits 15 seconds for an answer,
  or `timeout`. A signed-in item's `access_token` is renewed when it has ended, or when the service says it has.
- `items` lists the items switched on for the plugin, each with the service whose sign-in made it:
  `[{"name": "Google a@example.com", "sign_in": "Google"}]`. `stop_using` switches one off for it, as signing
  out does, and leaves it to whatever else uses it.
- The other tools are its person's, and refuse a plugin.
- A plugin that does something for other plugins, as `google-account` does for the Google plugins, calls these on
  behalf of whichever called it: `OneCore.Call.tool(call, "vault", "fetch", request, on_behalf_of: call.caller)`.

A plugin trusted with a secret itself, whose service hands it to a program, uses this plugin (`def uses, do:
["vault"]`) and calls its functions, which run on a server of its owner's:

- `value(call, item, field)` gives one field itself, to hand to a program a service brings. Hand it over on the
  program's input, and never answer with it, raise it or log it.
- `keep(call, item, fields)` keeps secrets the plugin made itself or an outside service handed it, such as the
  key a session it holds is encrypted with, in an item it makes, switched on for it. `remove(call, item)` removes
  one, as when that account is signed out. `items(call)` lists what's switched on for it.
- `value` raises unless the item is switched on for the plugin, which its person does in the Vault's app or
  with `use`, or which `keep` did.
