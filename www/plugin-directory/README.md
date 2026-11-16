# Plugin directory

Every plugin anyone has installed, ranked by installs, at <https://one.drevan.me/plugins>. Nobody submits a plugin:
it lives in any GitHub repo at `plugins/<name>/`, with a `SKILL.md` whose front matter has its `name` and
`description`, and it's listed the first time an installer says it installed it. `seed.js` lists the plugins in this
repo that a network may choose to install, so the list isn't empty before anyone has.

## The installer's call

After installing a plugin, unless `DO_NOT_TRACK` is set (`1`, or any value but empty and `0`), an installer makes one call:

    POST https://one.drevan.me/plugins/api/installs
    Content-Type: application/json

    {"source": "drevantonder/the-one-app", "plugin": "gmail", "commit": "c6ebcc7"}

`commit` is optional (7 to 40 hex characters); without it the directory reads the default branch. Answers:

- `204` counted.
- `400` the body isn't that shape; `404` `plugins/<plugin>/SKILL.md` with a description isn't in that repo at that
  commit; `429` more than 10 a minute from one address. Each has `{"error": "<what to do>"}`.

An installer doesn't wait on it or retry it, and nothing it installs depends on it.

## Reading the list

    GET https://one.drevan.me/plugins/api/plugins[?period=week]

    {"period": "all", "plugins": [{"source": "drevantonder/the-one-app", "name": "gmail", "description": "…",
      "installs": 12, "code": "https://github.com/…/tree/HEAD/plugins/gmail",
      "install": "one install drevantonder/the-one-app --plugin gmail"}]}

Most installs first; `week` counts the last seven days (UTC). Any origin may read it.

## What's kept

Per plugin, its source, name, description and when its `SKILL.md` was last read; per plugin per day, a count.
Nothing about who installed it: no address, network, cookie or identifier. The rate limit is Cloudflare's rate
limiting binding, keyed by the caller's address, which Cloudflare counts per location and we never store.
