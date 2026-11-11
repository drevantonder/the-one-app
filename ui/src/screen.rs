//! A screen's page, an app's as every node serves it, or a client's own as every client makes it: `@one/ui` and the bridge (ui/kit, ui/bridge.js), what the
//! screen is told (`window.__one`), and the screen's own HTML.
//!
//! The clients' own screens (the user's pages, a page's Accounts, a request for a Vault item, Settings) are drawn
//! here once, the same on every client, and act through the bridge's `one.api` as the user looking: each client makes
//! them from what it ships and never takes them from a node (the web client's, which is the native app's window too,
//! from its address's `screens/`), and answers `one.api` only for a screen it chose to show as its own. The page that
//! holds them answers the bridge for whatever a user touches: menus, alerts, sheets, files, scanning and links.

use std::borrow::Cow;

use crate::ui_folder;

/// Phosphor's icons, each name's path on a 256-point grid (ui/vendor/phosphor), for the web pages that draw screens: the
/// web client's, at the address, and a chat's view, by `screens_icons`.
pub const ICONS: &[u8] = include_bytes!("../vendor/phosphor/icons.json");

/// The bridge every screen has, which gives it `one`.
pub fn bridge() -> Cow<'static, str> {
    ui_folder::text("ui/bridge.js", include_str!("../bridge.js"))
}

/// What a web page that holds screens does for them: the web client's, and a chat's view.
pub fn host() -> Cow<'static, str> {
    ui_folder::text("ui/host.js", include_str!("../host.js"))
}

/// What a screen that isn't there shows (a body of no text: an old or mistyped link): a grey line and the way home.
const MISSING: &str = r#"<meta name="@one/ui">
<div id="root"></div>
<script type="module">
import { createElement as h } from "react";
import { createRoot } from "react-dom/client";
import { Empty } from "@one/ui";
createRoot(document.getElementById("root")).render(h(Empty, { action: { label: "Go to Home", run: () => one.close() } }, "This page isn’t here. The link may be old or mistyped."));
</script>"#;

/// `@one/ui` as `bun run build` in ui/kit made it, whose sources `cargo test` checks it was made from.
pub fn ui_css() -> Cow<'static, str> {
    ui_folder::text("ui/kit/dist/ui.css", include_str!("../kit/dist/ui.css"))
}

pub fn ui_js() -> Cow<'static, str> {
    ui_folder::text("ui/kit/dist/ui.js", include_str!("../kit/dist/ui.js"))
}

fn ui_imports() -> Cow<'static, str> {
    ui_folder::text("ui/kit/dist/importmap.json", include_str!("../kit/dist/importmap.json"))
}

/// An app's screen's page: `context` is `{ app, screen, params, me, scope, config }`, `body` its HTML. `linked`, `@one/ui`
/// and the bridge are links to the address's files (`/screen/<path>?v=`, named by content as the gateway names them),
/// which the web client already has, rather than in the page.
pub fn page(context: &serde_json::Value, body: &str, linked: bool) -> String {
    page_with(context, body, None, linked)
}

/// A screen's page, with a client's own screen's `nonce`, or none for an app's. A client's own acts as the user, and
/// draws what apps say, so it runs only its own scripts: they carry the nonce, and its policy refuses every other,
/// inline event handlers too.
fn page_with(context: &serde_json::Value, body: &str, nonce: Option<String>, linked: bool) -> String {
    let body = if body.is_empty() { MISSING } else { body };
    let mut context = context.clone();
    // What a user joins, in their words: "your One App network", or a workplace's, "the PTC App network".
    context["config"]["yours"] = custom_build::config::YOURS.into();
    // The domain whose users join by signing in with a provider's account, for Users to say so.
    if !custom_build::config::JOIN_DOMAIN.is_empty() {
        context["config"]["join_domain"] = custom_build::config::JOIN_DOMAIN.into();
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
    let ui = body.contains(crate::UI);
    let (ui_css, ui_js, ui_imports, bridge) = (ui_css(), ui_js(), ui_imports(), bridge());
    let tag = &script[..script.len() - 1];
    let head = if linked {
        format!("<link rel=\"stylesheet\" href=\"{}\">{script}window.__one={context};</script>{tag} src=\"{}\"></script>", named("ui/ui.css", &ui_css), named("bridge.js", &bridge))
    } else {
        format!("<style>{ui_css}</style>{script}window.__one={context};{bridge}</script>")
    };
    // The import map before the screen's own modules, and ui.js, which runs before them, as they're modules.
    let head = match (ui, linked) {
        (false, _) => head,
        (true, true) => format!("{head}{tag} type=\"importmap\">{ui_imports}</script>{tag} src=\"{}\"></script>", named("ui/ui.js", &ui_js)),
        (true, false) => format!("{head}{tag} type=\"importmap\">{ui_imports}</script>{script}{ui_js}</script>"),
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">{policy}<meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\">\
         {head}</head><body>{body}</body></html>"
    )
}

/// A file's address, named by its content as the gateway names the files it serves (oned/cloudflare/src/workers.rs).
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

/// The source map of a client's own screen: what a frame thrown in one of them came from (`insights::source_map`).
pub fn client_map(name: &str) -> Option<&'static str> {
    crate::carried::CLIENT_MAPS.iter().find(|(n, _)| *n == name).map(|(_, map)| *map)
}

