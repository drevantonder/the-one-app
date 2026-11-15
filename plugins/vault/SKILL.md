---
name: vault
description: Read whenever someone mentions or pastes a key, token or password, before asking for one or using one, and before a plugin signs in to an outside service or uses an account there.
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
  the status, headers and body. A secret goes only to one of the item's sites, and one the site says back is
  blotted out.
- `set` keeps one field of an item, making the item if there's none. Say `secret: false` for what may be
  shown, as a username, and `hosts` for the sites its secrets go to. The value is encrypted on the device you
  run on before it's sent, so `set` is for `one` or MCP on a device. An assistant that reaches the network
  over the web has no device: it asks with `request` instead.
- `use` switches a plugin on for an item, or off. A plugin that's switched on is handed the secret itself, so
  that's trusting its code with it. `agents` lets agents use an item with `fetch`: that's the person's to
  decide, in the Vault's app or by allowing a request, so ask with `request` and don't call it for yourself.
- `grant` chooses the servers that open the Vault, from a device that holds its person's key (the skill
  `encryption` says how one comes to). A Vault no server opens keeps no secret, so that comes first.
- Someone's Vault is theirs alone: nobody asks for another user's, and the network's owner reads none. A group's
  or the network's is its members' to use (`ONE_FOR=groups/family`), and its owner's to choose servers for.
- Removing an item, switching a plugin on and choosing servers are a person's decisions. Do them when asked
  to, and say what you did.

A plugin uses an account by using this plugin, and never keeps a secret itself:

    def uses, do: ["vault"]

    @github %{
      name: "GitHub",
      authorize: "https://github.com/login/oauth/authorize",
      token: "https://github.com/login/oauth/access_token",
      scopes: ["repo"],
      hosts: ["api.github.com"],
      client: "GitHub client"
    }

    def call("sign_in", _inputs, call), do: OneCore.Call.plugin(call, "vault").sign_in(call, @github)

    def call("me", _inputs, call) do
      vault = OneCore.Call.plugin(call, "vault")
      vault.fetch(call, "GitHub", %{url: "https://api.github.com/user", headers: %{"authorization" => "Bearer {{access_token}}"}}).body
    end

- `sign_in(call, service)` starts an OAuth sign-in and gives the link its person opens in their browser. The
  service sends them back to the network's address, and their tokens become the item named as the service is,
  or as a third argument says, for a second account, switched on for the plugin. All the plugin says is the
  service's own: where it signs in (`authorize`), where it gives tokens (`token`), its `scopes`, the sites its
  tokens are sent to (`hosts`), anything more its sign-in page is asked with (`params`), and `client`, the
  item that holds the `client_id` the service gave whoever registered the plugin with it, and its
  `client_secret` if it has one. A network without an address has nowhere to be sent back to.
- `fetch(call, item, request)` makes a web request with the item's fields filled in where `{{<field>}}` is
  written, and gives `%{status: ..., headers: ..., body: ...}`. A secret goes only to one of the item's sites.
  A signed-in item's `access_token` is renewed when it has ended, or when the service says it has.
- `value(call, item, field)` gives one field itself, to hand to a program a service brings. Hand it over on the
  program's input, and never answer with it, raise it or log it.
- `keep(call, item, fields)` keeps secrets the plugin made itself or an outside service handed it, such as the
  key a session it holds is encrypted with, in an item it makes, switched on for it. `remove(call, item)` removes
  one, as when that account is signed out.
- `fetch` and `value` raise unless the item is switched on for the plugin, which its person does in the Vault's
  app or with `use`, or which `keep` did.
