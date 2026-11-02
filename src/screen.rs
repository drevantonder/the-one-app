//! A screen's page, as every node serves it, an app's or a client's own: the design kit and the bridge (`src/screen/`), what the
//! screen is told (`window.__one`), and the screen's own HTML.

/// A screen's page: `context` is `{ app, screen, params, me, scope, config }` (a Settings screen has no app or scope), `body` its HTML.
pub fn page(context: &serde_json::Value, body: &str) -> String {
    let mut context = context.clone();
    // The domain whose people join by signing in with Google, for Users to say so.
    if crate::config::SIGN_IN == "google" {
        context["config"]["join_domain"] = crate::config::JOIN_DOMAIN.into();
    }
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

/// The clients' own screens, by name, the same on every client: the person's pages, a page's Services, a request for a
/// Vault item, and Settings' (`settings/`). They act through the bridge's `one.api` as the person looking, which a
/// client answers only for these.
const CLIENT_SCREENS: &[(&str, &str)] = &[
    ("pages", concat!("<script>", include_str!("screen/widgets.js"), include_str!("screen/home.js"), "</script>", include_str!("screen/pages.html"))),
    ("services", concat!("<script>", include_str!("screen/home.js"), "</script>", include_str!("screen/services.html"))),
    ("request", include_str!("screen/request.html")),
    ("account", include_str!("screen/settings/account.html")),
    ("appearance", include_str!("screen/settings/appearance.html")),
    ("vault", include_str!("screen/settings/vault.html")),
    ("requests", include_str!("screen/settings/requests.html")),
    ("groups", include_str!("screen/settings/groups.html")),
    ("users", include_str!("screen/settings/users.html")),
    ("general", include_str!("screen/settings/general.html")),
    ("backups", include_str!("screen/settings/backups.html")),
    ("devices", include_str!("screen/settings/devices.html")),
    ("agents", include_str!("screen/settings/agents.html")),
];

/// A client's own screen's HTML, or a note that it isn't there.
pub fn client_screen(name: &str) -> &'static str {
    CLIENT_SCREENS.iter().find(|(n, _)| *n == name).map_or("<div id=\"missing\"></div><script>missing.innerHTML = one.notFound()</script>", |(_, html)| html)
}
