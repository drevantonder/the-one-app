//! A strict page drawn offscreen, the same on every run: a PNG of it and its semantic tree as JSON.
//!
//!     cargo run -p strict --features headless --example screenshot -- <page's built folder> <out.png> <out.json> \
//!       [--ui core/ui/dist/client] [--answers answers.json] [--size 480x360] [--scale 2] [--dark] [--click <id>]...
//!
//! It runs the module the folder's `index.html` loads, as `bun app.js` built it, with the UI library a native host
//! loads from `--ui` (`bun run build` in core/ui), and answers what the page asks its client from `--answers`, as
//! core/ui's `record.js` does: `{ "<tool>": <its answer>, "<method>": <its result> }`. It draws with GPUI's own headless
//! context: its test platform, Allo alone as the typeface, the sandbox's fixed clock and randomness, and wgpu's
//! offscreen renderer on whatever adapter wgpu finds (`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json` picks
//! Mesa's software Vulkan, lavapipe, where there's no GPU). `--click` clicks a node first, by its id in the tree.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, WindowAppearance, px, size};
use serde_json::{Value, json};
use strict::Screen;
use strict::js::{Clock, Files, Sandbox};
use url::Url;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [folder, png, tree, ..] = &args[..] else {
        anyhow::bail!("screenshot <folder> <out.png> <out.json> [--ui <dir>] [--answers <json>] [--size WxH] [--scale N] [--dark] [--click id]...")
    };
    let flag = |name: &str| args.iter().position(|arg| arg == name).and_then(|at| args.get(at + 1)).cloned();
    let (width, height) = flag("--size")
        .and_then(|size| size.split_once('x').map(|(w, h)| (w.parse().unwrap_or(480.0), h.parse().unwrap_or(360.0))))
        .unwrap_or((480.0, 360.0));
    let scale: f32 = flag("--scale").and_then(|scale| scale.parse().ok()).unwrap_or(2.0);
    let dark = args.iter().any(|arg| arg == "--dark");
    let clicks: Vec<u64> = args.windows(2).filter(|pair| pair[0] == "--click").filter_map(|pair| pair[1].parse().ok()).collect();
    let answers: Value = match flag("--answers") {
        Some(answers) => serde_json::from_str(&std::fs::read_to_string(answers)?)?,
        None => json!({}),
    };

    // The page's files at `file:///`, as its own folder, with the UI library's under /ui/client/.
    let folder = std::fs::canonicalize(folder)?;
    let ui = flag("--ui").map(PathBuf::from);
    let root = folder.clone();
    let mut files: Files = Box::new(move |url: &Url| {
        let path = url.path();
        let file = match (path.strip_prefix("/ui/client/"), &ui) {
            (Some(file), Some(ui)) => ui.join(file),
            _ => root.join(path.trim_start_matches('/')),
        };
        std::fs::read_to_string(&file).map_err(|error| anyhow::anyhow!("{}: {error}", file.display()))
    });
    let html = std::fs::read_to_string(folder.join("index.html"))?;
    let module = html
        .split("<script")
        .find_map(|tag| tag.contains("type=\"module\"").then(|| tag.split("src=\"").nth(1)?.split('"').next()).flatten())
        .ok_or_else(|| anyhow::anyhow!("index.html loads no module"))?;
    let page = Url::parse("file:///index.html")?.join(module)?;
    // The design tokens, which come with the UI library.
    strict::tokens::load(&files(&Url::parse("file:///ui/client/ui.css")?)?);

    let started = std::time::Instant::now();
    let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(strict::FONT));
    let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(()), gpui_platform::current_headless_renderer);
    strict::add_fonts(cx.text_system())?;
    let sandbox = Sandbox::start(&page, files, None, Clock::fixed())?;
    let ran = started.elapsed();
    // What the page asks its client, answered from `answers`, as record.js answers it.
    let asked: strict::Asked = Box::new(move |message, _, _| {
        let id = message.get("id")?.clone();
        let (method, params) = (message["method"].as_str().unwrap_or_default(), &message["params"]);
        Some(match (method, params["name"].as_str()) {
            ("tools/call", Some(tool)) => match answers.get(tool) {
                Some(answer) => json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": answer.to_string()}]}}),
                None => json!({"jsonrpc": "2.0", "id": id, "result": {"isError": true, "content": [{"type": "text", "text": format!("No answer for {tool}")}]}}),
            },
            _ => match answers.get(method) {
                Some(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                None => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("{method} isn't answered here")}}),
            },
        })
    });
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        cx.new(|cx| {
            let mut screen = Screen::new(sandbox, Some(asked), window, cx);
            screen.theme = Some(if dark { WindowAppearance::Dark } else { WindowAppearance::Light });
            screen
        })
    })?;
    cx.simulate_window_scale_factor_change(window.into(), scale);
    cx.run_until_parked();
    for id in clicks {
        cx.update_window(window.into(), |_, window, cx| {
            if let Some(Some(screen)) = window.root::<Screen>() {
                screen.update(cx, |screen, cx| screen.event(id, "click", None, window, cx));
            }
        })?;
        cx.run_until_parked();
    }
    cx.update_window(window.into(), |_, window, cx| drop(window.draw(cx)))?;
    let drawn = started.elapsed();
    let image = cx.capture_screenshot(window.into())?;
    image.save(png)?;
    let (semantics, failed) = cx.update_window(window.into(), |_, window, cx| {
        let screen = window.root::<Screen>().flatten().map(|screen| screen.read(cx));
        (screen.map(Screen::semantics), screen.and_then(|screen| screen.failed.clone()))
    })?;
    std::fs::write(tree, serde_json::to_string_pretty(&semantics)?)?;
    if let Some(failed) = failed {
        eprintln!("the page failed: {failed}");
    }
    eprintln!("ran its JS in {ran:?}, first frame in {drawn:?}, {}x{} px", image.width(), image.height());
    Ok(())
}
