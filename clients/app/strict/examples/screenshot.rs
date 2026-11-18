//! A strict page drawn offscreen, the same on every run: a PNG of it and its semantic tree as JSON.
//!
//!     cargo run -p strict --example screenshot -- <page's built folder> <out.png> <out.json> \
//!       [--ui core/ui/dist/client] [--answers answers.json] [--size 480x360] [--scale 2] [--dark] [--requests <file>|-]
//!
//! It runs the module the folder's `index.html` loads, as `bun app.js` built it, with the UI library a native host
//! loads from `--ui` (`bun run build` in core/ui), and answers what the page asks its client from `--answers`, as
//! core/ui's `record.js` does: `{ "<tool>": <its answer>, "<method>": <its result> }`. It draws as a window does, with
//! Allo alone as the typeface, the sandbox's fixed clock and randomness, and our renderer drawing offscreen on
//! whatever GPU Vulkan finds (`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json` picks Mesa's software Vulkan,
//! lavapipe, where there's no GPU). `--requests` gives it the inspector's requests first, one a line
//! (`strict::inspect`), as a strict window takes them on its socket, from a file or stdin (`-`), each answered on
//! stdout, a frame drawn before each: `{"v":1,"id":1,"op":"tap","text":"Backups"}`. Its clock is paused: `clock`'s
//! `step` moves it.

use std::path::PathBuf;

use serde_json::{Value, json};
use strict::geometry::{DevicePixels, Pixels, Size};
use strict::js::{Clock, Files, Sandbox};
use strict::render::{context::BladeContext, renderer::BladeRenderer, scene::Scene};
use strict::text::{PlatformTextSystem, TextSystem};
use strict::{Appearance, Screen};
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
    let requests = match flag("--requests").as_deref() {
        Some("-") => std::io::read_to_string(std::io::stdin())?,
        Some(file) => std::fs::read_to_string(file)?,
        None => String::new(),
    };
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
    // Version 2's tables: the UI library's tokens and styles, and the page's own styles, where it has them.
    for table in ["file:///ui/client/tokens.json", "file:///ui/client/styles.json", "file:///styles.json"] {
        let Ok(text) = files(&Url::parse(table)?) else { continue };
        if table.ends_with("tokens.json") {
            strict::tokens::load_table(&text)?;
        } else {
            strict::load_styles(&text)?;
        }
    }

    let started = std::time::Instant::now();
    let mut text = TextSystem::new(PlatformTextSystem::new(strict::FONT, strict::fonts()?, false));
    let sandbox = Sandbox::start(&page, files, None, Clock::fixed())?;
    let ran = started.elapsed();
    // What the page asks its client, answered from `answers`, as record.js answers it.
    let asked: strict::Asked = Box::new(move |message, _| {
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
    let mut screen = Screen::new(sandbox, Some(asked), Size { width, height });
    screen.theme = Some(if dark { Appearance::Dark } else { Appearance::Light });
    let context = BladeContext::new(false)?;
    let device = Size { width: DevicePixels((width * scale).round() as i32), height: DevicePixels((height * scale).round() as i32) };
    let atlas = std::sync::Arc::new(strict::render::atlas::BladeAtlas::new(&context));
    let mut renderer = BladeRenderer::offscreen(&context, atlas, device);
    let mut scene = Scene::default();
    let size = Size { width: Pixels(width), height: Pixels(height) };
    let mut draw = |screen: &mut Screen| -> anyhow::Result<(u32, u32, Vec<u8>)> {
        screen.paint(size, scale, &mut text, renderer.sprite_atlas(), &mut scene);
        renderer.draw(&scene);
        renderer.read().ok_or_else(|| anyhow::anyhow!("nothing was drawn"))
    };
    for line in requests.lines().filter(|line| !line.trim().is_empty()) {
        let (w, h, rgba) = draw(&mut screen)?;
        let request = match strict::inspect::Request::parse(line) {
            Ok(request) => request,
            Err(refused) => {
                println!("{refused}");
                continue;
            }
        };
        if request.op == "snap" {
            let mut png = Vec::new();
            image::RgbaImage::from_raw(w, h, rgba).expect("the frame's size").write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
            let snapped = json!({"png": data_encoding::BASE64.encode(&png), "width": w, "height": h});
            println!("{}", strict::inspect::answer(&request.id, Ok(snapped)));
            continue;
        }
        let answered = std::rc::Rc::new(std::cell::RefCell::new(None));
        let to = answered.clone();
        let deadline = request.f64("deadline").unwrap_or(5000.0);
        screen.inspect(request, Box::new(move |answer| *to.borrow_mut() = Some(answer)));
        // A `settle` not settled at once is told so at its deadline.
        if answered.borrow().is_none() {
            std::thread::sleep(std::time::Duration::from_secs_f64(deadline / 1000.0));
            screen.fire();
        }
        let answer = answered.borrow_mut().take();
        println!("{}", answer.unwrap_or_else(|| json!({"error": "it didn't answer"})));
    }
    let (w, h, rgba) = draw(&mut screen)?;
    let drawn = started.elapsed();
    renderer.destroy();
    image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| anyhow::anyhow!("the frame is the wrong size"))?.save(png)?;
    std::fs::write(tree, serde_json::to_string_pretty(&screen.semantics())?)?;
    if let Some(failed) = &screen.failed {
        eprintln!("the page failed: {failed}");
    }
    eprintln!("ran its JS in {ran:?}, first frame in {drawn:?}, {w}x{h} px");
    Ok(())
}
