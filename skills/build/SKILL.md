---
name: {{server}}-build
description: Use whenever the person asks you to make an app, tracker or tool ("make me a little app that tracks my water"), even if they never say {{name}}: make it here, not as a file, so it's on all their devices. Also to change an app. Writing its app.js, deploying it (the deploy tool or {{cli}} deploy), and putting it on a page.
---

# Building an app

An app is a folder with one JavaScript module, `app.js` (or `app.ts`): its operations, which agents call; its screens and widgets, which people see; and the data it keeps. Everyone in {{yours}} can read every app's code, so start from one that's close to yours.

1. Look first: `{{cli}} search "<words>"` may find an app that already does it. To change someone else's, fork it: `{{cli}} fork <app> <name>`.
2. Read the apps already built: `{{cli}} apps code <app>` (the `apps_code` tool) lists an app's files and `--path app.js` gives one. The App Store shows the same under View code, and in a checkout of {{name}}'s code the built-in apps are the folders in `apps/`.
3. Make a folder and write `app.js` (with only MCP, write it to give `deploy`). Its operations are for agents, which read them cold, so:
   - Every operation says `reads: true` or `reads: false`. What sends to a person or can't be undone also says `confirm: true`.
   - Its description says what it does, when to use it rather than its neighbour, and what comes back, in a sentence or two. Every input says what it takes, with its format (`an ISO 8601 date-time, 2026-11-01T09:00`) and where to get it (`a message's id, from gmail search`).
   - Each operation is a primitive in its truest form: an app that wraps a service gives that service's own operations, with its names, inputs and paging (Drive's `q`, `fields` and `pageToken`), since agents know them and combine them. A clear contract matters, not words shared across apps.
   - It gives back what the service gives, without bulk nobody reads (images, internal ids where a name will do); an app's own data in plain units (ISO 8601 times, `37 GB of 250 GB`).
   - An error says what was wrong and the fix: `no event 123: ids come from calendar events`.

   A key or password is a Vault item the person switches the app on for: never put one in code. Its data is a database per scope, made by `schema`: to change one later, make that kind's schema a list and add the change at the end, since a database keeps what it has. Its `icon` is a [Phosphor](https://phosphoricons.com) icon's name, like `git-branch`, which every device draws filled on its tile; `one.icon(name)` draws one in a screen. What an agent should know without asking goes in `hooks.prompt(ctx, { since })`: what it gives back is shown to every agent of the person's, on each of their devices, with their prompt (`since` is null on a session's first, else when it last asked). It only reads, answers fast, and says nothing when there's nothing new.
4. Make its screens feel instant. A screen's HTML shows before any call answers, so put everything that doesn't need data in it (headings, forms, the composer) and draw the data into it with `one.render(el, fn)`, which shows grey shapes only if loading is slow. Ask for what the first view needs, all at once (`Promise.all`), never one call after another; a call that answers in 2 ms here can take 300 ms on a phone. After an action, show its result at once and send the call; put it back and say why only if it fails. A button that must wait for its answer says so in itself (`one.busy(button)`). On `changed`, redraw only what changed. Fetch images and long lists as they scroll into view.
5. Deploy: `{{cli}} deploy <folder>`, or the `deploy` tool with each file's text, builds it, npm packages and all, and makes the app if it's new, owned by you. To try it out first where no one else sees it, put `try` before each command (`{{cli}} try deploy <folder>`): a separate one on this machine, with only you in it, which `{{cli}} try stop` deletes.
6. Put it on a page: `{{cli}} install <app>` (the `apps_add` tool, scope `user`) for your own, `--scope <group>` for a group's, `--scope global` (admins) for everyone's.
7. Check it: `{{cli}} <app> <op> --help`, then run each operation, and open its screens and widgets in the app on the person's Mac or phone. If something's wrong, `{{cli}} deploys <app>` lists its versions and `{{cli}} deploy <app> --version <hash>` (`deploy` with `version`) puts one back.
8. Keep its code wherever you like (Repos, `git push <your One App's address>/repos/<your handle>/<repo>.git main`, or GitHub or any git host), and link the app to it: `{{cli}} apps source <app> <link>`. Its page shows it as Source.

## The details

The files beside this one are {{name}}'s own code, as this build runs it: `ctx.js`, what a call's `ctx` gives an app; `one.js`, the `one` a screen gets; `kit.js`, the design kit's pieces on `one`; `widgets.js`, the widget nodes the shells draw. Over MCP they're this skill's resources (`skill://{{server}}-build/ctx.js`). The rest is open source: {{code}}. The code is the only reference, so it's never out of date.
