//! A screen's page, an app's as every node serves it, or a client's own as every client makes it: the design kit and the bridge (`src/screen/`), what the
//! screen is told (`window.__one`), and the screen's own HTML.

/// Phosphor's icons, each name's path on a 256-point grid (vendor/phosphor), for the web pages that draw screens: the
/// web client's, at the address, and a chat's view, by `screens_icons`.
pub const ICONS: &[u8] = include_bytes!("../vendor/phosphor/icons.json");

/// The kit and the bridge, in the order a page has them: each path under `src/screen/` and its text.
const KIT: &[(&str, &str)] = &[
    ("kit.css", include_str!("screen/kit.css")),
    ("bridge.js", include_str!("screen/bridge.js")),
    ("kit.js", include_str!("screen/kit.js")),
    ("code.js", include_str!("screen/code.js")),
    ("markdown.js", include_str!("screen/markdown.js")),
];

/// A screen's page: `context` is `{ app, screen, params, me, scope, config }` (a Settings screen has no app or scope), `body` its HTML.
/// A client's own (`own`) acts as the person, and draws what apps say, so it runs only its own scripts: they carry this
/// page's nonce, and its policy refuses every other, inline event handlers too. `linked`, the kit and bridge are links
/// to the address's files (`/screen/<path>?v=`, named by content as the gateway names them), which the web client
/// already has, rather than in the page.
pub fn page(context: &serde_json::Value, body: &str, own: bool, linked: bool) -> String {
    page_with(context, body, own.then(|| data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>())), linked)
}

/// `page`, with a client's own screen's `nonce`, or none for an app's.
fn page_with(context: &serde_json::Value, body: &str, nonce: Option<String>, linked: bool) -> String {
    let mut context = context.clone();
    // What a person joins, in their words: "your One App", or a workplace's, "the PTC App".
    context["config"]["yours"] = crate::config::YOURS.into();
    // The domain whose people join by signing in with Google, for Users to say so.
    if crate::config::SIGN_IN == "google" {
        context["config"]["join_domain"] = crate::config::JOIN_DOMAIN.into();
    }
    // `</` can't appear inside the inline script, so it can't end it early.
    let context = context.to_string().replace("</", "<\\/");
    let (policy, script, body) = if let Some(nonce) = nonce {
        let policy = format!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"script-src 'nonce-{nonce}'; object-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'\">"
        );
        (policy, format!("<script nonce=\"{nonce}\">"), body.replace("<script>", &format!("<script nonce=\"{nonce}\">")))
    } else {
        (String::new(), "<script>".into(), body.to_string())
    };
    let head = if linked {
        let tag = &script[..script.len() - 1];
        let (kit, scripts) = KIT.split_first().unwrap();
        let scripts: String = scripts.iter().map(|(path, text)| format!("{tag} src=\"{}\"></script>", named(path, text))).collect();
        format!("<link rel=\"stylesheet\" href=\"{}\">{script}window.__one={context};</script>{scripts}", named(kit.0, kit.1))
    } else {
        let bridge: String = KIT[1..].iter().map(|(_, text)| *text).collect();
        format!("<style>{}</style>{script}window.__one={context};{bridge}</script>", KIT[0].1)
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">{policy}<meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\">\
         {head}</head><body>{body}</body></html>"
    )
}

/// A kit file's address, named by its content as the gateway names the files it serves (src/kernel/services.rs).
fn named(path: &str, text: &str) -> String {
    use sha2::Digest;
    format!("/screen/{path}?v={}", &data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(text.as_bytes()))[..16])
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
    ("labs", include_str!("screen/settings/labs.html")),
    ("devices", include_str!("screen/settings/devices.html")),
    ("connect-ai", include_str!("screen/settings/connect-ai.html")),
    ("developer", include_str!("screen/settings/developer.html")),
];

/// A client's own screen's HTML, or a note that it isn't there.
pub fn client_screen(name: &str) -> &'static str {
    CLIENT_SCREENS.iter().find(|(n, _)| *n == name).map_or("<div id=\"missing\"></div><script>missing.innerHTML = one.notFound()</script>", |(_, html)| html)
}

/// A client's own screen as its page, as each client makes it from the screens built into it, never a node's: one a
/// node served could act as the person. Native apps make it with this (client/ffi); the web client fills in its
/// address's `screens/<name>.html` (`client_template`).
pub fn client_page(name: &str, params: &serde_json::Value) -> String {
    let nonce = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>());
    page_with(&client_context(name, params), client_screen(name), Some(nonce), false)
}

/// What a client's own screen is told: its name, its params, and the product's.
fn client_context(name: &str, params: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "screen": name, "params": params, "config": { "name": crate::config::NAME, "cli": crate::config::CLI, "made_here": crate::config::MADE_HERE } })
}

/// Each client's own screen's name, and `missing`, what a name that's none of them shows.
pub fn client_screens() -> impl Iterator<Item = &'static str> {
    CLIENT_SCREENS.iter().map(|(name, _)| *name).chain(["missing"])
}

/// What the web client fills in a template: its page's own nonce, and its params' JSON.
pub const TEMPLATE_NONCE: &str = "__nonce__";
pub const TEMPLATE_PARAMS: &str = "\"__params__\"";

/// A client's own screen's page for the web client, which its address serves as `screens/<name>.html` (src/kernel/
/// services.rs): `client_page`, but for `TEMPLATE_NONCE` and `TEMPLATE_PARAMS`, which the web client fills in
/// (kernel/web/web.js).
pub fn client_template(name: &str) -> String {
    page_with(&client_context(name, &TEMPLATE_PARAMS.trim_matches('"').into()), client_screen(name), Some(TEMPLATE_NONCE.into()), false)
}
