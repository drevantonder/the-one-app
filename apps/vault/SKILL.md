---
name: vault
description: Read whenever someone mentions or pastes a key, token or password, before asking for one or using one, and before an app signs in to an outside service or uses an account there.
metadata:
  title: "Vault"
  subtitle: "Passwords and keys, kept safe"
  about: "Keeps your passwords, keys and sign-ins, and lets your apps and your AI use them without ever seeing them. You choose who may use each one."
  category: "Privacy & Security"
---

The Vault is the one place for everything a person has given access to: keys, passwords, sign-ins, accounts and
Chrome profiles, each an item, used without being seen. An item has fields (a secret one is never read back, by
anyone), the sites its secrets are sent to, the apps switched on for it, and whether agents may use it, and
can have contents too large for a field, kept sealed.

Never ask for a password, key or token in the chat, and never paste one. `request` gives the person a link to
their own device, where they pick or type it; `fetch` makes a web request with an item they let agents use, and
you never see the secret. If someone pastes one anyway, don't repeat it: in code mode, or over the web, ask for it
with `request`; only as a plain tool on a device (`one mcp`, `one vault set`) keep it with `set`.

- `list` shows what's there: each item's fields, a secret one without its value, its sites, the apps switched
  on for it, whether agents may use it, its state and its `artifact`, the requests still waiting, the apps that
  use the Vault, and the kinds of item they keep.
- `request` asks for an item by its `title` and the `fields` it needs, for an `app` or, with none, for you.
  Give the person the link it answers with. Today the link's page tells them to open Vault on a device of
  theirs, where the request waits at the top; `status` says when they've answered. Allowing it switches the
  item on for whoever asked.
- `fetch` fills `{{<field>}}` into a request's url, headers or body, for an item agents may use, and answers
  the status, headers and body. It goes only to one of the item's sites, and a secret the site says back is
  blotted out.
- `set` keeps one field of an item, making the item if there's none. Say `secret: false` for what may be
  shown, as a username, and `hosts` for the sites its secrets go to. The value is encrypted on the device you
  run on before it's sent, so `set` is for `one` or MCP on a device, as a plain tool. A script or `call` in code
  mode runs on a server, where nothing encrypts it, so a secret given there is refused: in code mode, and over the
  web, ask with `request` instead.
- `use` switches an app on for an item, or off. An app that's switched on uses it through the Vault's tools
  without seeing it, or, one with a service, is handed the secret itself, which trusts its code with it. `agents` lets agents use an item with `fetch`: that's the person's to
  decide, in the Vault's app or by allowing a request, so ask with `request` and don't call it for yourself.
- `grant` chooses the servers that open the Vault, from a device that holds its person's key (the skill
  `encryption` says how one comes to). A Vault no server opens keeps no secret, so that comes first.
- Someone's Vault is theirs alone: nobody asks for another user's, and the network's owner reads none. A group's
  or the network's is its members' to use (`ONE_SUBJECT=groups/family`), and its owner's to choose servers for.
- Deleting an item is soft delete's, as for anything an app made: `soft-delete delete --artifact` with its
  `artifact`, as `list` gives it, and it can be restored for 30 days. `remove` takes one field out of an item.
- Deleting an item, switching an app on and choosing servers are a person's decisions. Do them when asked to,
  and say what you did.

An app uses an account through this app's tools, which act for the app that calls them (its `caller`),
and never sees a secret, so an untrusted app can use one too. It requires this app:

    def requires, do: %{"vault" => "to sign you in to GitHub"}

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
  says, for a second account, switched on for the app. All the app says is the service's own: where it
  signs in (`authorize`), where it gives tokens (`token`), its `scopes`, the sites its tokens are sent to
  (`hosts`), anything more its sign-in page is asked with (`params`), and `client`, the item that holds the
  `client_id` the service gave whoever registered the app with it, and its `client_secret` if it has one,
  which goes only to that item's sites: its person keeps the service's token address among them. A network
  without an address has nowhere to be sent back to. An item's name that says `{{<claim>}}` is filled in from
  who signed in, as the service's OpenID Connect ID token says: `"Google {{email}}"`, with `openid` and `email`
  among the scopes, is an item for each account.
- `fetch`, called by an app, makes a web request with an item switched on for it, to one of the item's sites,
  with `{{<field>}}` filled in, and answers the whole of what the site said. It waits 15 seconds for an answer,
  or `timeout`. A signed-in item's `access_token` is renewed when it has ended, or when the service says it has.
- `items` lists the items switched on for the app, each with the service whose sign-in made it:
  `[{"name": "Google a@example.com", "sign_in": "Google", "artifact": "...", "state": null}]`. `state` says which
  of three states one is in (Item types, below), and `stop_using` switches one off for it, leaving it to whatever
  else uses it.
