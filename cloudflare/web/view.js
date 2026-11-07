// What the page draws with: @one/ui (src/screen/ui), whose ui.js, loaded before this, keeps it on `OneUI`; and #root,
// which shows the window or what stands before it (signing in, waiting to be let in, a problem).
export const ui = OneUI["@one/ui"];
export const h = OneUI.react.createElement;
export const $ = (sel, el = document) => el.querySelector(sel);

const view = OneUI["react-dom/client"].createRoot($("#root"));
/// Drawn at once, so what's in it is there for what comes next (a screen's frame going into the window).
export const show = (element) => OneUI["react-dom"].flushSync(() => view.render(element));
export const message = (title, text, note) => h(ui.Unavailable, { title, text, note });
