# Android: our own renderer (spike)

Can a strict screen be drawn on Android by our own renderer, from the strict protocol's ops, and beat today's WebView
(and the Lynx spike's numbers, the target), with text and typing as solid as a native field?

A throwaway Android app (`android/`) opens the same two screens two ways:

- `StrictActivity`: the screen's JS (Solid 2.0's universal renderer sending protocol v1 ops, `app/screen.js` and
  `app/form.js`) runs in QuickJS through rquickjs, with the desktop host's sandbox (`clients/app/strict/src/js.rs`);
  the ops build the desktop host's tree (`tree.rs`), laid out with taffy, set with cosmic-text in Allo (Android's own
  fonts as fallback), and drawn as GPUI's scene by GPUI's Blade renderer on Vulkan (`rust/`). `StrictView` (a
  SurfaceView) bridges the keyboard with Flutter's `InputConnectionAdaptor` and `ListenableEditingState`, and
  Flutter's `TextInputPlugin`'s editor configuration and autofill wiring (`android/.../editing/`), in Flutter's
  editing-state shape (text, selection, composing range). AccessKit's Android adapter gives the screen reader the
  tree's semantics.
- `WebActivity`: an Android WebView (what Tauri uses) running the same JS against the DOM (`app/web-host.js`).

`./run` builds the JS, the renderer and the APKs and installs them on the running emulator (`./run arm64-v8a` builds
for a phone). `./measure <Web|Strict> <runs>`, `./memory <Web|Strict>`, `./tap-paint <Web|Strict> <taps>` and
`./a11y-tree <Web|Strict> <file>` measure as the Lynx spike's scripts do. `android/autofill` is a test autofill
service (`adb shell settings put secure autofill_service app.theone.testautofill/.TestAutofillService`).

Where the code is from (each file says so, with its upstream commit):

- GPUI (Zed, Apache-2.0) at 8249ef56, the last commit with its Blade renderer: `scene.rs` (the primitives),
  `renderer.rs`, `atlas.rs`, `shaders.wgsl` (without subpixel sprites, which need dual-source blending).
- Blade (MIT) at 3d86d537 with one patch (`rust/blade-android-rgba.patch`): an RGBA swapchain where a surface has no
  BGRA, as Android's don't. Blade at GPUI's own commit refuses GPUs without `VK_EXT_inline_uniform_block` (the
  emulator's, and many phones'); 3d86d537 falls back to uniform buffers.
- Flutter's Android embedder (BSD-3-Clause) at 4d19bda0: `ListenableEditingState`, `InputConnectionAdaptor`,
  `TextInputPlugin` (adapted), `TextEditState` (written after it).
- Rune (MIT) at 78efd4a7: the secure field's masking (`edit.rs`).
- The desktop host (`clients/app/strict`): `js.rs`, `tree.rs` and `tokens.rs`, by path, unchanged; `rust/gpui` is
  GPUI's `Rgba` alone, which `tokens.rs` names.

What it needed, that a build of it for real would too:

- The emulator's Vulkan driver crashes naming objects (`vk_common_SetDebugUtilsObjectNameEXT`): no names are given.
- cosmic-text 0.19, ellipsizing, starts a wrapped line with the space it wrapped at (drawn around it here).
- Android 16's emoji font is COLRv1, which swash can't draw: emoji are blank.
- The shared `tree.rs` names `keydown`'s prop `onKeydown`; JSX's is `onKeyDown` (worked around here).
- The emulator's `hw.keyboard=yes` makes Gboard type as from a physical keyboard: no composing or autocorrect in any
  app. Keyboard checks need an AVD with `hw.keyboard=no`.