- The other tools are its person's, and refuse an app.
- An app that does something for other apps, as `google-account` does for the Google apps, calls these on
  behalf of whichever called it: `OneCore.Call.tool(call, "vault", "fetch", request, on_behalf_of: call.caller)`.

An app trusted with a secret itself, whose service hands it to a program, requires this app (`def requires, do:
%{"vault" => "to sign you in to GitHub"}`) and calls its functions, which run inside it on a server of its owner's:

- `value(call, item, field)` gives one field itself, to hand to a program a service brings. Hand it over on the
  program's input, and never answer with it, raise it or log it.
- `keep(call, item, fields)` keeps secrets the app made itself or an outside service handed it, such as the
  key a session it holds is encrypted with, in an item it makes, switched on for it. `remove(call, item)` removes
  one, as when that account is signed out. `items(call)` lists what's switched on for it.
- `value` raises unless the item is switched on for the app, which its person does in the Vault's app or
  with `use`, or which `keep` did. `values(call, item)` gives all its fields at once, and `rename(call, item, to)`
  renames it, its fields and contents with it.
- An item's contents are what's too large for a field, like a Chrome profile, kept as 1Password keeps a file: a
  blob of the Vault's, sealed under a key made for it alone, which only the item holds. `write_contents(call, item,
  name, send)` gives `send` a link that writes one blob and that key, for whatever holds the bytes to seal and send
  them a chunk at a time (Containers' `upload` does), and keeps them once it gives their size; what they replace goes
  then. `read_contents(call, item, name, receive)` gives `receive` a link that reads them and the key, for
  Containers' `download`, or gives `nil` where there are none yet. Neither ever passes the bytes through a tool's
  result, whatever their size. They're the item's, so deleting it deletes them, and restoring it brings them back.
- `state(call, item, state, message, action)` says which of the three states an item is in; `remove(call, item)`
  deletes one through soft delete.

## Item types

An app that keeps items in its people's Vaults says what kind they are, as Android's AccountManager has apps
define account types and keeps the accounts itself. An account (a Gmail address, a WhatsApp number) is one such item,
of a type its app registers, as a password or a Chrome profile is. It hooks into `vault.types`, answering with its
list of types:

    @impl true
    def hooks, do: ["vault.types"]

    @impl true
    def hook("vault.types", _about, _call),
      do: [%{name: "Chrome profile", section: "Chrome Profiles", prefix: "Chrome profile: ", component: "profile", create: %{tool: "create", name: true}}]

- `name` is one item of it, as a person reads it, and `prefix` what its items' names start with. An item switched on
  for the app whose name starts with it is of that type, and what follows is the item as a person reads it.
- `section` is the heading its items are listed under: its own (Chrome Profiles), or one several apps' types share
  (Accounts).
- `create` is how one is added, an action as below; `name: true` asks its person for a name first, which the tool is
  given as `name`.
- `component` is what draws one item beside the list: a Solid component the app's client exports
  (`client/package.json`: `{"exports": {"./profile": "./profile.tsx"}}`), the module's default export, given `item`
  (the item as a person reads it), `subject` (whose Vault it is), and `open(item)` and `close()`, for once it's
  renamed or gone. Its `Summary` export, given the same, is the item's line in the list when it's ready. A trusted
  app's component is drawn in the Vault's page and acts with what the Vault may do. An untrusted app's item
  opens its page `client/<component>.html` in a frame of its own instead, with the item after a `#`, where it keeps
  what its own app may do. Without a component, an item opens with its fields, as a password does.

An item is in one of three states, which the Vault shows and the app says, with `state` (or `state/5`). The app
keeps its own detailed states (WhatsApp's linking, Chrome Profiles' update) and says only which of these it's in:

- `ready`: usable, which shows nothing: fine is silent. Every item starts there, and a password is always there.
- `busy`, with a `message` ("Saving…"): something's under way, shown as a spinner and it.
- `action_required`, with a `message`, why ("Signed out on your phone"), and one `action`: its person has to act,
  shown as a dot, why and a button. An action is a tool of the app's, called as its person, whose `url`, if it
  gives one, opens in their browser (`%{label: "Sign In", tool: "sign_in", inputs: %{account: "+614..."}}`), or a
  page of its client's (`%{label: "Sign In", page: "client/sign-in.html#+614..."}`).

Two things are each app's to choose, and none chooses for another: whether its person is notified when its item
needs them (`notifications`), and what it does when its item is deleted, restored or purged, which soft delete's hook
tells it (signing out and revoking at once, or keeping its token so a restore just works).

What the app keeps of an item, and the bytes in its contents, are the Vault's to keep and encrypt, and deleting,
restoring and purging an item are soft delete's; what an item means and does (a running browser, signed-in tools) are
the app's.
