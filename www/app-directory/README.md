# App directory

Every app anyone has installed, ranked by installs, at <https://one.ptcau.dev/apps>. Nobody submits an app:
it lives in any GitHub repo at `apps/<name>/`, with a `SKILL.md` whose front matter has its `name` and
`description`, and what a person reads of it in its `metadata`'s `subtitle`, `about` and `category`, each on one line,
and it's listed the first time an installer says it installed it. Each is listed as a network's marketplace lists it:
what its SKILL.md says, and what its code says (`listing`), as the network that installed it gave it. `seed.js` lists
a repo's apps as a network's Marketplace lists them, so the list isn't empty before anyone has installed them:

    one apps marketplace | bun seed.js ptc-au/the-one-app ../../apps > seed.sql

## The installer's call

After installing an app, unless `DO_NOT_TRACK` is set (`1`, or any value but empty and `0`), an installer makes one call:

    POST https://one.ptcau.dev/apps/api/installs
    Content-Type: application/json

    {"source": "ptc-au/the-one-app", "app": "gmail", "commit": "c6ebcc7", "listing": {"title": "Gmail",
      "icon": "data:image/png;base64,…", "entrypoints": [{"name": "Gmail", "icon": "data:…", "category": null}],
      "widgets": [{"name": "Inbox", "sizes": ["small"]}], "requires": ["google-account"]}}

`commit` is optional (7 to 40 hex characters); without it the directory reads the default branch. `listing` is what
`deploy` answered of it, and is optional too; nothing checks it against the code. Answers:

- `204` counted.
- `400` the body isn't that shape; `404` `apps/<app>/SKILL.md` with a description isn't in that repo at that
  commit; `429` more than 10 a minute from one address. Each has `{"error": "<what to do>"}`.

An installer doesn't wait on it or retry it, and nothing it installs depends on it.

## Reading the list

    GET https://one.ptcau.dev/apps/api/apps[?period=week]

    {"period": "all", "apps": [{"source": "ptc-au/the-one-app", "name": "gmail", "title": "Gmail",
      "icon": "data:…", "subtitle": "…", "about": "…", "category": "Connectors", "entrypoints": […], "widgets": […],
      "requires": […], "description": "…", "installs": 12, "code": "https://github.com/…/tree/HEAD/apps/gmail",
      "install": "one install ptc-au/the-one-app --app gmail"}]}

Most installs first; `week` counts the last seven days (UTC). Any origin may read it.

## What's kept

Per app, its source, name, description, subtitle, about, category, listing and when its `SKILL.md` was last read; per app per day, a count.
Nothing about who installed it: no address, network, cookie or identifier. The rate limit is Cloudflare's rate
limiting binding, keyed by the caller's address, which Cloudflare counts per location and we never store.
