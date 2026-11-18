# Lynx spike (step 6a)

Can protocol v1 ops (`core/ui/src/strict/protocol.ts`) drive Lynx's Element PAPI on Android, and beat today's WebView?

A throwaway Android app (`android/`), not the Tauri app, opens the same screen two ways:

- `LynxActivity`: Lynx 4.2. Its background thread (PrimJS, one runtime per LynxView) runs the app: Solid 2.0's universal
  renderer emitting v1 ops (`app/screen.js`). Its main thread applies them with Element PAPI (`app/lynx-main.js`). The
  bundle is encoded by Lynx's own encoder (`@lynx-js/tasm`), with no ReactLynx or rspeedy (`app/build.js`).
- `WebActivity`: an Android WebView (what Tauri uses on Android), running the same app and op stream against the DOM
  (`app/web-host.js`).

`./run` builds and installs on the running emulator. `./measure <Web|Lynx> <runs>` times open to fully drawn,
`./memory <Web|Lynx>` gives PSS, and `./a11y-tree <Web|Lynx> <file>` shows what an accessibility service sees (turn
TalkBack on first: both hosts build their trees only for a screen reader).

What Lynx needed:

- PrimJS has no ES2021 logical assignment (`??=`), which Solid 2.0 uses, so the bundle goes down to ES2019 (esbuild).
  It has no `queueMicrotask`, which Solid's scheduler uses (`app/lynx-globals.js`).
- A box that scrolls has to be a `<scroll-view>`, so the host makes an element on its first insert, once its style is
  known.
- Solid inserts an empty text as a placeholder anywhere, so a `raw-text` outside a `text` is drawn as an empty box.
- `enableAccessibilityElement: true` in the template, or TalkBack sees nothing inside the LynxView.
- `ThreadStrategyForRendering.MULTI_THREADS`: about a quarter faster to open than the default.
