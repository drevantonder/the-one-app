---
name: chrome
description: Read before opening, reading or acting on a web page in a browser of your own, to open a page, read it, click, fill in a form or run JavaScript.
metadata:
  subtitle: "A browser for your AI"
  about: "Gives your AI a Chrome of its own on your machine, signed in to nothing, to open pages, click and fill in forms. You can watch it as it goes."
  category: Agent tools
---

Chrome is a Google Chrome of your own, in a container on the user's machine, signed in to nothing, on a screen of its
own that its person can watch. Use it for anything that's a web page. For anything that isn't (a desktop app, a phone), or a site that
blocks automated browsers, use Computer Use instead. To only fetch a page's HTML or an API, an HTTP request is
cheaper still.

- `open` starts a browser under a `session` name of your choosing, with what it's for (`description`) and a `url`
  to go to. Every other tool takes that `session`. Opening one that's open already carries on with it.
- Act from `snapshot`: the page as its accessibility tree, in YAML, where each element has a ref like `e12`. Pass
  the ref as `target` to `click`, `type`, `fill_form`, `select_option`, `hover` and `evaluate`. Refs are of the
  snapshot they came from: after the page changes, take a new one. An action answers with the page's URL and
  title; its `[Snapshot](.playwright-mcp/...)` line is a file inside the browser's container, so call `snapshot`
  to see what changed.
- `wait_for` waits for text to appear or go; `press_key`, `navigate_back`, `tabs` and `handle_dialog` do what they
  say. `evaluate` runs JavaScript in the page and gives what it returns: the cheapest way to pull out data.
- `take_screenshot` is for what only a picture shows. A snapshot costs far less: a few KB for a simple page, though
  a long article's can be 200 KB, so give `snapshot` a `target` or `depth`, or `evaluate` for the text you need.
- `close` closes it and removes its container. `list` shows yours.
- A running browser is a screen in Computer Use, called `chrome-<session>`: its person watches it there, and
  Computer Use's tools act on it by pixels, for what only a picture shows, the browser's own address bar and tabs
  too. Its screen is 1280×800 until a person's view resizes it to fit, and a screenshot says its size.

The first browser on a machine builds its image (about 1.5 GB, a few minutes); after that `open` takes a few
seconds. Each
browser holds up to 2 CPUs and 4 GB of the user's machine while it runs, so close it when you're done. One left
unused stops after Containers' `idle_hours`, and `open` with its session starts it again, afresh. A
browser is whoever opened it's: nobody else sees or uses it but the network's owner.
