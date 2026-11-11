// How the window goes from one screen to the next: both are layers in the screen's place (screens.js), the next drawn
// behind the one showing until it has drawn, and then this moves them. With no motion wanted it's a plain swap.
import { $, h, ui } from "./view.js";

const { animate, moves, still } = ui;

let running = null;
/// What's moving ends where it was going, at once.
export const settle = () => running?.();

function play(animations, clean) {
  const done = () => {
    if (running !== done) return;
    running = null;
    animations.forEach((a) => a.cancel?.());
    clean();
  };
  running = done;
  Promise.all(animations.map((a) => a.finished)).then(done, done);
}

/// The app's icon where the home screen's is, over what moves, so the first frame is the icon itself.
function iconOver({ tile, rect: [x, y, w] }) {
  const el = Object.assign(document.createElement("div"), { className: "screen-tile-icon" });
  Object.assign(el.style, { left: `${x}px`, top: `${y}px` });
  const root = OneUI["react-dom/client"].createRoot(el);
  OneUI["react-dom"].flushSync(() => root.render(h(ui.AppIcon, { app: tile, size: w })));
  document.body.append(el);
  return { el, remove: () => (root.unmount(), el.remove()) };
}

/// A layer's clip-path that shows only `rect`, with an icon's corners or those of `round`.
function clipTo(el, [x, y, w, h], round = w * 0.225) {
  const b = el.getBoundingClientRect();
  return `inset(${y - b.top}px ${b.right - x - w}px ${b.bottom - y - h}px ${x - b.left}px round ${round}px)`;
}
const WHOLE = "inset(0px 0px 0px 0px round 0px)";

/// On a phone, the app's screen and its bar growing out of its icon on the home screen (`grow`), or shrinking back into
/// it (`shrink`), as the icon fades out of it or into it.
function zoom(from, to, { tile, rect }, shrink, clean) {
  const app = shrink ? from : to;
  const icon = iconOver({ tile, rect });
  app.style.background = "var(--background)";
  if (shrink) (from.style.zIndex = "1"), (to.style.zIndex = "0");
  const bar = $('[data-slot="shell-bar"]');
  if (bar) bar.style.background = "var(--background)";
  // Going back it's the icon's shape first, a square the window's width, near where the icon is, and then only smaller.
  // Its corners stay small while it's large, and it's gone by the time it's the icon's size, so it never cuts into the
  // home screen behind it.
  const square = [0, Math.max(0, Math.min(innerHeight - innerWidth, rect[1] - innerWidth / 2)), innerWidth, innerWidth];
  const clip = (el) => (shrink ? [WHOLE, clipTo(el, square, 20), clipTo(el, rect)] : [clipTo(el, rect), WHOLE]);
  const times = { ...moves.travel, ...(shrink ? { times: [0, 0.3, 1] } : {}) };
  const parts = bar ? [app, bar] : [app];
  play([
    ...parts.map((el) => animate(el, { clipPath: clip(el) }, times)),
    // Growing, the home screen goes once it's covered: the app's edges take the last moment to reach the window's.
    ...(shrink ? parts.map((el) => animate(el, { opacity: [1, 1, 0, 0] }, { duration: moves.travel.duration, ease: "linear", times: [0, 0.1, 0.5, 1] })) : [animate(from, { opacity: [1, 0] }, { ...moves.quick, delay: 0.06 })]),
    animate(icon.el, { opacity: shrink ? [0, 1] : [1, 0] }, shrink ? { ...moves.quick, delay: 0.13 } : moves.quick),
  ], () => (icon.remove(), bar?.removeAttribute("style"), clean()));
}

/// Takes `from` away and shows `to`, as `how` says (`rise`, `cross`, `grow` or `shrink`), then `done`, which puts `from` away.
export function move(from, to, how, done) {
  settle();
  to.classList.remove("coming");
  // Over a layer that's older in the page (a page kept alive, or the spare frame's), and the reverse for a shrink below.
  to.style.zIndex = "1";
  from.style.pointerEvents = "none";
  const clean = () => {
    to.removeAttribute("style");
    done();
  };
  if (still()) return clean();
  if (how.grow) return zoom(from, to, how.grow, false, clean);
  if (how.shrink) return zoom(from, to, how.shrink, true, clean);
  const out = animate(from, { opacity: [1, 0] }, moves.quick);
  const arrive = how.cross
    ? animate(to, { opacity: [0, 1] }, moves.settle)
    : animate(to, { opacity: [0, 1], translate: ["0 10px", "0 0"] }, moves.settle);
  play([out, arrive], clean);
}