/// A client's own screen's HTML, as @one/ui's build made it (ui/build.rs finds them: the user's pages, a page's Accounts,
/// a request for a Vault item, and Settings'), or none for a name that's none of them, which a page shows as not there (`MISSING`).
pub fn client_screen(name: &str) -> Cow<'static, str> {
    crate::carried::CLIENT_SCREENS.iter().find(|(n, ..)| *n == name).map_or(Cow::Borrowed(""), |(_, path, html)| ui_folder::text(path, html))
}

fn client_context(name: &str, params: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "screen": name, "params": params, "config": { "name": custom_build::config::NAME, "cli": custom_build::config::CLI, "made_here": custom_build::config::MADE_HERE } })
}

pub fn client_screens() -> impl Iterator<Item = &'static str> {
    crate::carried::CLIENT_SCREENS.iter().map(|(name, ..)| *name).chain(["missing"])
}

/// What the web client fills in a template: its page's own nonce, and its params' JSON.
pub const TEMPLATE_NONCE: &str = "__nonce__";
pub const TEMPLATE_PARAMS: &str = "\"__params__\"";

/// A client's own screen's page for the web client, which its address serves as `screens/<name>.html` (src/cloudflare/
/// services.rs), with `TEMPLATE_NONCE` and `TEMPLATE_PARAMS` for the web client to fill in, and linking the bridge and `@one/ui`, which it writes in from its own (ui/web/screens.js).
pub fn client_template(name: &str) -> String {
    page_with(&client_context(name, &TEMPLATE_PARAMS.trim_matches('"').into()), &client_screen(name), Some(TEMPLATE_NONCE.into()), true)
}

#[cfg(test)]
mod tests {
    use sha2::Digest;

    /// @one/ui's built files, and the client's own screens written in React, are made from their sources as they are
    /// (ui/kit/build.js hashes them the same way), so a change to either reaches devices only built: run `bun run build`
    /// in ui/kit.
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
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let kit = repo.join("ui/kit");
        let mut files = Vec::new();
        walk(&kit, &kit, &mut files);
        // Each folder of the client's own screens, all of it but its dist/, since a screen may import what's beside it.
        for folder in crate::ui_files::screen_folders(&repo) {
            for entry in std::fs::read_dir(&folder).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".jsx") || name.ends_with(".tsx") {
                    assert!(super::client_screen(name.rsplit_once('.').unwrap().0).starts_with(crate::UI), "{name} isn't built in as its screen");
                }
            }
            let from = folder.strip_prefix(&repo).unwrap().to_string_lossy().replace('\\', "/");
            let mut own = Vec::new();
            walk(&folder, &folder, &mut own);
            files.extend(own.into_iter().map(|f| format!("../../{from}/{f}")));
        }
        // And onboarding's screens, which auth.json is drawn from and Welcome is @one/ui's.
        for entry in std::fs::read_dir(repo.join("onboarding")).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".tsx") || name.ends_with(".ts") || name.ends_with(".css") {
                files.push(format!("../../onboarding/{name}"));
            }
        }
        files.sort();
        let mut hash = sha2::Sha256::new();
        for file in &files {
            hash.update(file.as_bytes());
            hash.update([0]);
            hash.update(std::fs::read(kit.join(file)).unwrap());
            hash.update([0]);
        }
        let built = std::fs::read_to_string(kit.join("dist/sources.txt")).unwrap();
        assert_eq!(data_encoding::HEXLOWER.encode(&hash.finalize()), built.trim(), "ui/kit or a screen in React changed since it was built: run `bun run build` in ui/kit");
        // ui.js goes into pages as it is, inside a script.
        assert!(!super::ui_js().to_ascii_lowercase().contains("</script"), "ui.js can't be put in a page's script");
    }

    /// A screen that opts in gets React and `@one/ui`'s components, and its import map; a plain one gets only the
    /// theme, as every page does.
    #[test]
    fn ui_only_for_screens_that_opt_in() {
        let context = serde_json::json!({ "app": "a", "screen": "main" });
        let theme = |page: &str, linked: bool| if linked { page.contains("/screen/ui/ui.css?v=") } else { page.contains(&*super::ui_css()) };
        for linked in [false, true] {
            let plain = super::page(&context, "<p>hi</p>", linked);
            assert!(theme(&plain, linked) && !plain.contains("importmap") && !plain.contains("OneUI"));
            let new = super::page(&context, &format!("{}<div id=root></div>", crate::UI), linked);
            assert!(theme(&new, linked) && new.contains(r#"<script type="importmap">{"imports":{"react":"#));
            assert!(new.find("importmap").unwrap() < new.find("<div id=root>").unwrap());
        }
    }
}
