//! A screen's page, an app's as every node serves it, or a client's own as every client makes it: `@one/ui` and the bridge (`src/screen/`), what the
//! screen is told (`window.__one`), and the screen's own HTML.
//!
//! The clients' own screens (the person's pages, a page's Accounts, a request for a Vault item, Settings) are drawn
//! here once, the same on every client, and act through the bridge's `one.api` as the person looking: each client makes
//! them from what it ships and never takes them from a node (the web client's, which is the native app's window too,
//! from its address's `screens/`), and answers `one.api` only for a screen it chose to show as its own. The page that
//! holds them answers the bridge for whatever a person touches: menus, alerts, sheets, files, scanning and links.

/// Phosphor's icons, each name's path on a 256-point grid (vendor/phosphor), for the web pages that draw screens: the
/// web client's, at the address, and a chat's view, by `screens_icons`.
pub const ICONS: &[u8] = include_bytes!("../vendor/phosphor/icons.json");

const BRIDGE: (&str, &str) = ("bridge.js", include_str!("screen/bridge.js"));

/// What a screen's HTML has in it to use `@one/ui` (`src/screen/ui/`: React, and shadcn's components): its page then
/// has an import map giving `react`, `react-dom/client`, `@one/ui` and the rest (`UI_IMPORTS`), and ui.js, which they
/// come from. Every page has ui.css, `@one/ui`'s theme (shadcn's neutral, light and dark) and what a page starts with,
/// so a plain screen, of HTML and JS alone, looks as the rest do.
pub const UI: &str = r#"<meta name="@one/ui">"#;

/// What a screen that isn't there shows (a body of no text: an old or mistyped link): a grey line and the way home.
const MISSING: &str = r#"<meta name="@one/ui">
<div id="root"></div>
<script type="module">
import { createElement as h } from "react";
import { createRoot } from "react-dom/client";
import { Empty } from "@one/ui";
createRoot(document.getElementById("root")).render(h(Empty, { action: { label: "Go to Home", run: () => one.open("main") } }, "This page isn't here. The link may be old or mistyped."));
</script>"#;

/// `@one/ui` as `bun run build` in `src/screen/ui/` made it, whose sources `cargo test` checks it was made from.
const UI_CSS: (&str, &str) = ("ui/ui.css", include_str!("screen/ui/dist/ui.css"));
const UI_JS: (&str, &str) = ("ui/ui.js", include_str!("screen/ui/dist/ui.js"));
const UI_IMPORTS: &str = include_str!("screen/ui/dist/importmap.json");

/// A screen's page: `context` is `{ app, screen, params, me, scope, config }` (a Settings screen has no app or scope), `body` its HTML.
/// A client's own (`own`) acts as the person, and draws what apps say, so it runs only its own scripts: they carry this
/// page's nonce, and its policy refuses every other, inline event handlers too. `linked`, `@one/ui` and the bridge are links
/// to the address's files (`/screen/<path>?v=`, named by content as the gateway names them), which the web client
/// already has, rather than in the page.
pub fn page(context: &serde_json::Value, body: &str, own: bool, linked: bool) -> String {
    page_with(context, body, own.then(|| data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>())), linked)
}

