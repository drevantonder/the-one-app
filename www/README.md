# www

the One App's own website, at <https://one.drevan.me>: three Workers on Cloudflare, each deployable on its own.

- `gateway/` answers one.drevan.me (a custom domain) and sends `/apps` and everything under it to the app
  directory, and the rest to the marketing site, by service binding.
- `marketing/` is one static page.
- `app-directory/` is the app directory: its pages, its API and its counts in D1 (its own README).

Run it all on this machine, from this folder:

    bun install
    bunx wrangler d1 execute one-www-app-directory --local --file app-directory/schema.sql
    (cd app-directory && one apps marketplace | bun seed.js ptc-au/the-one-app ../../apps > seed.sql && bunx wrangler d1 execute one-www-app-directory --local --file seed.sql)
    bunx wrangler dev -c gateway/wrangler.jsonc -c app-directory/wrangler.jsonc -c marketing/wrangler.jsonc

The Workers are TypeScript, which wrangler builds without checking. `bun run typecheck` checks them;
`bun run types` makes each Worker's `worker-configuration.d.ts` again after its `wrangler.jsonc` changes.

Deploy, with `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` set, each from its own folder, the gateway last:

    cd app-directory && bunx wrangler deploy
    cd marketing && bunx wrangler deploy
    cd gateway && bunx wrangler deploy

The app directory's database is made once (`bunx wrangler d1 create one-www-app-directory`, its id in
`app-directory/wrangler.jsonc`), and its schema and seed run with `--remote` in place of `--local`.

The social card, `marketing/public/social-card.png`, is `marketing/social-card.html` rendered once at 1200×630, with
the fonts in `marketing/fonts/` (Geist Serif, from npm `geist`, SIL Open Font License, `fonts/OFL.txt`):
`chrome --headless --window-size=1200,630 --hide-scrollbars --force-device-scale-factor=1 --virtual-time-budget=3000 --screenshot=public/social-card.png file://$PWD/social-card.html`.
