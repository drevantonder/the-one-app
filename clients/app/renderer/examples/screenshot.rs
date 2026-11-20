//! A native page drawn offscreen, the same on every run: a PNG of it and its semantic tree as JSON.
//!
//!     cargo run -p renderer --example screenshot -- <page's built folder> <out.png> <out.json> \
//!       [--ui core/ui/dist/client] [--answers answers.json] [--size 480x360] [--scale 2] [--dark] [--requests <file>|-]
//!       [--safe-area <top>,<right>,<bottom>,<left>] [--touch] [--panel <corner>] [--gpu] [--measure]
//!
//! It runs the module the folder's `index.html` loads, as `bun app.js` built it, with the UI library a native host
//! loads from `--ui` (`bun run build` in core/ui), and answers what the page asks its client from `--answers`, as
//! core/ui's `record.js` does: `{ "<tool>": <its answer>, "<method>": <its result> }`. It draws as a window does, with
//! Allo alone as the typeface, the sandbox's fixed clock and randomness, and our renderer drawing offscreen on
//! whatever GPU Vulkan finds (`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json` picks Mesa's software Vulkan,
//! lavapipe, where there's no GPU), or on Windows Direct3D 12's (WARP, Windows' own software one, where there's none). `--requests` gives it the inspector's requests first, one a line
//! (`renderer::inspect`), as a native window takes them on its socket, from a file or stdin (`-`), each answered on
//! stdout, a frame drawn before each: `{"v":1,"id":1,"op":"tap","text":"Backups"}`. Its clock is paused: `clock`'s
//! `step` moves it. `--touch` draws it for a finger, as a phone's window is (`pointer: coarse`). `--panel` draws it as a
//! computer's panel is (a Mac's launcher, its corners `<corner>` round): what its page puts on the system's glass is a
//! window of its own, each drawn by a renderer of its own from where it is (`Display::arrange`), over the system's glass,
//! which is a flat colour here, and its picture and costs are theirs.
//! `--gpu` waits for the GPU after each frame drawn, and says how long that took with its cost (`gpu_ms`): on a real
//! GPU, its time for the frame, as a device that draws it would take.
//! `--measure` says, on stderr after the rest, how quickly it draws, as a line `measured {…}`: its first
//! frame (`first_frame_ms`, from the GPU's start to the frame read back), a frame's time when nothing changes
//! (`idle_ms`), as its first text field takes a character a frame (`typing_ms`) and as its first list scrolls
//! (`scrolling_ms`), each where it has one, the longest the drawing thread spent on a frame of all it drew
//! (`worst_paint_ms`: laying it out and making its scene, which a picture's decoding, in the background, isn't part of), the memory drawing took (`memory_mb`: the GPU, its atlas and the
//! renderer, as the process's resident set grew), and what a second window of its size adds (`window_mb`: a renderer
//! of its own, drawing the same frame, the GPU and the atlas shared).

use std::path::PathBuf;

