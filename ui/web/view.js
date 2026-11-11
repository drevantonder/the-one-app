// What the page draws with: @one/ui (ui/kit), whose ui.js, loaded before this, keeps it on `OneUI`; and #root,
// which shows the window or what stands before it (signing in, waiting to be let in, a problem).
export const ui = OneUI["@one/ui"];
export const h = OneUI.react.createElement;
export const $ = (sel, el = document) => el.querySelector(sel);

const view = OneUI["react-dom/client"].createRoot($("#root"));
export const logo = "/logo.png";

// The screens before the window. Going from one to another (a new kind, or a new key) is a transition, which draws a
// moment later; anything else, as the window has to be, is drawn at once, so what's in it is there for what comes next
// (a screen's frame going into the window). The latest asked for is the one a transition still to draw draws.
const stages = new Set([ui.Welcome, ui.Join, ui.Starting]);
let shown = null;
const draw = () => OneUI["react-dom"].flushSync(() => view.render(shown));

/// The screen before the window, kept over it as it was, until the window's first screen has drawn (`reveal`).
let cover = null;
function keepOver() {
  const was = $("#root").firstElementChild?.cloneNode(true);
  if (!was) return;
  cover = document.createElement("div");
  cover.className = "window-cover";
  cover.append(was);
  document.body.append(cover);
}
export function reveal() {
  const leaving = cover;
  cover = null;
  if (!leaving) return;
  if (ui.still()) return leaving.remove();
  ui.animate(leaving, { opacity: [1, 0] }, ui.moves.settle).finished.then(() => leaving.remove());
}

export function show(element) {
  const changing = shown && stages.has(shown.type) && stages.has(element.type) && (shown.type !== element.type || shown.key !== element.key);
  if (shown && stages.has(shown.type) && !stages.has(element.type)) keepOver();
  shown = element;
  if (changing) ui.transition(draw);
  else draw();
}
/// What stops the window, in place of it: a title and one line.
export const message = (title, text) => h(ui.Welcome, { key: title, logo, title, text });
