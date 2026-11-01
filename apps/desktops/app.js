// Desktops: computers for agents and people, each a live picture with a browser in it, on the person's servers; and
// the browser profiles they start with. The desktops driver runs them (Incus or a Mac's virtual machines), on each
// server whose owner installed it; this app is its page and what agents use.

/// Runs one of the desktops driver's operations, wherever it runs for the caller. When none of their servers runs it,
/// says what's needed instead of how the call failed.
async function driver(ctx, op, input) {
  try {
    return await ctx.drivers.desktops[op](input);
  } catch (err) {
    if (String(err.message).includes("no server runs desktops for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Desktops needs the Desktops driver on one of your servers, and none of them runs it. A server's owner installs it there: Drivers, in the App Store, shows how.";

/// The caller's online servers that run the driver, by name.
async function running(ctx) {
  return (await ctx.servers()).filter((s) => s.online && (s.drivers || []).includes("desktops")).map((s) => s.name);
}

export default {
  name: "desktops",
  title: "Desktops",
  icon: "macwindow.on.rectangle",
  color: "#a2845e",
  description: "Desktops with a browser, for your agents and you",
  instructions: "Desktops are Linux desktops on the user's servers, each with a browser. To browse, borrow a desktop of your own: desktops_borrow with for (what it's for, in a few words: the person sees it) gives you one whose browser starts with a copy of a browser profile, signed in to its sites (default unless you pass profile; desktops_profiles lists them, like Personal or PTC); pass its name as desktop to the other tools, and give it back with desktops_give_back when done: it's deleted, with anything it did to the profile. One left 15 minutes without a call is given back for you. Borrow rather than use a lasting desktop (desktops_create makes those, for the person), so agents never share a browser. In it: desktops_browser_prepare with allow_launch true and profile {\"mode\":\"isolated_named\",\"name\":\"default\"} (the profile's name) launches the browser; desktops_use with tool list_windows gives its window_id; desktops_get_browser_state with that pid and window_id gives target_id and tab_id; then desktops_browser_navigate, desktops_browser_click and desktops_browser_type act, and desktops_get_browser_state reads the page. A browser left idle for 5 minutes closes; prepare it again. desktops_screen shows the desktop. When a site needs the person to sign in, call desktops_sign_in with the site's url and give them its link: they sign in there, in the profile itself (pass profile), and it's saved when they're done (or desktops_save_profile). For a challenge in your own desktop, give them desktops_view's link. Never sign in or submit forms for them unless they asked. Everything else on a desktop (apps, windows, keyboard, mouse): desktops_tools lists the tools and desktops_use runs one. desktops_list shows every server's desktops, who has borrowed which and what for; to act on one on another server, pass server too. A lasting desktop nobody calls or watches for 30 minutes is paused, and unpaused by the next call.",
  uses: { drivers: ["desktops"] },

  ops: {
    list: {
      description: "Every server's desktops: each one's server, whether it's running, paused or stopped, its size, the memory and CPU it uses now, and who has borrowed it and what for (or that it's the spare kept for borrowing).",
      input: {"properties": {"here": {"description": "only this server's", "type": "boolean"}}, "type": "object"},
      run: (ctx, input) => driver(ctx, "list", input),
    },
    borrow: {
      description: "Borrow a desktop of your own for as long as you need a browser, starting its browser with a copy of a browser profile. Quick: a spare is kept ready. Give it back with give_back when done; it's deleted then, or after 15 minutes without a call, with anything in it.",
      input: {"properties": {"by": {"description": "who's borrowing, e.g. your app's name; default the device you call from", "type": "string"}, "for": {"description": "what you need it for, in a few words, e.g. Reading Messenger; the person sees it", "type": "string"}, "profile": {"description": "the browser profile its browser starts with; default default if saved, \"\" for none", "type": "string"}}, "required": ["for"], "type": "object"},
      run: (ctx, input) => driver(ctx, "borrow", input),
    },
    give_back: {
      description: "Give back a borrowed desktop: it's deleted with everything in it, its browser's changes to the profile too. A sign_in desktop's profile is saved first.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => driver(ctx, "give_back", input),
    },
    sign_in: {
      description: "For the person: opens a page in a browser on a browser profile (made if new) and gives a link to use it from their own devices, to sign in to sites agents then borrow desktops with. It's saved when its time is up, or on give_back or save_profile.",
      input: {"properties": {"minutes": {"description": "how long until it's saved and closed; default 30, at most 240", "type": "integer"}, "profile": {"description": "a browser profile's name, like Personal or PTC; default default", "type": "string"}, "url": {"description": "the page to open, e.g. a site's sign-in page", "type": "string"}}, "type": "object"},
      run: (ctx, input) => driver(ctx, "sign_in", input),
    },
    save_profile: {
      description: "Save a desktop's browser profile, closing its browsers first; a sign_in desktop is also given back. Only when the person asked: borrowed desktops start from saved profiles.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}, "profile": {"description": "the profile; default the desktop's, or default", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => driver(ctx, "save_profile", input),
    },
    profiles: {
      description: "The browser profiles desktops can be borrowed with, like Personal and PTC, each with its own sign-ins: which server keeps it, and when it was last saved.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => driver(ctx, "profiles", input),
    },
    forget_profile: {
      description: "Delete a browser profile, signing agents out of every site in it. Only when the person asked.",
      input: {"properties": {"profile": {"description": "the profile's name", "type": "string"}}, "required": ["profile"], "type": "object"},
      run: (ctx, input) => driver(ctx, "forget_profile", input),
    },
    create: {
      description: "Create a lasting desktop and start it, for the person: Linux, with a browser of its own. Takes a minute. Agents borrow instead.",
      input: {"properties": {"desktop": {"description": "its name: lowercase letters, digits and dashes", "type": "string"}, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => driver(ctx, "create", input),
    },
    resize: {
      description: "Change how much CPU and memory a desktop may use, at once, without restarting it.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "required": ["desktop", "size"], "type": "object"},
      run: (ctx, input) => driver(ctx, "resize", input),
    },
    start: {
      description: "Start a stopped (or paused) desktop. It starts again after the server restarts.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => driver(ctx, "start", input),
    },
    stop: {
      description: "Stop a desktop. Its files and sign-ins stay; it stays stopped until started.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => driver(ctx, "stop", input),
    },
    delete: {
      description: "Delete a desktop and everything in it, its sign-ins too. Only when the person asked.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => driver(ctx, "delete", input),
    },
    view: {
      description: "A link to watch and use a desktop in a browser, for the person: to solve a challenge, or to see what an agent is doing. Works on their own devices.",
      input: {"properties": {"desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "minutes": {"description": "how long the link works; default 60, at most 1440", "type": "integer"}}, "type": "object"},
      run: (ctx, input) => driver(ctx, "view", input),
    },
    screen: {
      description: "A small picture of a desktop as it is now.",
      input: {"properties": {"desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "width": {"description": "the picture's width in pixels; default 640, at most 1920", "type": "integer"}}, "type": "object"},
      run: (ctx, input) => driver(ctx, "screen", input),
    },
    tools: {
      description: "The desktop's other tools, for desktops_use; with name, that tool's full description and input.",
      input: {"properties": {"desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "name": {"description": "a tool's name", "type": "string"}}, "type": "object"},
      run: (ctx, input) => driver(ctx, "tools", input),
    },
    refresh_tools: {
      description: "Read the desktops' tools again from a running desktop, as after its image was updated.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => driver(ctx, "refresh_tools", input),
    },
    use: {
      description: "Run one of the desktop's tools, as desktops_tools lists them, e.g. list_windows, launch_app, click, type_text, get_guide.",
      input: {"properties": {"args": {"description": "the tool's input", "type": "object"}, "desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "tool": {"description": "the tool's name", "type": "string"}}, "required": ["tool"], "type": "object"},
      run: (ctx, input) => driver(ctx, "use", input),
    },
    backup: {
      description: "Back up a desktop's home folder (its files and sign-ins, without caches) as a blob any server can fetch, keeping the newest 5. Not off-site: backups to a bucket aren't built yet.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => driver(ctx, "backup", input),
    },
    backups: {
      description: "Every server's backups of desktops, newest first.",
      input: {"properties": {"desktop": {"description": "only this desktop's", "type": "string"}, "here": {"description": "only this server's", "type": "boolean"}}, "type": "object"},
      run: (ctx, input) => driver(ctx, "backups", input),
    },
    restore: {
      description: "Make a desktop on this server from a backup: the newest of the desktop named, from any server, or the one with blob.",
      input: {"properties": {"as": {"description": "the new desktop's name; default the same", "type": "string"}, "blob": {"description": "a particular backup's blob", "type": "string"}, "desktop": {"description": "the backed-up desktop's name", "type": "string"}, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "type": "object"},
      run: (ctx, input) => driver(ctx, "restore", input),
    },
    move: {
      description: "Move a desktop to another server: backs it up, restores it there and stops it here, kept until you delete it. Takes a few minutes.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}, "to": {"description": "the server to move it to", "type": "string"}}, "required": ["desktop", "to"], "type": "object"},
      run: (ctx, input) => driver(ctx, "move", input),
    },
    get_browser_state: {
      description: "Read-only browser inspection. Mode 1 (bind): pass pid + window_id of a native browser window to classify it, correlate it to a CDP target (exact-or-refuse), and mint a session-scoped target id plus tab ids. Mode 2 (snapshot): pass target_id + tab_id. The dom_refs_v1 compatibility format returns composed DOM refs. semantic_v2 joins accessibility, DOM, layout, and viewport state; ranks visible content before retained/offscreen state; and returns a semantic outline, typed action refs, content refs, scoped reads, and opaque continuation. Never performs setup: a browser that is not attached yet refuses with browser_consent_required or browser_requires_setup, and detail.next_call names the browser_prepare call that attaches it. Use this route for any web page in Chrome, Edge or Brave; desktop pointer tools move the user's pointer. Setup and recovery: `get_guide({topic:\"browser\"})`.",
      input: {"additionalProperties": true, "properties": {"continuation": {"description": "Opaque continuation minted by an earlier semantic_v2 response.", "type": "string"}, "desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "include_screenshot": {"default": false, "description": "Capture the exact tab viewport as PNG through CDP without selecting the tab or foregrounding its native window. The request refuses if capture cannot be completed.", "type": "boolean"}, "pid": {"description": "Native browser process id (bind mode).", "type": "integer"}, "query": {"description": "Read-only semantic match over role, accessible name, and visible text.", "type": "string"}, "scope_ref": {"description": "Current semantic/content ref whose subtree should be observed.", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "snapshot_format": {"description": "Versioned snapshot contract. dom_refs_v1 remains the compatibility default.", "enum": ["dom_refs_v1", "semantic_v2"], "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}, "window_id": {"description": "Native window id owned by pid (bind mode).", "type": "integer"}}, "type": "object"},
      run: (ctx, input) => driver(ctx, "get_browser_state", input),
    },
    browser_prepare: {
      description: "Attach the browser tools to a browser. For a page in the user's running Chrome, Edge or Brave, call it once with pid, window_id, session and strategy {\"kind\":\"existing_profile\"} when get_browser_state refuses with browser_consent_required or browser_requires_setup; then call get_browser_state again and act with browser_click, browser_type or browser_pointer. It attaches, or refuses with nothing changed and says why. Explicitly prepare an owned DevTools endpoint for a browser. pid is required for an existing process or existing-profile attachment, and optional only for allow_launch=true with an isolated profile. Existing endpoints are detected without side effects. Acting setup for an isolated profile follows the runtime permission mode and optional capability manifest. It requires allow_launch=true, launches a separate browser, and never copies, modifies, or terminates the requested user profile. Without pid, only a platform-attested system Chrome/Edge installation (or a root-owned package payload on Linux) is eligible; redirects and user-controlled locations fail closed. Existing-profile attachment is explicit and follows the runtime's immutable permission mode: standard requires an explicit --grant existing-profile launch grant or an embedding authorization host, bounded requires a launch-approved exact resource manifest, and unrestricted requires explicit trusted startup risk acceptance. Ordinary MCP transport approval never proves profile authorization. On proven platforms, an authorized request also permits one bounded exact-window setup: open the recognized browser product's fixed remote-debugging page, toggle its uniquely matched per-instance checkbox, prove the PID-owned loopback endpoint, and close the temporary tab. Every visible effect is reported; ambiguity is refused. Where attaching would bring the browser in front of the user's app (macOS Chrome shows its own remote-debugging prompt in front whenever the driver opens a new browser connection; sessions for the same browser share one approved connection, kept for up to 30 minutes idle and 8 hours in total, so later sessions do not prompt again), existing-profile attachment refuses with background_unavailable unless delivery_mode is \"foreground\" (the app that was in front is brought back afterwards) or the browser window is on another Space than the user's, where it attaches in the background while the driver reverts the browser's own brief activations. A driver-owned isolated launch result carries an advisory lifecycle object: the browser process ends with the calling session after idle_expires_in_seconds without session activity, an isolated_named profile persists after the session and an isolated_new profile does not, and explicit end, disconnect, revocation or shutdown end it earlier. Setup and recovery: `get_guide({topic:\"browser\"})`.",
      input: {"additionalProperties": true, "properties": {"allow_launch": {"description": "Allow a separate driver-owned isolated Chromium process to be launched (default false).", "type": "boolean"}, "delivery_mode": {"default": "background", "description": "Existing-profile attachment only. background (default) attaches in the background when the browser window is on another Space than the user's, and otherwise refuses with background_unavailable, before any visible effect, where attaching would bring the browser in front of the user's app. foreground accepts that, for a screen nobody else is using or when the user agreed; the app that was in front is brought back afterwards.", "enum": ["background", "foreground"], "type": "string"}, "desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "pid": {"description": "Browser process id to prepare. Required except for a driver-owned isolated_new/isolated_named launch with allow_launch=true.", "type": "integer"}, "profile": {"additionalProperties": false, "description": "Driver-owned isolated Chromium profile to launch with allow_launch=true. mode=isolated_new creates a fresh throwaway profile; mode=isolated_named reuses the named driver-owned profile. Never an existing user profile.", "properties": {"mode": {"enum": ["isolated_new", "isolated_named"], "type": "string"}, "name": {"description": "Required only for isolated_named; 1-64 path-safe ASCII characters.", "type": "string"}}, "required": ["mode"], "type": "object"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "strategy": {"additionalProperties": false, "description": "Attach to an already-running browser instead of launching one. Pass an object, {\"kind\":\"existing_profile\"}, not a string. kind=existing_profile attaches to the user's running profile at pid/window_id under the runtime's profile authorization.", "properties": {"kind": {"enum": ["existing_profile"], "type": "string"}}, "required": ["kind"], "type": "object"}, "window_id": {"description": "Exact native window approval anchor; required for strategy.kind=existing_profile.", "type": "integer"}}, "required": [], "type": "object"},
      run: (ctx, input) => driver(ctx, "browser_prepare", input),
    },
    browser_navigate: {
      description: "Navigate one tab of an exactly-bound browser target to a new URL (http/https/about only). Refused for heuristic bindings. Navigation invalidates all p<snapshot>:<index> refs for the tab.",
      input: {"additionalProperties": true, "properties": {"desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}, "url": {"description": "Destination URL (http:, https:, or about:).", "type": "string"}}, "required": ["target_id", "tab_id", "url"], "type": "object"},
      run: (ctx, input) => driver(ctx, "browser_navigate", input),
    },
    browser_click: {
      description: "Click a page element (by ref) or viewport coordinates in an exactly-bound tab. Default route is trusted hardware-like input (Input.dispatchMouseEvent), and refuses where that route cannot preserve standalone-browser background posture unless delivery_mode=\"foreground\" accepts that the browser window may activate (including Linux Chromium configurations whose trusted route cannot stay in the background; for example a browser inside a sandbox). input_route=\"dom_event\" (synthetic pointer and mouse down/up/click sequence at the element's center, ref required) is used only when explicitly requested; it proves dispatch, not control activation, because trust-gated controls may ignore synthetic events. Refused for heuristic bindings.",
      input: {"additionalProperties": true, "properties": {"delivery_mode": {"default": "background", "description": "background (default) refuses trusted input where it would activate the browser window, including Linux Chromium and macOS Chromium configurations whose trusted route cannot stay in the background. foreground accepts that activation, for a browser whose window nobody else is using (for example inside a sandbox).", "enum": ["background", "foreground"], "type": "string"}, "desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "input_route": {"description": "\"trusted\" (default): Input.dispatchMouseEvent. It refuses rather than foregrounding a standalone browser. \"dom_event\": synthetic full-background DOM click, only when explicitly requested. Dispatch does not prove the control activated; refresh page state and verify the expected postcondition.", "enum": ["trusted", "dom_event"], "type": "string"}, "ref": {"description": "Page element ref in the p<snapshot>:<index> namespace from get_browser_state. Refs are invalidated by navigation and by newer snapshots of the same tab.", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}, "x": {"description": "Viewport x (CSS px) — alternative to ref.", "type": "number"}, "y": {"description": "Viewport y (CSS px) — alternative to ref.", "type": "number"}}, "required": ["target_id", "tab_id"], "type": "object"},
      run: (ctx, input) => driver(ctx, "browser_click", input),
    },
    browser_type: {
      description: "Type text into an exactly-bound tab via the Input domain. mode=\"insert_text\" (default) uses Input.insertText; mode=\"keystrokes\" dispatches per-character key events. Both insert at the caret, so typing into a field that already holds text appends to it; pass replace=true to set the field instead, or to clear it by typing an empty string. Pass a ref to an editable element from the latest snapshot. A ref is required; heuristic bindings are refused.",
      input: {"additionalProperties": true, "properties": {"desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "mode": {"description": "insert_text (default): bulk Input.insertText. keystrokes: per-character Input.dispatchKeyEvent.", "enum": ["insert_text", "keystrokes"], "type": "string"}, "ref": {"description": "Page element ref in the p<snapshot>:<index> namespace from get_browser_state. Refs are invalidated by navigation and by newer snapshots of the same tab.", "type": "string"}, "replace": {"description": "false (default): insert at the caret, appending to whatever the field already holds. true: select the element's whole content first so the text replaces it — with an empty text this clears the field. Replacement goes through the selection, so beforeinput/input still fire and framework state stays consistent.", "type": "boolean"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}, "text": {"description": "Text to type.", "type": "string"}}, "required": ["target_id", "tab_id", "ref", "text"], "type": "object"},
      run: (ctx, input) => driver(ctx, "browser_type", input),
    },
    browser_select: {
      description: "Select one option of a native HTML <select> ref on an exactly-bound tab without opening its popup. Pass exactly one of label or value; exactly one option must equal it. Refuses non-native, multiple-selection, and disabled selects, disabled options, and zero or ambiguous matches before changing anything. A change dispatches bubbling input and change events; they are synthetic, so verify the page accepted the selection. Selecting the option that is already selected dispatches no events.",
      input: {"additionalProperties": false, "properties": {"desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "label": {"description": "Exact visible label of the option to select.", "type": "string"}, "ref": {"description": "Page element ref in the p<snapshot>:<index> namespace from get_browser_state. Refs are invalidated by navigation and by newer snapshots of the same tab.", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}, "value": {"description": "Exact value attribute of the option to select.", "type": "string"}}, "required": ["target_id", "tab_id", "ref"], "type": "object"},
      run: (ctx, input) => driver(ctx, "browser_select", input),
    },
    browser_dialog: {
      description: "Inspect or resolve a page-owned JavaScript alert, confirm, prompt, or beforeunload dialog on one exactly-bound tab. This never handles browser permission UI, extension UI, native dialogs, or file pickers. Inspect returns an opaque dialog_id; accept/dismiss require that exact current id. Resolution defaults to background delivery; Linux callers must explicitly request foreground delivery because Chromium's native modal cannot be resolved there without changing foreground posture.",
      input: {"additionalProperties": true, "properties": {"action": {"description": "inspect returns the current dialog and its dialog_id; accept or dismiss resolves that exact dialog.", "enum": ["inspect", "accept", "dismiss"], "type": "string"}, "delivery_mode": {"default": "background", "description": "Requested foreground posture for accept/dismiss. Linux Chromium requires foreground; inspect is read-only.", "enum": ["background", "foreground"], "type": "string"}, "desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "dialog_id": {"description": "Opaque current dialog generation returned by action=inspect.", "type": "string"}, "prompt_text": {"description": "Sensitive response text, valid only when accepting a prompt dialog.", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. Omit it to use the authenticated transport's implicit lifecycle session. Browser targets, tabs, and refs belong to the resolved lifecycle session.", "type": "string"}, "tab_id": {"description": "Opaque tab id from get_browser_state (session-scoped).", "type": "string"}, "target_id": {"description": "Opaque browser target id minted by get_browser_state (session-scoped; never a CDP id).", "type": "string"}}, "required": ["target_id", "tab_id", "action"], "type": "object"},
      run: (ctx, input) => driver(ctx, "browser_dialog", input),
    },
    browser_pointer: {
      description: "Perform hover, right-click, double-click, scroll, or drag in an exactly-bound browser tab. Semantic refs must declare pointer for hover, right-click, double-click, and drag; scroll accepts a scroll or pointer capability. The trusted route uses CDP Input events and refuses if standalone background posture cannot be preserved. The explicit dom_event route requires a page ref and synthesizes full-background DOM events. Never activates or brings a tab to the foreground.",
      input: {"additionalProperties": true, "properties": {"action": {"description": "Pointer gesture. scroll needs delta_x or delta_y; drag needs destination_ref or to_x/to_y.", "enum": ["hover", "right_click", "double_click", "scroll", "drag"], "type": "string"}, "delivery_mode": {"default": "background", "description": "background (default) refuses trusted input where it would activate the browser window, including Linux Chromium and macOS Chromium configurations whose trusted route cannot stay in the background. foreground accepts that activation, for a browser whose window nobody else is using (for example inside a sandbox).", "enum": ["background", "foreground"], "type": "string"}, "delta_x": {"description": "Horizontal scroll delta in CSS pixels.", "type": "number"}, "delta_y": {"description": "Vertical scroll delta in CSS pixels.", "type": "number"}, "desktop": {"description": "which desktop, by name; default the only one", "type": "string"}, "destination_ref": {"description": "Drag destination page ref in the exact same frame.", "type": "string"}, "input_route": {"default": "trusted", "description": "trusted sends CDP Input events; dom_event synthesizes DOM events in the page and requires ref.", "enum": ["trusted", "dom_event"], "type": "string"}, "ref": {"description": "Origin page ref. Alternative to x/y.", "type": "string"}, "session": {"description": "For multi-call work, prefer a short public session label and repeat it on every call that accepts it. This tool requires the label that owns its browser target, tab, and refs.", "type": "string"}, "tab_id": {"description": "Opaque tab id minted by get_browser_state.", "type": "string"}, "target_id": {"description": "Opaque target id minted by get_browser_state.", "type": "string"}, "to_x": {"description": "Drag destination viewport x in CSS pixels.", "type": "number"}, "to_y": {"description": "Drag destination viewport y in CSS pixels.", "type": "number"}, "x": {"description": "Origin viewport x in CSS pixels.", "type": "number"}, "y": {"description": "Origin viewport y in CSS pixels.", "type": "number"}}, "required": ["target_id", "tab_id", "session", "action"], "type": "object"},
      run: (ctx, input) => driver(ctx, "browser_pointer", input),
    },
    _running: {
      reads: true,
      description: "Which of your servers run the driver, for the screens to say when none does.",
      input: { type: "object", properties: {} },
      run: async (ctx) => ({ servers: await running(ctx), needs: NEEDS }),
    },
  },

  screens: {
    main: {
      title: "Desktops",
      html: `
<style>
  body { max-width: 1120px; }
  header { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; margin: 4px 0 18px; }
  header .uses { flex: 1; min-width: 0; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; color: var(--muted); font-size: 13px; }
  .meter { display: inline-flex; align-items: center; gap: 4px; flex: none; }
  button.primary { font: inherit; font-size: 15px; font-weight: 600; border: 0; border-radius: 980px; padding: 7px 16px; color: white; background: var(--accent); cursor: pointer; }
  button.primary:disabled { opacity: 0.5; cursor: default; }
  button.plain { font: inherit; font-size: 15px; border: 0; background: none; color: var(--accent); cursor: pointer; padding: 0; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(240px, 40vw), 1fr)); gap: 20px 14px; }
  .tile { display: block; color: inherit; min-width: 0; }
  .pic { aspect-ratio: 16 / 10; border-radius: 10px; overflow: hidden; background: var(--card); border: 0.5px solid var(--line); display: grid; place-items: center; color: var(--muted); font-size: 13px; }
  .pic img { width: 100%; height: 100%; object-fit: cover; display: block; }
  a.tile:hover .pic { border-color: var(--accent); }
  .tile.free .pic { background: none; border: 1px dashed var(--faint); }
  .tile.free .name { color: var(--muted); font-weight: 500; }
  .name { font-weight: 600; margin-top: 8px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .line { display: flex; align-items: center; gap: 6px; color: var(--muted); font-size: 13px; white-space: nowrap; min-width: 0; }
  .line .who { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .line .host { margin-left: auto; padding-left: 6px; }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--faint); flex: none; }
  .dot.on { background: #34c759; }
  .banner { background: var(--card); border-radius: var(--radius); padding: 14px 16px; margin: 0 0 18px; display: flex; gap: 12px; align-items: center; flex-wrap: wrap; }
  .banner .text { flex: 1; min-width: 200px; }
  svg { flex: none; display: block; }
</style>
<header>
  <div class="uses" id="uses"></div>
  <button class="primary" id="sign-in">Sign in to a site</button>
</header>
<div id="banner"></div>
<div class="grid" id="grid"><div class="empty">Loading…</div></div>
<h2 class="section">Browser Profiles</h2>
<div class="list" id="profiles"></div>
<p class="muted small" style="margin: 6px 4px">Each profile is a browser of its own, with its own sign-ins, like Personal and PTC. Agents' desktops start with a copy of one, and apps like Messenger link to one.</p>
<script>
  one.title("Desktops");
  const grid = document.getElementById("grid");
  const pics = {}; // by server/name: a data URL
  const cpus = {}; // by server/name: recent CPU percentages, oldest first
  const links = {}; // by server/name: { url, until }
  let desktops = [];
  let signing = null; // { desktop, server, login, url }, once a sign-in is open
  let profiles = []; // { profile, server, saved }

  const key = (d) => \`\${d.server || ""}/\${d.name}\`;
  const at = (d) => (d.server ? { desktop: d.name, server: d.server } : { desktop: d.name });
  const host = (s) => String(s || "").split(".")[0];

  function size(bytes) {
    if (!(bytes > 0)) return "0 MB";
    return bytes >= 1 << 30 ? \`\${(bytes / (1 << 30)).toFixed(1)} GB\` : \`\${Math.round(bytes / (1 << 20))} MB\`;
  }
  // Memory in use and the most it may use, in bytes, however the driver says it.
  function memory(d) {
    const m = d.memory;
    if (m && typeof m === "object") return [m.used ?? m.usage ?? 0, m.limit ?? m.total ?? 0];
    return [m ?? 0, d.memory_limit ?? 0];
  }
  const tone = (share) => (share >= 0.85 ? "#ff3b30" : share >= 0.6 ? "#ff9f0a" : "#34c759");
  function ring(share) {
    const r = 5.5, c = 2 * Math.PI * r, s = Math.max(0, Math.min(1, share || 0));
    return \`<svg width="14" height="14" viewBox="0 0 14 14"><circle cx="7" cy="7" r="\${r}" fill="none" stroke="var(--line)" stroke-width="2.5"/>\` +
      \`<circle cx="7" cy="7" r="\${r}" fill="none" stroke="\${tone(s)}" stroke-width="2.5" stroke-dasharray="\${c * s} \${c}" transform="rotate(-90 7 7)" stroke-linecap="round"/></svg>\`;
  }
  function spark(values) {
    const w = 32, h = 12, top = Math.max(100, ...values);
    const pts = (values.length > 1 ? values : [values[0] || 0, values[0] || 0]).map((v, i, a) => \`\${(i / (a.length - 1)) * w},\${h - 1 - (v / top) * (h - 2)}\`);
    return \`<svg width="\${w}" height="\${h}"><polyline points="\${pts.join(" ")}" fill="none" stroke="#0a84ff" stroke-width="1.5" stroke-linejoin="round"/></svg>\`;
  }
  function meters(used, limit, cpu, history) {
    const share = limit ? used / limit : 0;
    return \`<span class="meter">\${ring(share)}\${size(used)}</span><span class="meter">\${spark(history)}\${Math.round(cpu || 0)}%</span>\`;
  }

  // The driver's list: { desktops: [...] }, or, from a driver before that, lines of "name: state, ...".
  function parse(listed) {
    if (listed && Array.isArray(listed.desktops)) return listed.desktops;
    if (typeof listed !== "string" || listed.startsWith("No desktops")) return [];
    return listed.split("\\n").map((l) => {
      const [name, rest = ""] = l.split(": ");
      return { name, state: rest.split(",")[0] };
    });
  }

  const free = (d) => d.spare && !d.borrowed;
  // A tile's title says what the desktop is doing; its line, who for.
  function title(d) {
    const b = d.borrowed;
    if (b?.signing_in) return "Signing in to sites";
    if (b) return b.for || "Borrowed";
    if (free(d)) return "Free";
    return d.name[0].toUpperCase() + d.name.slice(1);
  }
  function who(d) {
    const b = d.borrowed;
    if (b?.signing_in) return b.login && b.login !== "default" ? \`Profile \${b.login}\` : "";
    if (b) return b.by || "An agent";
    if (free(d)) return "Ready for the next agent";
    return "";
  }

  function draw() {
    if (!desktops.length) {
      grid.innerHTML = \`<div class="empty" style="grid-column:1/-1">No desktops yet. An agent makes one when it needs a browser, or sign in to a site to start one.</div>\`;
    } else {
      const order = (d) => (d.borrowed?.signing_in ? 0 : d.borrowed ? 1 : d.spare ? 3 : 2);
      const shown = [...desktops].sort((a, b) => order(a) - order(b) || (Date.parse(a.borrowed?.since) || 0) - (Date.parse(b.borrowed?.since) || 0) || a.name.localeCompare(b.name));
      grid.innerHTML = shown.map((d) => {
        const k = key(d), running = d.state === "running";
        const pic = free(d) ? "" : pics[k] ? \`<img src="\${pics[k]}" alt="">\` : running ? "" : one.esc(d.state || "stopped");
        const [used, limit] = memory(d);
        const line = [
          \`<span class="dot\${running ? " on" : ""}"></span>\`,
          who(d) ? \`<span class="who">\${one.esc(who(d))}</span>\` : "",
          running && d.memory != null && !free(d) ? meters(used, limit, d.cpu, cpus[k] || []) : "",
          d.server ? \`<span class="host">\${one.esc(host(d.server))}</span>\` : "",
        ].join("");
        const tag = links[k] && !free(d) ? \`a href="\${one.esc(links[k].url)}" target="_blank" rel="noopener"\` : "div";
        return \`<\${tag} class="tile\${free(d) ? " free" : ""}" title="\${one.esc(d.name)}"><div class="pic">\${pic}</div>\` +
          \`<div class="name">\${one.esc(title(d))}</div><div class="line">\${line}</div></\${tag.split(" ")[0]}>\`;
      }).join("");
    }
    // What they all use, together.
    const running = desktops.filter((d) => d.state === "running");
    let used = 0, limit = 0, cpu = 0;
    for (const d of running) {
      const [u, l] = memory(d);
      used += u; limit += l; cpu += d.cpu || 0;
    }
    const borrowed = desktops.filter((d) => d.borrowed && !d.borrowed.signing_in).length;
    const parts = [\`\${running.length} running\`];
    if (borrowed) parts.push(\`\${borrowed} borrowed\`);
    const total = cpus[""] || [];
    document.getElementById("uses").innerHTML = parts.map(one.esc).join(" · ") +
      (running.some((d) => d.memory != null) ? \` · \${meters(used, limit, cpu, total)}\` : "");
    drawBanner();
  }

  function drawBanner() {
    const banner = document.getElementById("banner");
    const open = signing || desktops.find((d) => d.borrowed?.signing_in);
    if (!open) return (banner.innerHTML = "");
    const s = signing || { desktop: open.name, server: open.server, login: open.borrowed.login };
    const url = s.url || links[key({ name: s.desktop, server: s.server })]?.url;
    banner.innerHTML = \`<div class="banner"><div class="text"><b>Signing in to the \${one.esc(s.login || "default")} profile on \${one.esc(s.desktop)}</b><div class="muted small">Open the desktop, sign in to your sites in its browser (messenger.com, say), then save. Agents' desktops start signed in to them.</div></div>\` +
      (url ? \`<a class="plain" href="\${one.esc(url)}" target="_blank" rel="noopener">Open desktop</a>\` : "") +
      \`<button class="primary" id="save">Save profile</button></div>\`;
    document.getElementById("save").onclick = () => save(s);
  }

  async function save(s) {
    const button = document.getElementById("save");
    button.disabled = true;
    button.textContent = "Saving…";
    try {
      await one.call("save_profile", { desktop: s.desktop, ...(s.server ? { server: s.server } : {}), ...(s.login ? { login: s.login } : {}) });
      signing = null;
      await refresh();
      await loadProfiles();
    } catch (e) {
      button.disabled = false;
      button.textContent = "Save profile";
      alert(e.message);
    }
  }

  // Opens a desktop's browser on a profile (made if new), for the person to sign in to sites in it.
  async function signIn(button, profile) {
    const label = button.textContent;
    button.disabled = true;
    button.textContent = "Opening a desktop…";
    try {
      const r = await one.call("sign_in", profile ? { profile } : {});
      signing = { desktop: r.desktop, server: r.server, login: r.profile || r.login, url: r.url };
      await refresh();
      return true;
    } catch (err) {
      alert(err.message);
    } finally {
      button.disabled = false;
      button.textContent = label;
    }
  }
  document.getElementById("sign-in").onclick = (e) => signIn(e.currentTarget);

  async function loadProfiles() {
    try { profiles = (await one.call("profiles", {})).profiles || []; } catch { profiles = []; }
    const list = document.getElementById("profiles");
    list.innerHTML = profiles.map((p) => \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(p.profile)}</span></div>\` +
      \`<div class="sub small">Saved \${one.esc(one.ago(Date.parse(p.saved)))}\${p.server ? " on " + one.esc(host(p.server)) : ""}</div></div>\` +
      \`<button class="plain" data-profile="\${one.esc(p.profile)}">Sign in to a site</button></div>\`).join("") +
      \`<div class="row plain"><div class="main"><input id="new-profile" class="search" style="margin:0" placeholder="New profile, like Personal or PTC" autocapitalize="words" autocorrect="off"></div>\` +
      \`<button class="plain" id="add-profile">Add</button></div>\`;
    for (const b of list.querySelectorAll("[data-profile]")) b.onclick = () => signIn(b, b.dataset.profile);
    const add = document.getElementById("add-profile");
    add.onclick = async () => {
      const name = document.getElementById("new-profile").value.trim();
      if (name && (await signIn(add, name))) document.getElementById("new-profile").value = "";
    };
  }
  loadProfiles();

  // A picture of each running desktop, and a link to open it, kept until shortly before the link stops working.
  async function pictures() {
    await Promise.all(desktops.filter((d) => d.state === "running" && !free(d)).map(async (d) => {
      const k = key(d);
      try {
        const r = await one.call("screen", { ...at(d), width: 480 });
        const img = (r.content || []).find((c) => c.type === "image");
        if (img) pics[k] = \`data:\${img.mimeType};base64,\${img.data}\`;
      } catch {}
      if (!links[k] || links[k].until < Date.now()) {
        try {
          const r = await one.call("view", { ...at(d), minutes: 60 });
          const url = typeof r === "string" ? r.split("\\n")[0] : r.url;
          if (url) links[k] = { url, until: Date.now() + 50 * 60_000 };
        } catch {}
      }
    }));
  }

  async function refresh() {
    try {
      desktops = parse(await one.call("list", {}));
    } catch (e) {
      grid.innerHTML = \`<div class="empty error" style="grid-column:1/-1">\${one.esc(e.message)}</div>\`;
      return;
    }
    let all = 0;
    for (const d of desktops.filter((d) => d.state === "running")) {
      const h = (cpus[key(d)] ||= []);
      h.push(d.cpu || 0);
      if (h.length > 20) h.shift();
      all += d.cpu || 0;
    }
    (cpus[""] ||= []).push(all);
    if (cpus[""].length > 20) cpus[""].shift();
    draw();
    await pictures();
    draw();
  }

  let busy = false;
  async function tick() {
    if (busy || document.visibilityState === "hidden") return;
    busy = true;
    try { await refresh(); } finally { busy = false; }
  }
  tick();
  setInterval(tick, 5000);
  document.addEventListener("visibilitychange", tick);
</script>
<div class="card" id="needs" hidden></div>
<script>
one.call("_running").then(({ servers, needs }) => {
  if (servers.length) return;
  const n = document.getElementById("needs");
  n.textContent = needs;
  n.hidden = false;
  
});
</script>`,
    },
  },
};
