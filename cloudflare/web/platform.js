// Where the web client runs, chosen once as the page starts: a browser (browser.js), or the app's own window
// (app.js, on client/native's `window.__oneNative`). Both are the same object, so the rest of the page never asks which:
//
// - `start()`, before anything else; `client()`, what a call says it comes from; `phone()`, whether it's a phone's
//   home of pages side by side; `behind()`, whether this client is too old for the network; `label()`, what device this is as users say it; `mac()`, whether it's a Mac; `panel`, whether this page is the app's menu bar panel.
// - Signing in: `signedIn()`, `user()`, `until()` (until when the keys a request hands may be used), `signIn()`,
//   `signOut()` (its tokens gone), and `welcome(props)`, the screen that asks for it.
// - Being its user's device: `join(waiting)` (let in, calling `waiting(code, invited)` while it waits), `reaches(user)`,
//   `fetch(…)` (net.js's), `removed()` (taken out, as the relay or the app says), `leave()` (signing out as a
//   device), `nodes(state)`, `settle()`, `kept(…)`, `renew(app)`, `forget()`, `device()` (what Settings › Devices says
//   of it), `address()` (where its Approve link goes).
// - What the app's window does beyond a page: `changed()`, `badge(n)`, `appearance(v)`, `copy(text)`, `open(href)`,
//   `notify(ask)`, `bridge(frame)` (what screens' bridges may ask of it), `look()` (what their
//   frames are told it does), `fit(height)` and `main(route)` (the panel's).
import { app } from "./app.js";
import { browser } from "./browser.js";

export const platform = window.__oneNative ? app(window.__oneNative) : browser();
