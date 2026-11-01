---
name: {{server}}-build
description: Making or changing an app in {{name}} - writing its app.js, deploying it with {{cli}} deploy, and putting it on a page. Use when the person wants {{name}} to do something no app does yet, or to change an app.
---

# Building an app

An app is a folder with one JavaScript module, `app.js` (or `app.ts`): its operations, which agents call; its screens and widgets, which people see; and the data it keeps. Everyone in the install can read every app's code, so start from one that's close to yours.

1. Look first: `{{cli}} search "<words>"` may find an app that already does it. To change someone else's, fork it: `{{cli}} fork <app> <name>`.
2. Read the apps already built: `{{cli}} apps code <app>` lists an app's files and `{{cli}} apps code <app> --path app.js` gives one. The App Store shows the same under View code, and in a checkout of {{name}}'s code the built-in apps are the folders in `apps/`.
3. Make a folder and write `app.js`. Describe each operation and its input for an agent reading them cold. Mark what only reads `reads: true`, and what sends or can't be undone `confirm: true`. A key or password is a Vault item the person switches the app on for: never put one in code.
4. Deploy: `{{cli}} deploy <folder>` builds it, npm packages and all, and makes the app if it's new, owned by you. To try it out first where no one else sees it, put `try` before each command (`{{cli}} try deploy <folder>`): a new install of its own on this machine, which `{{cli}} try stop` deletes.
5. Put it on a page: `{{cli}} install <app>` for your own, `--scope <group>` for a group's, `--scope global` (admins) for everyone's.
6. Check it: `{{cli}} <app> <op> --help`, then run each operation, and open its screens and widgets in the app on the person's Mac or phone. If something's wrong, `{{cli}} deploys <app>` lists its versions and `{{cli}} deploy <app> --version <hash>` puts one back.
7. Keep its code wherever you like (Repos, `git push {{cli}}://<repo> main`, or GitHub or any git host), and link the app to it: `{{cli}} apps source <app> <link>`. Its page shows it as Source.

## The details

What a call's `ctx` gives an app, what a screen's bridge and design kit offer, and which widget nodes the shells draw are all in {{name}}'s own code, which is open source: {{code}}. Read it there or in a checkout. It's the only reference, so it's never out of date.
