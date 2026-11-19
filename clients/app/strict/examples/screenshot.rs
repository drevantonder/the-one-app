//! A strict page drawn offscreen, the same on every run: a PNG of it and its semantic tree as JSON.
//!
//!     cargo run -p strict --example screenshot -- <page's built folder> <out.png> <out.json> \
//!       [--ui core/ui/dist/client] [--answers answers.json] [--size 480x360] [--scale 2] [--dark] [--requests <file>|-]
//!       [--safe-area <top>,<right>,<bottom>,<left>] [--touch] [--measure]
//!
//! It runs the module the folder's `index.html` loads, as `bun app.js` built it, with the UI library a native host
//! loads from `--ui` (`bun run build` in core/ui), and answers what the page asks its client from `--answers`, as
//! core/ui's `record.js` does: `{ "<tool>": <its answer>, "<method>": <its result> }`. It draws as a window does, with
//! Allo alone as the typeface, the sandbox's fixed clock and randomness, and our renderer drawing offscreen on
//! whatever GPU Vulkan finds (`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json` picks Mesa's software Vulkan,
//! lavapipe, where there's no GPU), or on Windows Direct3D 11's (WARP, Windows' own software one, where there's none). `--requests` gives it the inspector's requests first, one a line
//! (`strict::inspect`), as a strict window takes them on its socket, from a file or stdin (`-`), each answered on
//! stdout, a frame drawn before each: `{"v":1,"id":1,"op":"tap","text":"Backups"}`. Its clock is paused: `clock`'s
//! `step` moves it. `--touch` draws it for a finger, as a phone's window is (`pointer: coarse`).
//! `--measure` says, on stderr after the rest, how quickly it draws, as a line `measured {…}`: its first
//! frame (`first_frame_ms`, from the GPU's start to the frame read back), a frame's time when nothing changes
//! (`idle_ms`), as its first text field takes a character a frame (`typing_ms`) and as its first list scrolls
//! (`scrolling_ms`), each where it has one, and the memory drawing took (`memory_mb`: the GPU, its atlas and the
//! renderer, as the process's resident set grew).

use std::path::PathBuf;

