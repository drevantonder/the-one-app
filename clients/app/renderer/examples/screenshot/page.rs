//! A page built by `bun app.js` and what answers its client: its files, its tables and its tools' answers.

use renderer::js::{Clock, Files, Thread};
use renderer::Screen;
use serde_json::{Value, json};
use std::path::PathBuf;
use url::Url;

/// The page `page` (`<app>/client/<page>.html`) of the app built beside the page in `folder`, started as a frame's
/// page, its tools answered from `answers`.
pub(crate) fn framed(folder: &std::path::Path, page: &str, ui: Option<PathBuf>, clock: Clock, touch: bool, answers: std::rc::Rc<std::cell::RefCell<Value>>) -> anyhow::Result<Screen> {
    let at = folder.join(page);
    let (module, files, tokens_page) = built_page(folder, &at, ui)?;
    let js = Thread::start(&module, files, None, clock, touch)?;
    let asked: renderer::Asked = Box::new(move |message, _| {
        let id = message.get("id")?.clone();
        let (method, params) = (message["method"].as_str().unwrap_or_default(), &message["params"]);
        let answers = answers.borrow();
        Some(answered(&answers, id, method, params))
    });
    let mut screen = Screen::new(js, Some(asked));
    screen.page = tokens_page;
    Ok(screen)
}

/// The page `bun app.js` built in `folder`: its module, what reads its files (at `file:///`, its own folder, with the UI
/// library's under /ui/client/, from `ui`), and its own table of tokens (`Screen::page`), loaded with the UI library's.
pub(crate) fn built(folder: &std::path::Path, ui: Option<PathBuf>) -> anyhow::Result<(Url, Files, u32)> {
    let folder = std::fs::canonicalize(folder)?;
    built_page(&folder, &folder.join("index.html"), ui)
}

/// The page at `page` of what's built in `folder`, which is where the page's addresses start from: its module, its
/// files, and its own styles' tokens, its tables loaded.
/// Where a page's file is here: the UI library's in `ui`, the rest in its built folder.
pub(crate) fn local(folder: &std::path::Path, ui: Option<&std::path::Path>, url: &Url) -> PathBuf {
    match (url.path().strip_prefix("/ui/client/"), ui) {
        (Some(file), Some(ui)) => ui.join(file),
        _ => folder.join(url.path().trim_start_matches('/')),
    }
}

pub(crate) fn built_page(folder: &std::path::Path, page: &std::path::Path, ui: Option<PathBuf>) -> anyhow::Result<(Url, Files, u32)> {
    let root = folder.to_owned();
    let mut files: Files = Box::new(move |url: &Url| {
        let file = local(&root, ui.as_deref(), url);
        std::fs::read_to_string(&file).map_err(|error| anyhow::anyhow!("{}: {error}", file.display()))
    });
    let html = std::fs::read_to_string(page)?;
    let module = html
        .split("<script")
        .find_map(|tag| tag.contains("type=\"module\"").then(|| tag.split("src=\"").nth(1)?.split('"').next()).flatten())
        .ok_or_else(|| anyhow::anyhow!("{} loads no module", page.display()))?;
    let at = Url::parse("file:///")?.join(&page.strip_prefix(folder)?.to_string_lossy())?;
    let module = at.join(module)?;
    // The tables: the UI library's tokens and styles, and the page's own styles, where it has them.
    let mut tokens_page = 0;
    let own = at.join("styles.json")?;
    for table in [Url::parse("file:///ui/client/tokens.json")?, Url::parse("file:///ui/client/styles.json")?, own.clone()] {
        let Ok(text) = files(&table) else { continue };
        if table.path().ends_with("tokens.json") {
            renderer::tokens::load_table(&text)?;
        } else {
            renderer::load_styles(&text)?;
            if table == own {
                tokens_page = renderer::tokens::load_page(&text)?;
            }
        }
    }
    Ok((module, files, tokens_page))
}

/// What a call is, as `asks` counts it and `answered` looks it up: `<tool> for <subject>` for someone, else the tool.
pub(crate) fn asked_of(params: &Value) -> Option<String> {
    Some(format!("{} for {}", params["name"].as_str()?, params["_meta"]["subject"].as_str()?))
}

/// What a page's request is answered with from `answers` (`--answers`, as core/ui's scenes.js has them): a tool's answer
/// as its text, `{ "$error": "…" }` as the tool's refusal, and `{ "$image": "data:image/png;base64,…" }` as a picture,
/// as a server answers one (MCP's `image`: Computer Use's screenshot); any other method by its result, or not at all.
pub(crate) fn answered(answers: &Value, id: Value, method: &str, params: &Value) -> Value {
    let picture = |src: &str| src.strip_prefix("data:").and_then(|data| data.split_once(";base64,")).map(|(kind, data)| json!({"type": "image", "mimeType": kind, "data": data}));
    match (method, params["name"].as_str()) {
        ("tools/call", Some(tool)) => match asked_of(params).and_then(|asked| answers.get(&asked)).or_else(|| answers.get(tool)) {
            Some(answer) if answer.get("$error").is_some() => json!({"jsonrpc": "2.0", "id": id, "result": {"isError": true, "content": [{"type": "text", "text": answer["$error"].as_str().unwrap_or_default()}]}}),
            Some(answer) if answer.get("$image").is_some() => json!({"jsonrpc": "2.0", "id": id, "result": {"content": [answer["$image"].as_str().and_then(picture)]}}),
            Some(answer) => json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": answer.to_string()}]}}),
            None => json!({"jsonrpc": "2.0", "id": id, "result": {"isError": true, "content": [{"type": "text", "text": format!("No answer for {tool}")}]}}),
        },
        _ => match answers.get(method) {
            Some(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            None => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("{method} isn't answered here")}}),
        },
    }
}