/// `page`, with a client's own screen's `nonce`, or none for an app's.
fn page_with(context: &serde_json::Value, body: &str, nonce: Option<String>, linked: bool) -> String {
    let body = if body.is_empty() { MISSING } else { body };
    let mut context = context.clone();
    // What a person joins, in their words: "your One App network", or a workplace's, "the PTC App network".
    context["config"]["yours"] = crate::config::YOURS.into();
    // The domain whose people join by signing in with Google, for Users to say so.
    if crate::config::SIGN_IN == crate::config::SignIn::Google {
        context["config"]["join_domain"] = crate::config::JOIN_DOMAIN.into();
    }
    // `</` can't appear inside the inline script, so it can't end it early.
    let context = context.to_string().replace("</", "<\\/");
    let (policy, script, body) = if let Some(nonce) = nonce {
        let policy = format!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"script-src 'nonce-{nonce}'; object-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'\">"
        );
        // Every script of its own, modules too (an `@one/ui` screen's), whose imports then carry the nonce.
        let body = body.replace("<script ", &format!("<script nonce=\"{nonce}\" ")).replace("<script>", &format!("<script nonce=\"{nonce}\">"));
        (policy, format!("<script nonce=\"{nonce}\">"), body)
    } else {
        (String::new(), "<script>".into(), body.to_string())
    };
    let ui = body.contains(UI);
    let tag = &script[..script.len() - 1];
    let head = if linked {
        format!("<link rel=\"stylesheet\" href=\"{}\">{script}window.__one={context};</script>{tag} src=\"{}\"></script>", named(UI_CSS.0, UI_CSS.1), named(BRIDGE.0, BRIDGE.1))
    } else {
        format!("<style>{}</style>{script}window.__one={context};{}</script>", UI_CSS.1, BRIDGE.1)
    };
    // The import map before the screen's own modules, and ui.js, which runs before them, as they're modules.
    let head = match (ui, linked) {
        (false, _) => head,
        (true, true) => format!("{head}{tag} type=\"importmap\">{UI_IMPORTS}</script>{tag} src=\"{}\"></script>", named(UI_JS.0, UI_JS.1)),
        (true, false) => format!("{head}{tag} type=\"importmap\">{UI_IMPORTS}</script>{script}{}</script>", UI_JS.1),
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">{policy}<meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\">\
         {head}</head><body>{body}</body></html>"
    )
}

/// A file's address, named by its content as the gateway names the files it serves (src/cloudflare/workers.rs).
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

/// The clients' own screens, by name, the same on every client: the person's pages, a page's Accounts, a request for a
/// Vault item, Settings' (`settings/`), and Developer's panel (`devtools`). They act through the bridge's `one.api` as the person looking, which a
/// client answers only for these.
const CLIENT_SCREENS: &[(&str, &str)] = &[
    // Written in React (home/<name>.jsx), built by @one/ui's build into home/dist/.
    ("pages", include_str!("screen/home/dist/pages.html")),
    ("accounts", include_str!("screen/home/dist/accounts.html")),
    ("panel", include_str!("screen/home/dist/panel.html")),
    ("request", include_str!("screen/settings/dist/request.html")),
    ("appearance", include_str!("screen/settings/dist/appearance.html")),
    // Written in React (settings/<name>.jsx), built by @one/ui's build into settings/dist/.
    ("vault", include_str!("screen/settings/dist/vault.html")),
    ("requests", include_str!("screen/settings/dist/requests.html")),
    ("groups", include_str!("screen/settings/dist/groups.html")),
    ("users", include_str!("screen/settings/dist/users.html")),
    ("backups", include_str!("screen/settings/dist/backups.html")),
    ("labs", include_str!("screen/settings/dist/labs.html")),
    ("devices", include_str!("screen/settings/dist/devices.html")),
    ("connect-ai", include_str!("screen/settings/dist/connect-ai.html")),
    ("developer", include_str!("screen/settings/dist/developer.html")),
    ("devtools", include_str!("screen/settings/dist/devtools.html")),
];

/// A client's own screen's HTML, or none for a name that's none of them, which a page shows as not there (`MISSING`).
pub fn client_screen(name: &str) -> &'static str {
    CLIENT_SCREENS.iter().find(|(n, _)| *n == name).map_or("", |(_, html)| html)
}

fn client_context(name: &str, params: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "screen": name, "params": params, "config": { "name": crate::config::NAME, "cli": crate::config::CLI, "made_here": crate::config::MADE_HERE } })
}

pub fn client_screens() -> impl Iterator<Item = &'static str> {
    CLIENT_SCREENS.iter().map(|(name, _)| *name).chain(["missing"])
}