use serde_json::{Value, json};
use strict::geometry::{DevicePixels, Pixels, Size};
use strict::js::{Clock, Files, Thread};
use strict::render::{Atlas, Context, Renderer, scene::Scene};
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
    let touch = args.iter().any(|arg| arg == "--touch");
    // What the window keeps for itself at each edge, as a phone's status bar and home indicator, or a title bar.
    let safe: Vec<f32> = flag("--safe-area").map(|edges| edges.split(',').filter_map(|edge| edge.parse().ok()).collect()).unwrap_or_default();
    let insets = match safe[..] {
        [top, right, bottom, left] => strict::geometry::Edges { top, right, bottom, left },
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
    // The design tokens, which come with the UI library.
    strict::tokens::load(&files(&Url::parse("file:///ui/client/ui.css")?)?);
    // Version 2's tables: the UI library's tokens and styles, and the page's own styles, where it has them.
    let mut tokens_page = 0;
    for table in ["file:///ui/client/tokens.json", "file:///ui/client/styles.json", "file:///styles.json"] {
        let Ok(text) = files(&Url::parse(table)?) else { continue };
        if table.ends_with("tokens.json") {
            strict::tokens::load_table(&text)?;
        } else {
            strict::load_styles(&text)?;
            if table == "file:///styles.json" {
                tokens_page = strict::tokens::load_page(&text)?;
            }
        }
    }

    let started = std::time::Instant::now();
    let mut text = TextSystem::new(PlatformTextSystem::new(strict::FONT, strict::fonts()?, false));
    let js = Thread::start(&page, files, None, Clock::fixed())?;
    // What the page asks its client, answered from `answers`, as record.js answers it.
    let asked: strict::Asked = Box::new(move |message, _| {
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
    // Its JS runs on a thread of its own, as in a window: each step is drawn once what it set off has run.
    screen.idle();
    let ran = started.elapsed();
    if let Some(why) = screen.didnt_start() {
        anyhow::bail!("its page didn't start: {why}");
    }
    let gpu = std::time::Instant::now();
    let resident = || memory_stats::memory_stats().map_or(0, |memory| memory.physical_mem);
    let before = resident();
    let context = Context::new(false)?;
    let device = Size { width: DevicePixels((width * scale).round() as i32), height: DevicePixels((height * scale).round() as i32) };
    let atlas = std::sync::Arc::new(Atlas::new(&context));
    let mut renderer = Renderer::offscreen(&context, atlas, device);
    let mut scene = Scene::default();
    let size = Size { width: Pixels(width), height: Pixels(height) };
    let first_frame = std::cell::Cell::new(None);
    // Trial (Vello): `--dump <dir>` writes each frame's scene there as JSON, named by `dump_name`, for
    // clients/app/strict/trial-vello to draw again; and each frame's `renderer.draw` time is kept in `draws`.
    let dump_dir = flag("--dump").map(PathBuf::from);
    let dump_name: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(Some("frame".into()));
    let draws: std::cell::RefCell<Vec<f64>> = std::cell::RefCell::new(Vec::new());
    // A frame drawn, and with `read`, waited for and read back.
    let mut frame = |screen: &mut Screen, read: bool| -> anyhow::Result<(u32, u32, Vec<u8>)> {
        screen.idle();
        screen.paint(size, scale, &mut text, renderer.sprite_atlas(), &mut scene);
        let drawing = std::time::Instant::now();
        renderer.draw(&scene);
        draws.borrow_mut().push(drawing.elapsed().as_secs_f64() * 1000.0);
        if let (Some(dir), Some(name)) = (&dump_dir, dump_name.borrow().as_ref()) {
            std::fs::create_dir_all(dir)?;
            std::fs::write(dir.join(format!("{name}.json")), dump(&scene, [device.width.0 as u32, device.height.0 as u32]).to_string())?;
        }
        if !read {
            return Ok((0, 0, Vec::new()));
        }
        let read = renderer.read().ok_or_else(|| anyhow::anyhow!("nothing was drawn"))?;
        first_frame.set(first_frame.get().or(Some(gpu.elapsed())));
        Ok(read)
    };
    for line in requests.lines().filter(|line| !line.trim().is_empty()) {
        let (w, h, rgba) = frame(&mut screen, true)?;
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
        screen.idle();
        // A `settle` not settled at once is told so at its deadline.
        if answered.borrow().is_none() {
            std::thread::sleep(std::time::Duration::from_secs_f64(deadline / 1000.0));
            screen.fire();
        }
        let answer = answered.borrow_mut().take();
        println!("{}", answer.unwrap_or_else(|| json!({"error": "it didn't answer"})));
    }
    let (w, h, rgba) = frame(&mut screen, true)?;
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
        // Trial: each phase's `renderer.draw` times, median and worst, by name.
        let phases: std::cell::RefCell<Vec<(String, f64, f64)>> = std::cell::RefCell::new(Vec::new());
        let phase_name: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
        let mut frames = |screen: &mut Screen, act: &mut dyn FnMut(&mut Screen, u32) -> bool| -> anyhow::Result<Option<f64>> {
            let started = std::time::Instant::now();
            draws.borrow_mut().clear();
            for at in 0..FRAMES {
                if !act(screen, at) {
                    return Ok(None);
                }
                let phase = phase_name.borrow().clone();
                *dump_name.borrow_mut() = (phase != "idle" || at == 0).then(|| format!("{phase}-{at:02}"));
                frame(screen, at + 1 == FRAMES)?;
            }
            let mut drawn = draws.borrow().clone();
            drawn.sort_by(f64::total_cmp);
            phases.borrow_mut().push((phase_name.borrow().clone(), drawn[drawn.len() / 2], *drawn.last().unwrap_or(&0.0)));
            Ok(Some(started.elapsed().as_secs_f64() * 1000.0 / f64::from(FRAMES)))
        };
        *phase_name.borrow_mut() = "idle".into();
        let idle = frames(&mut screen, &mut |_, _| true)?;
        *phase_name.borrow_mut() = "typing".into();
        let found = |screen: &mut Screen, role: &str| ask(screen, json!({"op": "query", "role": role}))?["result"]["nodes"][0]["node"].as_u64();
        let field = ["searchbox", "textbox"].into_iter().find_map(|role| found(&mut screen, role));
        let typing = match field {
            Some(node) => frames(&mut screen, &mut |screen, _| ask(screen, json!({"op": "type", "node": node, "text": "a"})).is_some_and(|answer| answer.get("result").is_some()))?,
            None => None,
        };
        *phase_name.borrow_mut() = "scrolling".into();
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
        let memory_mb = (resident().saturating_sub(before) as f64 / 1e5).round() / 10.0;
        eprintln!(
            "measured {}",
            json!({"first_frame_ms": ms(first_frame_ms), "idle_ms": ms(idle), "typing_ms": ms(typing), "scrolling_ms": ms(scrolling), "memory_mb": memory_mb})
        );
        let draw: serde_json::Map<String, Value> = phases.borrow().iter().map(|(name, median, worst)| (name.clone(), json!({"median_ms": median, "worst_ms": worst}))).collect();
        eprintln!("drawn {}", Value::Object(draw));
    }
    renderer.destroy();
    Ok(())
}

/// What the screen answers `request`, an inspector's, once what it set off has run.
fn ask(screen: &mut Screen, mut request: Value) -> Option<Value> {
    request["v"] = json!(strict::inspect::VERSION);
    request["id"] = json!(0);
    let request = strict::inspect::Request::parse(&request.to_string()).ok()?;
    let answered = std::rc::Rc::new(std::cell::RefCell::new(None));
    let to = answered.clone();
    screen.inspect(request, Box::new(move |answer| *to.borrow_mut() = Some(answer)));
    screen.idle();
    answered.borrow_mut().take()
}

/// Trial (Vello): a frame's scene as JSON, in device pixels: every primitive with its draw order, each paragraph's
/// words and styles, and what each sprite is a picture of.
fn dump(scene: &Scene, size: [u32; 2]) -> Value {
    use strict::geometry::{Bounds, Corners, Hsla, Rgba, ScaledPixels};
    use strict::render::scene::{SpriteSource, TransformationMatrix};
    let b = |bounds: &Bounds<ScaledPixels>| json!([bounds.origin.x.0, bounds.origin.y.0, bounds.size.width.0, bounds.size.height.0]);
    let c = |colour: &Hsla| {
        let rgba = Rgba::from(*colour);
        json!([rgba.r, rgba.g, rgba.b, rgba.a])
    };
    let r = |radii: &Corners<ScaledPixels>| json!([radii.top_left.0, radii.top_right.0, radii.bottom_right.0, radii.bottom_left.0]);
    let t = |turn: &TransformationMatrix| json!([turn.rotation_scale[0][0], turn.rotation_scale[0][1], turn.rotation_scale[1][0], turn.rotation_scale[1][1], turn.translation[0], turn.translation[1]]);
    let quads: Vec<Value> = scene.quads.iter().map(|quad| json!({
        "order": quad.order, "bounds": b(&quad.bounds), "mask": b(&quad.content_mask.bounds), "background": c(&quad.background.solid),
        "border_color": c(&quad.border_color), "radii": r(&quad.corner_radii),
        "borders": [quad.border_widths.top.0, quad.border_widths.right.0, quad.border_widths.bottom.0, quad.border_widths.left.0],
        "dashed": quad.border_style as u32 == 1, "transform": t(&quad.transformation),
    })).collect();
    let shadows: Vec<Value> = scene.shadows.iter().map(|shadow| json!({
        "order": shadow.order, "bounds": b(&shadow.bounds), "mask": b(&shadow.content_mask.bounds), "color": c(&shadow.color),
        "radii": r(&shadow.corner_radii), "blur": shadow.blur_radius.0, "inset": shadow.inset == 1, "outer": b(&shadow.outer),
        "outer_radii": r(&shadow.outer_radii), "transform": t(&shadow.transformation),
    })).collect();
    let underlines: Vec<Value> = scene.underlines.iter().map(|line| json!({
        "order": line.order, "bounds": b(&line.bounds), "mask": b(&line.content_mask.bounds), "color": c(&line.color), "thickness": line.thickness.0,
    })).collect();
    let sprites: Vec<Value> = scene.polychrome_sprites.iter().filter(|sprite| !scene.sources.glyph_tiles.contains(&sprite.tile.key())).map(|sprite| {
        let source = match scene.sources.sprites.get(&sprite.tile.key()) {
            Some(SpriteSource::Image { src, size }) => json!({"image": src, "size": size}),
            Some(SpriteSource::Path { d, view_box, fill, stroke, stroke_width }) => json!({
                "path": d, "view_box": view_box, "fill": fill.as_ref().map(c), "stroke": stroke.as_ref().map(c), "stroke_width": stroke_width,
            }),
            None => Value::Null,
        };
        json!({"order": sprite.order, "bounds": b(&sprite.bounds), "mask": b(&sprite.content_mask.bounds), "radii": r(&sprite.corner_radii),
               "opacity": sprite.opacity, "grayscale": sprite.grayscale, "transform": t(&sprite.transformation), "source": source})
    }).collect();
    let backdrops: Vec<Value> = scene.backdrops.iter().map(|backdrop| json!({
        "order": backdrop.order, "bounds": b(&backdrop.bounds), "mask": b(&backdrop.content_mask.bounds), "radii": r(&backdrop.corner_radii),
        "blur": backdrop.blur.0, "saturation": backdrop.saturation,
    })).collect();
    let texts: Vec<Value> = scene.sources.texts.iter().map(|text| json!({
        "order": text.order, "words": text.words, "origin": text.origin, "width": text.width, "wrap": text.wrap, "max_lines": text.max_lines, "align": text.align,
        "line_height": text.line_height, "scale": text.scale, "mask": b(&text.mask), "transform": t(&text.transformation),
        "runs": text.runs.iter().map(|(range, style, colour)| json!({
            "start": range.start, "end": range.end, "size": style.size, "weight": style.weight, "tracking": style.tracking,
            "line_height": style.line_height, "tabular": style.tabular, "color": c(colour),
        })).collect::<Vec<_>>(),
    })).collect();
    let glyphs = scene.monochrome_sprites.len() + scene.polychrome_sprites.iter().filter(|sprite| scene.sources.glyph_tiles.contains(&sprite.tile.key())).count();
    json!({"size": size, "quads": quads, "shadows": shadows, "underlines": underlines, "sprites": sprites, "backdrops": backdrops, "texts": texts, "glyphs": glyphs})
}
