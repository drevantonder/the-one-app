---
name: {{server}}-build
description: Making or changing an app in {{name}} - its app.js, operations, screens, widgets and schedules, and deploying it with {{cli}} deploy, keeping its code in Repos. Use when the person wants {{name}} to do something no app does yet, or to change an app.
---

# Building an app

An app is one JavaScript module, `app.js` (or `app.ts`), with what it does for agents (operations), what people see (screens and widgets), and the data it keeps (entries, replayed into a SQLite store per page). Everyone in the install can read every app's code as deployed.

1. Look first: `{{cli}} search "<words>"` and the App Store may already have it. `{{cli}} source <app>` shows an app's code; fork one with `{{cli}} fork <app> <name>`.
2. Start from an app that's close: `{{cli}} source <app> app.js` prints its code as deployed. The shape of an app is in `references/apps.md`.
3. Write it. Operations are what agents call (`<app>_<op>`, `{{cli}} <app> <op>`), each with a description and a JSON Schema for its input: write them for an agent reading them cold. Mark what only reads `reads: true`, and what sends or can't be undone `confirm: true`. Give a long guide to agents as `instructions`: it becomes the app's skill.
4. Deploy: `{{cli}} deploy <dir>` builds it (npm packages and all) and deploys it, making the app if it's new, owned by you. Install it where it's wanted: `{{cli}} install <app>` (yours), `--scope <group>` or `--scope global` (admins).
5. Try it: `{{cli}} <app> <op> --help`, then run each operation. Its screens show in the app on the person's Mac and phone.
6. Something wrong? `{{cli}} deploys <app>` lists its versions, and `{{cli}} deploy <app> --version <hash>` puts an earlier one back.
7. Keep its code in a repo in Repos (`{{cli}} repos create`, then `git push {{cli}}://<repo> main`). On someone else's repo, push `main@<your name>`: a suggestion its owner takes.

Secrets an app needs are Vault items the person switches it on for: declare them as the app's `vault`, and write `{{field}}` where a value goes in a request; never put a key in code.

The exact contract: `references/apps.md` (an app's module, `ctx`, stores, connectors), `references/schedules.md`, `references/screens.md` and `references/widgets.md`.
