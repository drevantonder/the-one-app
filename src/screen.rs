//! A screen's page, as every node serves it, an app's or Settings': the design kit and the bridge (`src/screen/`), what the screen is
//! told (`window.__one`), and the screen's own HTML.

/// A screen's page: `context` is `{ app, screen, params, me, scope, config }` (a Settings screen has no app or scope), `body` its HTML.
pub fn page(context: &serde_json::Value, body: &str) -> String {
    // `</` can't appear inside the inline script, so it can't end it early.
    let context = context.to_string().replace("</", "<\\/");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\">\
         <style>{kit}:root{{--accent:{accent}}}</style><script>window.__one={context};{bridge}</script></head><body>{body}</body></html>",
        kit = include_str!("screen/kit.css"),
        accent = crate::config::ACCENT,
        bridge = concat!(include_str!("screen/bridge.js"), include_str!("screen/kit.js"), include_str!("screen/code.js"), include_str!("screen/markdown.js")),
    )
}
