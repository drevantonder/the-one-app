---
name: vault
description: Read whenever someone mentions or pastes a key, token or password, before asking for one or using one, and before a plugin signs in to an outside service or uses an account there.
metadata:
  subtitle: "Passwords and keys, kept safe"
  about: "Keeps your passwords, keys and sign-ins, and lets your apps and your AI use them without ever seeing them. You choose who may use each one."
---

The Vault is the one place for everything a person has given access to: keys, passwords, sign-ins, accounts and
Chrome profiles, each an item, used without being seen. An item has fields (a secret one is never read back, by
anyone), the sites its secrets are sent to, the plugins switched on for it, and whether agents may use it, and
can have contents too large for a field, kept sealed.

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
and never sees a secret, so an untrusted plugin can use one too. It names them in its `permissions/0`:

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
  with `use`, or which `keep` did. `values(call, item)` gives all its fields at once, and `rename(call, item, to)`
  renames it, its fields and contents with it.
- An item's contents are what's too large for a field, like a Chrome profile, kept as 1Password keeps a file: a
  blob of the Vault's, sealed under a key made for it alone, which only the item holds. `write_contents(call, item,
  name, send)` gives `send` a link that writes one blob and that key, for whatever holds the bytes to seal and send
  them a chunk at a time (Containers' `upload` does), and keeps them once it gives their size; what they replace goes
  then. `read_contents(call, item, name, receive)` gives `receive` a link that reads them and the key, for
  Containers' `download`, or gives `nil` where there are none yet. Neither ever passes the bytes through a tool's
  result, whatever their size. Removing the item removes its contents.

## Item types

A plugin that keeps items in its people's Vaults says what kind they are, as Android's AccountManager has apps
define account types and keeps the accounts itself. It hooks into `vault.types`, answering with its list of types:

    @impl true
    def hooks, do: ["vault.types"]

    @impl true
    def hook("vault.types", _about, _call),
      do: [%{name: "Chrome profile", plural: "Chrome Profiles", prefix: "Chrome profile: ", component: "profile", create: "create"}]

- `name` is one item of it, as a person reads it, and `prefix` what its items' names start with. An item switched on
  for the plugin whose name starts with it is of that type, and what follows is the item as a person reads it.
- `account: true` makes its items accounts: they're under Accounts in the Vault, by the plugin's own three tools.
  `accounts` lists them, the default first: `%{accounts: [%{account: "sam@example.com", ok: true}]}`, with `why`,
  a few words, where `ok` is false. `sign_in` signs one in, or with `account` signs that one in again, answering
  `%{url: ...}`, where the person signs in; one whose signing in needs a screen of its own (a QR code) gives it as
  `sign_in`'s `ui` (`client/sign-in.html`), which is opened instead, with the account after a `#`. `sign_out` takes
  `account`. Its other tools take `account`, the first `accounts` lists when it's left out.
- Any other type has its own section, headed `plural`. `component` is what draws one item beside the list: a Solid
  component the plugin's client exports (`client/package.json`: `{"exports": {"./profile": "./profile.tsx"}}`), the
  module's default export, given `item` (the item as a person reads it), `subject` (whose Vault it is), and `open(item)`
  and `close()`, for once it's renamed or gone. Its `Summary` export, given the same, is the item's line in the
  list. A trusted plugin's component is drawn in the Vault's page and acts with what the Vault may do. An untrusted
  plugin's item opens its page `client/<component>.html` in a frame of its own instead, with the item after a `#`,
  where it keeps what its own app may do. Without a component, an item opens with its fields, as a password does.
- `create` is the plugin's tool that makes one from a `name`, which the Vault's Add offers.

What the plugin keeps of an item, and the bytes in its contents, are the Vault's to keep and encrypt; what they mean
and do (a running browser, signed-in tools) are the plugin's.