/// What the web client fills in a template: its page's own nonce, and its params' JSON.
pub const TEMPLATE_NONCE: &str = "__nonce__";
pub const TEMPLATE_PARAMS: &str = "\"__params__\"";

/// A client's own screen's page for the web client, which its address serves as `screens/<name>.html` (src/cloudflare/
/// services.rs), with `TEMPLATE_NONCE` and `TEMPLATE_PARAMS` for the web client to fill in, and linking the bridge and `@one/ui`, which it writes in from its own (cloudflare/web/web.js).
pub fn client_template(name: &str) -> String {
    page_with(&client_context(name, &TEMPLATE_PARAMS.trim_matches('"').into()), client_screen(name), Some(TEMPLATE_NONCE.into()), true)
}

#[cfg(test)]
mod tests {
    use sha2::Digest;

    /// @one/ui's built files, and the Settings screens written in React, are made from their sources as they are
    /// (src/screen/ui/build.js hashes them the same way), so a change to either reaches devices only built: run
    /// `bun run build` in src/screen/ui/.
    #[test]
    fn ui_is_built_from_its_sources() {
        fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name == "node_modules" || name == "dist" {
                    continue;
                }
                let path = entry.path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    out.push(path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/screen/ui");
        let mut files = Vec::new();
        walk(&root, &root, &mut files);
        // The Settings screens written in React, which the same build makes, and the pages' and Accounts' (home/, all
        // of it but its dist/, since they share its lib/).
        for folder in ["settings", "home"] {
            for entry in std::fs::read_dir(root.join("..").join(folder)).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".jsx") || name.ends_with(".tsx") {
                    assert!(super::client_screen(name.rsplit_once('.').unwrap().0).starts_with(super::UI), "{name} isn't built in as its screen");
                    if folder == "settings" {
                        files.push(format!("../settings/{name}"));
                    }
                } else if folder == "settings" && name.ends_with(".js") {
                    files.push(format!("../settings/{name}"));
                }
            }
        }
        let home = root.join("../home");
        let mut shared = Vec::new();
        walk(&home, &home, &mut shared);
        files.extend(shared.into_iter().map(|f| format!("../home/{f}")));
        // And the sign-in pages, whose class names sign-in.css is made of.
        files.push("../../../cloudflare/sign-in/pages.js".into());
        files.sort();
        let mut hash = sha2::Sha256::new();
        for file in &files {
            hash.update(file.as_bytes());
            hash.update([0]);
            hash.update(std::fs::read(root.join(file)).unwrap());
            hash.update([0]);
        }
        let built = std::fs::read_to_string(root.join("dist/sources.txt")).unwrap();
        assert_eq!(data_encoding::HEXLOWER.encode(&hash.finalize()), built.trim(), "src/screen/ui or a Settings screen in React changed since it was built: run `bun run build` in src/screen/ui");
        // ui.js goes into pages as it is, inside a script.
        assert!(!super::UI_JS.1.to_ascii_lowercase().contains("</script"), "ui.js can't be put in a page's script");
    }

    /// A screen that opts in gets React and `@one/ui`'s components, and its import map; a plain one gets only the
    /// theme, as every page does.
    #[test]
    fn ui_only_for_screens_that_opt_in() {
        let context = serde_json::json!({ "app": "a", "screen": "main" });
        let theme = |page: &str, linked: bool| if linked { page.contains("/screen/ui/ui.css?v=") } else { page.contains(super::UI_CSS.1) };
        for linked in [false, true] {
            let plain = super::page(&context, "<p>hi</p>", false, linked);
            assert!(theme(&plain, linked) && !plain.contains("importmap") && !plain.contains("OneUI"));
            let new = super::page(&context, &format!("{}<div id=root></div>", super::UI), false, linked);
            assert!(theme(&new, linked) && new.contains(r#"<script type="importmap">{"imports":{"react":"#));
            assert!(new.find("importmap").unwrap() < new.find("<div id=root>").unwrap());
        }
    }
}
