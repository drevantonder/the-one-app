//! A screen's page, as every node serves it, an app's or a client's own: the design kit and the bridge (`src/screen/`), what the
//! screen is told (`window.__one`), and the screen's own HTML.

/// A screen's page: `context` is `{ app, screen, params, me, scope, config }` (a Settings screen has no app or scope), `body` its HTML.
/// A client's own (`own`) acts as the person, and draws what apps say, so it runs only its own scripts: they carry this
/// page's nonce, and its policy refuses every other, inline event handlers too.
pub fn page(context: &serde_json::Value, body: &str, own: bool) -> String {
    let mut context = context.clone();
    // What a person joins, in their words: "your One App", or a workplace's, "the PTC App".
    context["config"]["yours"] = crate::config::YOURS.into();
    // The domain whose people join by signing in with Google, for Users to say so.
    if crate::config::SIGN_IN == "google" {
        context["config"]["join_domain"] = crate::config::JOIN_DOMAIN.into();
    }
    // `</` can't appear inside the inline script, so it can't end it early.
    let context = context.to_string().replace("</", "<\\/");
    let (policy, script, body) = if own {
        let nonce = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>());
        let policy = format!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"script-src 'nonce-{nonce}'; object-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'\">"
        );
        (policy, format!("<script nonce=\"{nonce}\">"), body.replace("<script>", &format!("<script nonce=\"{nonce}\">")))
    } else {
        (String::new(), "<script>".into(), body.to_string())
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">{policy}<meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\">\
         <style>{kit}</style>{script}window.__one={context};{bridge}</script></head><body>{body}</body></html>",
        kit = include_str!("screen/kit.css"),
        bridge = concat!(include_str!("screen/bridge.js"), include_str!("screen/kit.js"), include_str!("screen/code.js"), include_str!("screen/markdown.js")),
    )
}

/// What names a page's content, or any answer's (its ETag): the same for two pages that differ only in their nonce.
pub fn tag(page: &str) -> String {
    use sha2::Digest;
    let nonce = page.split("'nonce-").nth(1).and_then(|rest| rest.get(..32));
    let content = nonce.map_or_else(|| page.to_string(), |nonce| page.replace(nonce, ""));
    format!("\"{}\"", &data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(content.as_bytes()))[..32])
}

/// The clients' own screens, by name, the same on every client: the person's pages, a page's Services, a request for a
/// Vault item, and Settings' (`settings/`). They act through the bridge's `one.api` as the person looking, which a
/// client answers only for these.
const CLIENT_SCREENS: &[(&str, &str)] = &[
    ("pages", concat!("<script>", include_str!("screen/widgets.js"), include_str!("screen/home.js"), "</script>", include_str!("screen/pages.html"))),
    ("services", concat!("<script>", include_str!("screen/home.js"), "</script>", include_str!("screen/services.html"))),
    ("request", include_str!("screen/request.html")),
    ("appearance", include_str!("screen/settings/appearance.html")),
    ("vault", include_str!("screen/settings/vault.html")),
    ("requests", include_str!("screen/settings/requests.html")),
    ("groups", include_str!("screen/settings/groups.html")),
    ("users", include_str!("screen/settings/users.html")),
    ("general", include_str!("screen/settings/general.html")),
    ("backups", include_str!("screen/settings/backups.html")),
    ("devices", include_str!("screen/settings/devices.html")),
    ("connect-ai", include_str!("screen/settings/connect-ai.html")),
];

/// A client's own screen's HTML, or a note that it isn't there.
pub fn client_screen(name: &str) -> &'static str {
    CLIENT_SCREENS.iter().find(|(n, _)| *n == name).map_or("<div id=\"missing\"></div><script>missing.innerHTML = one.notFound()</script>", |(_, html)| html)
}