use serde_json::{Value, json};
use renderer::geometry::{DevicePixels, Pixels, Size};
use renderer::js::{Clock, Files, Thread};
use renderer::render::{atlas::Atlas, context::Context, renderer::Renderer, scene::Scene};
use renderer::text::TextSystem;
use renderer::{Appearance, Screen};
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
    let touch = args.iter().any(|arg| arg == "--touch");
    let gpu_times = args.iter().any(|arg| arg == "--gpu");
    let panel: Option<f32> = flag("--panel").and_then(|corner| corner.parse().ok());
    // What the window keeps for itself at each edge, as a phone's status bar and home indicator, or a title bar.
    let safe: Vec<f32> = flag("--safe-area").map(|edges| edges.split(',').filter_map(|edge| edge.parse().ok()).collect()).unwrap_or_default();
    let insets = match safe[..] {
        [top, right, bottom, left] => renderer::geometry::Edges { top, right, bottom, left },
        _ => Default::default(),
    };
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
    // The tables: the UI library's tokens and styles, and the page's own styles, where it has them.
    let mut tokens_page = 0;
    for table in ["file:///ui/client/tokens.json", "file:///ui/client/styles.json", "file:///styles.json"] {
        let Ok(text) = files(&Url::parse(table)?) else { continue };
        if table.ends_with("tokens.json") {
            renderer::tokens::load_table(&text)?;
        } else {
            renderer::load_styles(&text)?;
            if table == "file:///styles.json" {
                tokens_page = renderer::tokens::load_page(&text)?;
            }
        }
    }

    let started = std::time::Instant::now();
    let mut text = TextSystem::new(renderer::FONT, renderer::fonts()?, false);
    let js = Thread::start(&page, files, None, Clock::fixed(), touch)?;
    // What the page asks its client, answered from `answers`, as record.js answers it.
    let asked: renderer::Asked = Box::new(move |message, _| {
        let id = message.get("id")?.clone();
        let (method, params) = (message["method"].as_str().unwrap_or_default(), &message["params"]);
        Some(match (method, params["name"].as_str()) {
            ("tools/call", Some(tool)) => match answers.get(tool) {
                // `{ "$error": "…" }` is the tool's refusal, as the scenes (core/ui/scenes.js) answer it.
                Some(answer) if answer.get("$error").is_some() => json!({"jsonrpc": "2.0", "id": id, "result": {"isError": true, "content": [{"type": "text", "text": answer["$error"].as_str().unwrap_or_default()}]}}),
                Some(answer) => json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": answer.to_string()}]}}),
                None => json!({"jsonrpc": "2.0", "id": id, "result": {"isError": true, "content": [{"type": "text", "text": format!("No answer for {tool}")}]}}),
            },
            _ => match answers.get(method) {
                Some(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                None => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("{method} isn't answered here")}}),
            },
        })
    });
    let mut screen = Screen::new(js, Some(asked), Size { width, height }, insets);
    screen.page = tokens_page;
    screen.theme = Some(if dark { Appearance::Dark } else { Appearance::Light });
    screen.touch = touch;
    screen.panel = panel.map(|corner| renderer::Panel { corner, fill: false, whole: false });
    // Its JS runs on a thread of its own, as in a window: each step is drawn once what it set off has run.
    screen.idle();
    let ran = started.elapsed();
    if let Some(why) = screen.didnt_start() {
        anyhow::bail!("its page didn't start: {why}");
    }
    let gpu = std::time::Instant::now();
    let resident = || memory_stats::memory_stats().map_or(0, |memory| memory.physical_mem);
    let before = resident();
    let context = Context::new()?;
    let device = Size { width: DevicePixels((width * scale).round() as i32), height: DevicePixels((height * scale).round() as i32) };
    let atlas = std::sync::Arc::new(Atlas::new(&context));
    let mut renderer = Renderer::offscreen(&context, atlas, device);
    let mut scene = Scene::default();
    let size = Size { width: Pixels(width), height: Pixels(height) };
    let first_frame = std::cell::Cell::new(None);
    // A panel's parts, each the size it was made and its renderer.
    let mut parts: Vec<(Size<DevicePixels>, Renderer)> = Vec::new();
    let worst_paint = std::cell::Cell::new(std::time::Duration::ZERO);
    // A frame drawn, and with `read`, waited for and read back: drawn again once the pictures it decodes in the
    // background are ready, as a window is woken to. → its pixels, if read, and what it cost (`Screen::cost`).
    let mut frame = |screen: &mut Screen, read: bool| -> anyhow::Result<(u32, u32, Vec<u8>, Value)> {
        loop {
            screen.idle();
            let painting = std::time::Instant::now();
            screen.paint(size, scale, &mut text, renderer.sprite_atlas(), &mut scene);
            worst_paint.set(worst_paint.get().max(painting.elapsed()));
            if !screen.decoding() {
                break;
            }
        }
        let drawing = std::time::Instant::now();
        // On a panel, each pane of the system's glass its page asks for, its edges on whole points, as a window's are.
        let panes: Vec<(f32, f32, Size<DevicePixels>)> = screen
            .glass()
            .iter()
            .map(|pane| {
                let (left, top) = (pane.bounds.origin.x.round(), pane.bounds.origin.y.round());
                let (right, bottom) = ((pane.bounds.origin.x + pane.bounds.size.width).round(), (pane.bounds.origin.y + pane.bounds.size.height).round());
                let device = |length: f32| DevicePixels((length * scale).round() as i32);
                (left * scale, top * scale, Size { width: device(right - left), height: device(bottom - top) })
            })
            .collect();
        let mut cost = if panes.is_empty() {
            renderer.draw(&scene, [0.0, 0.0]);
            screen.cost(&renderer)
        } else {
            parts.truncate(panes.len());
            for (at, (_, _, size)) in panes.iter().enumerate() {
                if parts.get(at).is_none_or(|(made, _)| made != size) {
                    let made = (*size, Renderer::offscreen(&context, renderer.sprite_atlas().clone(), *size));
                    if at < parts.len() { parts[at] = made } else { parts.push(made) }
                }
            }
            let mut costs = Vec::new();
            for ((left, top, _), (_, part)) in panes.iter().zip(&mut parts) {
                part.draw(&scene, [*left, *top]);
                costs.push(screen.cost(part));
            }
            summed(costs)
        };
        // How long the GPU took, waited for, where it's asked: on a real GPU, that GPU's time for the frame.
        if gpu_times {
            renderer.finish();
            cost["gpu_ms"] = json!((drawing.elapsed().as_secs_f64() * 100_000.0).round() / 100.0);
        }
        // The first frame is read back too, which `first_frame_ms` is until.
        if !read && first_frame.get().is_some() {
            return Ok((0, 0, Vec::new(), cost));
        }
        let (width, height, rgba) = if panes.is_empty() {
            renderer.read().ok_or_else(|| anyhow::anyhow!("nothing was drawn"))?
        } else {
            // The parts over the system's glass, which is the window's grey in its Colour Mode here.
            let (width, height) = (device.width.0 as u32, device.height.0 as u32);
            let glass: [u8; 3] = if dark { [30, 30, 32] } else { [236, 236, 238] };
            let mut rgba: Vec<u8> = (0..width * height).flat_map(|_| [glass[0], glass[1], glass[2], 255]).collect();
            for ((left, top, _), (_, part)) in panes.iter().zip(&mut parts) {
                let (w, h, drawn) = part.read().ok_or_else(|| anyhow::anyhow!("nothing was drawn"))?;
                for (y, x) in (0..h).flat_map(|y| (0..w).map(move |x| (y, x))) {
                    let (to_x, to_y) = (x as i64 + *left as i64, y as i64 + *top as i64);
                    if to_x < 0 || to_y < 0 || to_x >= i64::from(width) || to_y >= i64::from(height) {
                        continue;
                    }
                    let (from, to) = (((y * w + x) * 4) as usize, ((to_y as u32 * width + to_x as u32) * 4) as usize);
                    let alpha = u32::from(drawn[from + 3]);
                    for channel in 0..3 {
                        rgba[to + channel] = ((u32::from(drawn[from + channel]) * alpha + u32::from(rgba[to + channel]) * (255 - alpha)) / 255) as u8;
                    }
                }
            }
            (width, height, rgba)
        };
        first_frame.set(first_frame.get().or(Some(gpu.elapsed())));
        Ok((width, height, rgba, cost))
    };
    for line in requests.lines().filter(|line| !line.trim().is_empty()) {
        let request = renderer::inspect::Request::parse(line);
        // A frame is drawn before each, and read back only for a picture, as a window's is never read.
        let (w, h, rgba, _) = frame(&mut screen, request.as_ref().is_ok_and(|request| request.op == "snap"))?;
        let request = match request {
            Ok(request) => request,
            Err(refused) => {
                println!("{refused}");
                continue;
            }
        };
        if request.op == "snap" {
            println!("{}", renderer::inspect::answer(&request.id, renderer::inspect::png(w, h, rgba)));
            continue;
        }
        // Each of its frames drawn as the window draws one, and read back where it asks for pictures.
        if request.op == "profile" || request.op == "replay" {
            let snaps = renderer::inspect::snaps(&request);
            let mut draw = |screen: &mut Screen| {
                let (w, h, rgba, mut cost) = frame(screen, snaps)?;
                if snaps {
                    for (name, value) in renderer::inspect::png(w, h, rgba)?.as_object().into_iter().flatten() {
                        cost[name] = value.clone();
                    }
                }
                Ok(cost)
            };
            let profiled = if request.op == "replay" { screen.replay(&request, &mut draw) } else { screen.profile(&request, &mut draw) };
            println!("{}", renderer::inspect::answer(&request.id, profiled));
            continue;
        }
        let answered = std::rc::Rc::new(std::cell::RefCell::new(None));
        let to = answered.clone();
        let deadline = request.f64("deadline").unwrap_or(5000.0);
        screen.inspect(request, Box::new(move |answer| *to.borrow_mut() = Some(answer)));
        screen.idle();
        // A `settle` not settled at once is told so at its deadline.
        if answered.borrow().is_none() {
            std::thread::sleep(std::time::Duration::from_secs_f64(deadline / 1000.0));
            screen.fire();
        }
        let answer = answered.borrow_mut().take();
        println!("{}", answer.unwrap_or_else(|| json!({"error": "it didn't answer"})));
    }
    let (w, h, rgba, _) = frame(&mut screen, true)?;
    let drawn = started.elapsed();
    // What it told the page as it drew (where each node is, what's in sight) has run, for its tree.
    screen.idle();
    image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| anyhow::anyhow!("the frame is the wrong size"))?.save(png)?;
    std::fs::write(tree, serde_json::to_string_pretty(&screen.semantics())?)?;
    if let Some(failed) = &screen.failed {
        eprintln!("the page failed: {failed}");
    }
    // A window would open it in a web view instead.
    if let Some(why) = &screen.mismatched {
        eprintln!("it opens in a web view: {why}");
    }
    eprintln!("ran its JS in {ran:?}, first frame in {drawn:?}, {w}x{h} px");
    if args.iter().any(|arg| arg == "--measure") {
        // How long a frame takes, on average over FRAMES each after `act`, which says false where there's nothing
        // to do it to: drawn one after another, as a window draws them, and the last waited for.
        const FRAMES: u32 = 30;
        let mut frames = |screen: &mut Screen, act: &mut dyn FnMut(&mut Screen, u32) -> bool| -> anyhow::Result<Option<f64>> {
            let started = std::time::Instant::now();
            for at in 0..FRAMES {
                if !act(screen, at) {
                    return Ok(None);
                }
                frame(screen, at + 1 == FRAMES)?;
            }
            Ok(Some(started.elapsed().as_secs_f64() * 1000.0 / f64::from(FRAMES)))
        };
        let idle = frames(&mut screen, &mut |_, _| true)?;
        let found = |screen: &mut Screen, role: &str| ask(screen, json!({"op": "query", "role": role}))?["result"]["nodes"][0]["node"].as_u64();
        let field = ["searchbox", "textbox"].into_iter().find_map(|role| found(&mut screen, role));
        let typing = match field {
            Some(node) => frames(&mut screen, &mut |screen, _| ask(screen, json!({"op": "type", "node": node, "text": "a"})).is_some_and(|answer| answer.get("result").is_some()))?,
            None => None,
        };
        let list = found(&mut screen, "list");
        let scrolling = match list {
            Some(node) => frames(&mut screen, &mut |screen, at| {
                let dy = if at < FRAMES / 2 { 40 } else { -40 };
                ask(screen, json!({"op": "scroll", "node": node, "dy": dy})).is_some_and(|answer| answer.get("result").is_some())
            })?,
            None => None,
        };
        let ms = |ms: Option<f64>| ms.map(|ms| (ms * 100.0).round() / 100.0);
        let first_frame_ms = first_frame.get().map(|first| first.as_secs_f64() * 1000.0);
        let worst_paint_ms = worst_paint.get().as_secs_f64() * 1000.0;
        let mb = |bytes: usize| (bytes as f64 / 1e5).round() / 10.0;
        let memory_mb = mb(resident().saturating_sub(before));
        let alone = resident();
        let mut second = Renderer::offscreen(&context, renderer.sprite_atlas().clone(), device);
        second.draw(&scene, [0.0, 0.0]);
        second.read();
        let window_mb = mb(resident().saturating_sub(alone));
        second.destroy();
        eprintln!(
            "measured {}",
            json!({"first_frame_ms": ms(first_frame_ms), "worst_paint_ms": ms(Some(worst_paint_ms)), "idle_ms": ms(idle), "typing_ms": ms(typing), "scrolling_ms": ms(scrolling), "memory_mb": memory_mb, "window_mb": window_mb})
        );
    }
    renderer.destroy();
    Ok(())
}

/// What the screen answers `request`, an inspector's, once what it set off has run.
fn ask(screen: &mut Screen, mut request: Value) -> Option<Value> {
    request["v"] = json!(renderer::inspect::VERSION);
    request["id"] = json!(0);
    let request = renderer::inspect::Request::parse(&request.to_string()).ok()?;
    let answered = std::rc::Rc::new(std::cell::RefCell::new(None));
    let to = answered.clone();
    screen.inspect(request, Box::new(move |answer| *to.borrow_mut() = Some(answer)));
    screen.idle();
    answered.borrow_mut().take()
}

/// The costs of a panel's parts' frames as one frame's: their draw calls and their GPU's work added up.
fn summed(costs: Vec<Value>) -> Value {
    let mut all = costs.first().cloned().unwrap_or(json!({}));
    for cost in costs.iter().skip(1) {
        all["draws"] = json!(all["draws"].as_u64().unwrap_or(0) + cost["draws"].as_u64().unwrap_or(0));
        for (what, value) in cost["work"].as_object().into_iter().flatten() {
            all["work"][what] = json!(all["work"][what].as_f64().unwrap_or(0.0) + value.as_f64().unwrap_or(0.0));
        }
    }
    all
}
