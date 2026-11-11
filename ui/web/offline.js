// While the user's devices don't answer, what the window shows from what it kept, as one banner across its top:
// every screen showing its kept page, and the window's state, each with how it's asked again (`retry`) and when what
// it shows was kept. Asked again by itself, and at Try Again; the banner goes once all have answered. The window draws
// it again as it changes (`whenChanged`).
import { net } from "./net.js";
import { retryWait } from "./request.js";
import { h, ui } from "./view.js";

/// How long an answer may take on a connection that's only slow (a phone through the relay), before the window says
/// its devices aren't answering; and with no network at all, which the browser knows at once.
export const late = () => (navigator.onLine ? 12_000 : 1000);

const changes = [];
const redraw = () => changes.forEach((fn) => fn());
addEventListener("online", () => offline.waiting.size && offline.now());
net.whenRelay((up) => (up ? offline.done("relay") : offline.add("relay", () => net.still())));

export const offline = {
  waiting: new Map(),
  shown: false,
  trying: false,
  timer: null,
  misses: 1,
  whenChanged: (fn) => changes.push(fn),
  add(key, retry, at) {
    if (this.waiting.has(key)) return;
    this.waiting.set(key, { retry, at: at || Date.now() });
    setTimeout(() => this.waiting.size && !this.shown && ((this.shown = true), redraw()), 1500);
    if (!this.timer) this.later();
  },
  done(key) {
    if (!this.waiting.delete(key) || this.waiting.size) return;
    clearTimeout(this.timer);
    Object.assign(this, { timer: null, misses: 1, shown: false, trying: false });
    redraw();
  },
  later() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.now(), Math.max(5000, retryWait(this.misses++)));
  },
  async now() {
    clearTimeout(this.timer);
    this.trying = true;
    redraw();
    await Promise.all([...this.waiting].map(([key, w]) => w.retry().then(() => this.done(key), () => {})));
    this.trying = false;
    if (this.waiting.size) (this.later(), redraw());
  },
  /// The banner, once it's been a moment: one thin line, with Try Again while it isn't already trying.
  banner() {
    if (!this.shown || !this.waiting.size) return null;
    const at = new Date(Math.min(...[...this.waiting.values()].map((w) => w.at)));
    const today = at.toDateString() === new Date().toDateString();
    const when = new Intl.DateTimeFormat(undefined, today ? { timeStyle: "short" } : { dateStyle: "medium", timeStyle: "short" }).format(at);
    return h(ui.Banner, { action: this.trying ? undefined : { label: "Try Again", run: () => this.now() } }, `Offline. Last updated ${when}.`);
  },
};
