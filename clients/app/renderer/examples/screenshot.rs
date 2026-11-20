//! A native page drawn offscreen, the same on every run: a PNG of it and its semantic tree as JSON.
//!
//!     cargo run -p renderer --example screenshot --features fake -- <page's built folder> <out.png> <out.json> \
//!       [--ui core/ui/dist/client] [--answers answers.json] [--size 480x360] [--scale 2] [--dark] [--requests <file>|-]
//!       [--safe-area <top>,<right>,<bottom>,<left>] [--touch] [--panel <corner>] [--gpu] [--measure] [--seed <n>]
//!
//! It runs the module the folder's `index.html` loads, as `bun app.js` built it, with the UI library a native host
//! loads from `--ui` (`bun run build` in core/ui), and answers what the page asks its client from `--answers`, as
//! core/ui's `record.js` does: `{ "<tool>": <its answer>, "<method>": <its result> }`. It's a window of the fake
//! platform's (`renderer::platform`'s `fake`, built with that feature), drawn by the loop every host's windows are,
//! whose display ticks as this says and whose time is the page's clock, with Allo alone as the typeface, the
//! sandbox's fixed clock and randomness, and our renderer drawing offscreen on
//! whatever GPU Vulkan finds (`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json` picks Mesa's software Vulkan,
//! lavapipe, where there's no GPU), or on Windows Direct3D 12's (WARP, Windows' own software one, where there's none). `--requests` gives it the inspector's requests first, one a line
//! (`renderer::inspect`), as a native window takes them on its socket, from a file or stdin (`-`), each answered on
//! stdout, a frame drawn before each: `{"v":1,"id":1,"op":"tap","text":"Backups"}`. What a person does (`tap`, `key`,
//! `scroll`, `drag`, `replay`) comes as the display's input, each event at its own time, a click held down 80 ms,
//! and what it presses scrolled into sight first, as Playwright's click does. Its clock is paused: `clock`'s `step`
//! moves it, the display ticking each frame meanwhile; and after the requests, until what it shows comes to rest but
//! for loops (at most 3 s), which is what's pictured. `--touch` draws it for a finger, as a phone's window is (`pointer: coarse`),
//! whose touches are the inspector's own, as a phone's host hands its screen each touch.
//! `--panel` opens it as a computer's panel is (a Mac's launcher, its corners `<corner>` round), summoned: what its page
//! puts on the system's glass is a window of its own, each drawn by a renderer of its own where the panel arranges it,
//! over the system's glass, a flat grey here, and its picture is theirs put together.
//! `--seed` makes it adversarial, the same way each time for the same seed, as GPUI's randomized executor is: one
//! refresh in five comes late (up to three in a row), what a person does comes a moment after what came before, as
//! things are on their way somewhere, and a click is held from 30 ms to a quarter of a second.
//! `--gpu` waits for the GPU after each frame drawn, and says how long that took with its cost (`gpu_ms`): on a real
//! GPU, its time for the frame, as a device that draws it would take.
//! `--measure` says, on stderr after the rest, how quickly it draws, as a line `measured {…}`: its first
//! frame (`first_frame_ms`, from the GPU's start to the frame read back), a frame's time when nothing changes
//! (`idle_ms`), as its first text field takes a character a frame (`typing_ms`) and as its first list scrolls
//! (`scrolling_ms`), each where it has one, each the loop's whole turn as a window takes it, the longest turn of all
//! it drew (`worst_paint_ms`: laying it out, making its scene and handing it to the GPU, which a picture's decoding,
//! in the background, isn't part of), the memory drawing took (`memory_mb`: the GPU, its atlas and the
//! renderer, as the process's resident set grew), and what a second window of its size adds (`window_mb`: a renderer
//! of its own, drawing the same frame, the GPU and the atlas shared).

use std::path::PathBuf;

use serde_json::{Value, json};
use renderer::geometry::{DevicePixels, Pixels, Size};
use renderer::js::{Clock, Files, Thread};
use renderer::platform::{Event, WindowOptions, Windows};
use renderer::render::renderer::Renderer;
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
    let clock = Clock::fixed();
    let js = Thread::start(&page, files, None, clock.clone(), touch)?;
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
    // A window of the fake platform's, drawn by the loop every host's windows are (`Windows::turn`).
    let gpu = std::time::Instant::now();
    let resident = || memory_stats::memory_stats().map_or(0, |memory| memory.physical_mem);
    let before = resident();
    let mut windows = Windows::fake(scale, 60.0, clock)?;
    // With a seed, adversarial as a busy device and a person are: refreshes late, input as things move.
    let seed: u64 = flag("--seed").and_then(|seed| seed.parse().ok()).unwrap_or(0);
    windows.seed(seed);
    let options = WindowOptions { title: "screenshot".into(), app_id: "screenshot".into(), size: Size { width: Pixels(width), height: Pixels(height) }, tag: json!({}), panel, places: None, frame: None, root: false, icon: None, owner: None, snapshot: None };
    let said = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let saying = said.clone();
    let id = windows.open(options, Ok(screen), move |event| match event {
        // A window would open it in a web view instead.
        Event::Mismatched(why) => saying.borrow_mut().push(format!("it opens in a web view: {why}")),
        Event::Failed(why) => saying.borrow_mut().push(format!("its page didn't start: {why}")),
        _ => {}
    })?;
    if panel.is_some() {
        windows.summon(id, std::time::Instant::now(), None);
    }
    let first_frame = std::cell::Cell::new(None);
    let worst_turn = std::cell::Cell::new(std::time::Duration::ZERO);
    // A frame: its page has run what it was sent, the display shows a frame, and the loop takes its turn, drawing what
    // changed; again once the pictures it decodes in the background are ready, as a window is woken to.
    let frame = |windows: &mut Windows| {
        loop {
            windows.wait();
            windows.tick();
            let turning = std::time::Instant::now();
            windows.turn();
            worst_turn.set(worst_turn.get().max(turning.elapsed()));
            if first_frame.get().is_none() {
                windows.presented(id);
                first_frame.set(Some(gpu.elapsed()));
            }
            if !windows.with(id, |screen| screen.decoding()).unwrap_or(false) {
                break;
            }
        }
    };
    for line in requests.lines().filter(|line| !line.trim().is_empty()) {
        // A frame is drawn before each, as a window draws what the last one changed before the next comes.
        frame(&mut windows);
        let mut request = match renderer::inspect::Request::parse(line) {
            Ok(request) => request,
            Err(refused) => {
                println!("{refused}");
                continue;
            }
        };
        if gpu_times && matches!(request.op.as_str(), "profile" | "replay") {
            request.inputs.insert("gpu".into(), json!(true));
        }
        // What a person does comes as the display's input, on the platform's time, as a host's does; with a seed, a
        // moment or two after what came before, so it lands as things are on their way somewhere.
        if !touch && INPUT.contains(&request.op.as_str()) {
            if seed != 0 && request.op != "clock" {
                windows.run(windows.random(50) as f64);
            }
            let answered = input(&mut windows, id, &request);
            println!("{}", renderer::inspect::answer(&request.id, answered));
            continue;
        }
        let deadline = request.f64("deadline").unwrap_or(5000.0);
        let answered = std::rc::Rc::new(std::cell::RefCell::new(None));
        let to = answered.clone();
        windows.inspect(id, request, Box::new(move |answer| *to.borrow_mut() = Some(answer)));
        windows.wait();
        windows.turn();
        // A `settle` not settled at once is told so at its deadline, as the loop waits for what's due.
        let asked = std::time::Instant::now();
        while answered.borrow().is_none() && asked.elapsed().as_secs_f64() * 1000.0 < deadline + 1000.0 {
            std::thread::sleep(windows.due().unwrap_or(std::time::Duration::from_millis(10)).min(std::time::Duration::from_millis(100)));
            windows.wait();
            windows.turn();
        }
        let answer = answered.borrow_mut().take();
        println!("{}", answer.unwrap_or_else(|| json!({"error": "it didn't answer"})));
    }
    // What it comes to rest on is what's pictured, as a page's animations that end are waited for on the DOM
    // (scenes.js): its time runs, the display ticking, until nothing's on its way anywhere but in a loop (a spinner),
    // for at most SETTLE_MS.
    let settling = renderer::platform::fake_time();
    frame(&mut windows);
    while windows.with(id, |screen| screen.arriving()).unwrap_or(false) && renderer::platform::fake_time() - settling < SETTLE_MS {
        windows.run(1000.0 / 60.0);
    }
    let drawn = started.elapsed();
    let (w, h, rgba) = windows.presented(id).ok_or_else(|| anyhow::anyhow!("nothing was drawn"))?;
    image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| anyhow::anyhow!("the frame is the wrong size"))?.save(png)?;
    let semantics = windows.with(id, |screen| (screen.semantics(), screen.failed.clone())).unwrap_or((json!([]), None));
    std::fs::write(tree, serde_json::to_string_pretty(&semantics.0)?)?;
    if let Some(failed) = &semantics.1 {
        eprintln!("the page failed: {failed}");
    }
    for said in said.borrow().iter() {
        eprintln!("{said}");
    }
    eprintln!("ran its JS in {ran:?}, first frame in {drawn:?}, {w}x{h} px");
    if args.iter().any(|arg| arg == "--measure") {
        // How long a frame takes, on average over FRAMES each after `act`, which says false where there's nothing
        // to do it to: drawn one after another, as a window draws them, and the last waited for.
        const FRAMES: u32 = 30;
        let frames = |windows: &mut Windows, act: &mut dyn FnMut(&mut Windows, u32) -> bool| -> anyhow::Result<Option<f64>> {
            let started = std::time::Instant::now();
            for at in 0..FRAMES {
                if !act(windows, at) {
                    return Ok(None);
                }
                windows.redraw(id);
                frame(windows);
            }
            windows.presented(id);
            Ok(Some(started.elapsed().as_secs_f64() * 1000.0 / f64::from(FRAMES)))
        };
        let idle = frames(&mut windows, &mut |_, _| true)?;
        let found = |windows: &mut Windows, role: &str| ask(windows, id, json!({"op": "query", "role": role}))?["result"]["nodes"][0]["node"].as_u64();
        let field = ["searchbox", "textbox"].into_iter().find_map(|role| found(&mut windows, role));
        let typing = match field {
            Some(node) => frames(&mut windows, &mut |windows, _| ask(windows, id, json!({"op": "type", "node": node, "text": "a"})).is_some_and(|answer| answer.get("result").is_some()))?,
            None => None,
        };
        let list = found(&mut windows, "list");
        let scrolling = match list {
            Some(node) => frames(&mut windows, &mut |windows, at| {
                let dy = if at < FRAMES / 2 { 40 } else { -40 };
                ask(windows, id, json!({"op": "scroll", "node": node, "dy": dy})).is_some_and(|answer| answer.get("result").is_some())
            })?,
            None => None,
        };
        let ms = |ms: Option<f64>| ms.map(|ms| (ms * 100.0).round() / 100.0);
        let first_frame_ms = first_frame.get().map(|first| first.as_secs_f64() * 1000.0);
        let worst_paint_ms = worst_turn.get().as_secs_f64() * 1000.0;
        let mb = |bytes: usize| (bytes as f64 / 1e5).round() / 10.0;
        let memory_mb = mb(resident().saturating_sub(before));
        let alone = resident();
        let (scene, context, atlas) = windows.drawn(id).ok_or_else(|| anyhow::anyhow!("its window is gone"))?;
        let device = Size { width: DevicePixels((width * scale).round() as i32), height: DevicePixels((height * scale).round() as i32) };
        let mut second = Renderer::offscreen(context, atlas.clone(), device);
        second.draw(scene, [0.0, 0.0]);
        second.read();
        let window_mb = mb(resident().saturating_sub(alone));
        second.destroy();
        eprintln!(
            "measured {}",
            json!({"first_frame_ms": ms(first_frame_ms), "worst_paint_ms": ms(Some(worst_paint_ms)), "idle_ms": ms(idle), "typing_ms": ms(typing), "scrolling_ms": ms(scrolling), "memory_mb": memory_mb, "window_mb": window_mb})
        );
    }
    Ok(())
}

/// What window `id` answers `request`, an inspector's, once what it set off has run.
fn ask(windows: &mut Windows, id: renderer::platform::WindowId, mut request: Value) -> Option<Value> {
    request["v"] = json!(renderer::inspect::VERSION);
    request["id"] = json!(0);
    let request = renderer::inspect::Request::parse(&request.to_string()).ok()?;
    let answered = std::rc::Rc::new(std::cell::RefCell::new(None));
    let to = answered.clone();
    windows.inspect(id, request, Box::new(move |answer| *to.borrow_mut() = Some(answer)));
    windows.wait();
    answered.borrow_mut().take()
}

/// The inspector's requests that a person's input answers, and its clock's: on the fake platform, the display's own
/// events, each at its own time, with the display's frames between, as a host gives a window input, and its time
/// moved on with the display ticking through it. A finger's (`--touch`) are the
/// inspector's own, as a phone's host hands a window's screen each touch.
const INPUT: [&str; 7] = ["tap", "key", "scroll", "drag", "replay", "clock", "wait_for"];

/// The longest it's run for, at the end, for what it shows to come to rest.
const SETTLE_MS: f64 = 3000.0;

/// How long a click holds the button down, as a person's does.
const CLICK_MS: f64 = 80.0;

/// Where a request's press goes: the middle of the node it selects, as the window last drew it, or its point.
fn point_of(windows: &mut Windows, id: renderer::platform::WindowId, request: &renderer::inspect::Request) -> anyhow::Result<(f32, f32)> {
    if !request.selects() {
        return match (request.f32("x"), request.f32("y")) {
            (Some(x), Some(y)) => Ok((x, y)),
            _ => anyhow::bail!("say where: a node, by a selector, or a point, x and y"),
        };
    }
    let mut query = json!({"op": "query"});
    for input in ["node", "role", "name", "text", "key"] {
        if let Some(value) = request.inputs.get(input) {
            query[input] = value.clone();
        }
    }
    let nth = request.inputs.get("nth").and_then(Value::as_u64).unwrap_or(0) as usize;
    let found = ask(windows, id, query.clone()).ok_or_else(|| anyhow::anyhow!("its window didn't answer"))?;
    let node = found["result"]["nodes"].get(nth).and_then(|node| node["node"].as_u64()).ok_or_else(|| anyhow::anyhow!("no node matches that"))?;
    // Scrolled into sight first where it isn't, as a person scrolls to it, and Playwright's click does, then found
    // again as it's drawn now: what's pressed is what's there, though the page drew it afresh meanwhile.
    if windows.with(id, |screen| screen.reveal(node)).unwrap_or(false) {
        windows.run(0.0);
    }
    // Where it's drawn: what was drawn afresh and isn't yet is waited for, a second at most, as Playwright waits for
    // what it clicks to be there and still.
    let started = renderer::platform::fake_time();
    let rect = loop {
        let found = ask(windows, id, query.clone()).ok_or_else(|| anyhow::anyhow!("its window didn't answer"))?;
        if let Some(rect) = found["result"]["nodes"].get(nth).map(|node| node["rect"].clone()).filter(Value::is_object) {
            break rect;
        }
        if renderer::platform::fake_time() - started >= 1000.0 {
            anyhow::bail!("what matches that isn't drawn");
        }
        windows.run(1000.0 / 60.0);
    };
    let at = |edge: &str| rect[edge].as_f64().unwrap_or_default() as f32;
    Ok((at("x") + at("width") / 2.0, at("y") + at("height") / 2.0))
}

/// What a person's input request does, played as the display's events (`INPUT`).
fn input(windows: &mut Windows, id: renderer::platform::WindowId, request: &renderer::inspect::Request) -> anyhow::Result<Value> {
    match request.op.as_str() {
        "tap" => {
            let (x, y) = point_of(windows, id, request)?;
            let clicks = request.inputs.get("clicks").and_then(Value::as_u64).unwrap_or(1).clamp(1, 3);
            windows.pointer_moved(id, x, y);
            windows.run(0.0);
            // With a seed, held as long as a person might: from a quick tap to a quarter of a second.
            let held = request.f64("hold").unwrap_or(CLICK_MS + windows.random(170) as f64 - windows.random(50) as f64);
            for _ in 0..clicks {
                windows.pointer_down(id, x, y, 0, request.bool("shift"));
                windows.run(held);
                windows.pointer_up(id, x, y, 0);
                windows.run(0.0);
            }
            Ok(json!({"tapped": {"x": x, "y": y}}))
        }
        "clock" => {
            if request.inputs.get("pause").and_then(Value::as_bool) == Some(false) {
                anyhow::bail!("the fake platform's clock is its driver's: step or seek it");
            }
            let now = renderer::platform::fake_time();
            let ms = request.f64("step").unwrap_or(0.0) + request.f64("seek").map_or(0.0, |at| at - now);
            if ms < 0.0 {
                anyhow::bail!("the clock goes forward only: it's at {now} ms");
            }
            windows.run(ms);
            Ok(json!({"time": renderer::platform::fake_time(), "paused": true}))
        }
        // As Playwright's `locator.waitFor` and its assertions do: what's selected comes to be `state` (`shown`: drawn
        // whole and in the window; `gone`: not there; `focused`: with the keyboard; `there`: there at all), with its
        // states as `is` says where it says, within `timeout` ms (1000 unless said), the display ticking meanwhile.
        "wait_for" => {
            let state = request.str("state").unwrap_or("shown");
            let timeout = request.f64("timeout").unwrap_or(1000.0);
            let mut query = json!({"op": "query"});
            for input in ["node", "role", "name", "text", "key"] {
                if let Some(value) = request.inputs.get(input) {
                    query[input] = value.clone();
                }
            }
            let is = request.inputs.get("is").and_then(Value::as_object).cloned().unwrap_or_default();
            let started = renderer::platform::fake_time();
            loop {
                let found = ask(windows, id, query.clone()).ok_or_else(|| anyhow::anyhow!("its window didn't answer"))?;
                let nodes = found["result"]["nodes"].as_array().cloned().unwrap_or_default();
                let node = nodes.first();
                let states = |node: &Value| is.iter().all(|(state, want)| node["states"].get(state).unwrap_or(&json!(false)) == want);
                let met = match state {
                    "gone" => nodes.is_empty(),
                    "focused" => nodes.iter().any(|node| node["focused"] == true && states(node)),
                    "there" => node.is_some_and(states),
                    _ => node.is_some_and(|node| node["opacity"].as_f64().is_some_and(|opacity| opacity >= 0.99) && node["seen"].as_f64().is_some_and(|seen| seen >= 0.99) && states(node)),
                };
                let waited = renderer::platform::fake_time() - started;
                if met || waited >= timeout {
                    return Ok(json!({"met": met, "ms": waited, "node": node}));
                }
                windows.run(1000.0 / 60.0);
            }
        }
        "key" => {
            windows.key(id, request)?;
            windows.run(0.0);
            Ok(json!({"key": request.str("key")}))
        }
        "scroll" => {
            let (x, y) = if request.selects() { point_of(windows, id, request)? } else { (0.0, 0.0) };
            let (dx, dy) = (request.f32("dx").unwrap_or(0.0), request.f32("dy").unwrap_or(0.0));
            let momentum = request.inputs.get("momentum").and_then(Value::as_bool).unwrap_or(false);
            windows.wheel(id, (x, y), (dx, dy), request.str("phase"), momentum);
            windows.run(0.0);
            let scrolled = windows.with(id, |screen| screen.scrolled_under(renderer::geometry::point(x, y))).flatten();
            Ok(json!({"scrolled": scrolled.map(|at| json!({"x": at.x, "y": at.y}))}))
        }
        "drag" => {
            let (x, y) = point_of(windows, id, request)?;
            let (dx, dy) = (request.f32("dx").unwrap_or(0.0), request.f32("dy").unwrap_or(0.0));
            let steps = request.inputs.get("steps").and_then(Value::as_u64).unwrap_or(10).max(1);
            let ms = request.f64("ms").unwrap_or(200.0).max(0.0);
            windows.pointer_moved(id, x, y);
            windows.pointer_down(id, x, y, 0, false);
            for step in 1..=steps {
                windows.run(ms / steps as f64);
                let part = step as f32 / steps as f32;
                windows.pointer_moved(id, x + dx * part, y + dy * part);
            }
            windows.pointer_up(id, x + dx, y + dy, 0);
            windows.run(0.0);
            Ok(json!({"dragged": {"x": dx, "y": dy}}))
        }
        _ => replay(windows, id, request),
    }
}

/// The inspector's `replay`, as the display's events: the press goes down, moves along `moves` (`{dx, dy, ms}`,
/// sampled `input_hz` times a second) and lifts unless `lift` is false, on a display of `hz`, each of its frames
/// reported as `replay` reports one (`renderer::inspect`'s), until `after` ms past its end or nothing moves.
fn replay(windows: &mut Windows, id: renderer::platform::WindowId, request: &renderer::inspect::Request) -> anyhow::Result<Value> {
    let (x, y) = point_of(windows, id, request)?;
    let moves: Vec<(f32, f32, f64)> = request.inputs.get("moves").and_then(Value::as_array).ok_or_else(|| anyhow::anyhow!("say how it moves: `moves`, each {{dx, dy, ms}}"))?
        .iter()
        .map(|one| (one["dx"].as_f64().unwrap_or(0.0) as f32, one["dy"].as_f64().unwrap_or(0.0) as f32, one["ms"].as_f64().unwrap_or(0.0).max(0.0)))
        .collect();
    let lift = request.inputs.get("lift").and_then(Value::as_bool) != Some(false);
    let hz = request.f64("hz").unwrap_or(60.0).clamp(1.0, 480.0);
    let every = 1000.0 / request.f64("input_hz").unwrap_or(hz).clamp(1.0, 1000.0);
    let after = request.f64("after").unwrap_or(2000.0).max(0.0);
    let snaps = renderer::inspect::snaps(request);
    let track = match request.inputs.get("track") {
        Some(Value::Object(selector)) => {
            let mut query = json!({"op": "query"});
            for (name, value) in selector {
                query[name] = value.clone();
            }
            ask(windows, id, query).and_then(|found| found["result"]["nodes"][0]["node"].as_u64())
        }
        _ => None,
    };
    let length: f64 = moves.iter().map(|(_, _, ms)| ms).sum();
    // Where it is `t` ms in.
    let at = |t: f64| {
        let (mut at, mut since) = ((x, y), 0.0);
        for (dx, dy, ms) in &moves {
            let part = if *ms <= 0.0 { 1.0 } else { ((t - since) / ms).clamp(0.0, 1.0) as f32 };
            at = (at.0 + dx * part, at.1 + dy * part);
            if t < since + ms {
                break;
            }
            since += ms;
        }
        at
    };
    let samples: Vec<(f64, (f32, f32))> = (1..=(length / every).ceil() as usize).map(|n| (n as f64 * every).min(length)).map(|t| (t, at(t))).collect();
    windows.set_refresh(hz);
    // With `frame_ms`, each frame takes that long to draw, as on a device others load, which presses it as a window's
    // late frames do.
    windows.set_frame_ms(request.f64("frame_ms").unwrap_or(0.0));
    let refresh = 1000.0 / hz;
    let begun = renderer::platform::fake_time();
    windows.pointer_moved(id, x, y);
    windows.pointer_down(id, x, y, 0, false);
    windows.run(0.0);
    let (mut next, mut lifted, mut frames) = (0, !lift, Vec::new());
    let elapsed = || renderer::platform::fake_time() - begun;
    loop {
        // The display's next refresh, and each move before it at its own time, as a device delivers them.
        // At least a microsecond on, so a clock a hair short of a refresh isn't moved by nothing.
        let now = renderer::platform::fake_time();
        let boundary = ((now / refresh).floor() + 1.0) * refresh;
        let tick = if boundary - now < 0.001 { boundary + refresh } else { boundary } - begun;
        while let Some((t, (mx, my))) = samples.get(next).copied().filter(|(t, _)| *t <= tick) {
            windows.run((t - elapsed()).max(0.0));
            windows.pointer_moved(id, mx, my);
            next += 1;
        }
        if next == samples.len() && !lifted {
            let (ux, uy) = at(length);
            windows.run((length - elapsed()).max(0.0));
            windows.pointer_up(id, ux, uy, 0);
            lifted = true;
        }
        let before = windows.frames(id);
        windows.run((tick - elapsed()).max(0.0));
        let drew = windows.frames(id) > before;
        let mut frame = match drew {
            true => windows.cost(id).unwrap_or_default(),
            false => json!({"mutations": 0, "script_ms": 0.0, "layout_ms": 0.0, "paint_ms": 0.0, "draws": 0, "work": {"quads": 0, "shadows": 0, "sprites": 0, "backdrops": 0, "rasters": 0, "copied": 0, "uploaded": 0}}),
        };
        frame["at"] = json!((tick * 100.0).round() / 100.0);
        frame["pressure"] = json!(windows.pressure());
        frame["drawn"] = json!(drew);
        if let Some(node) = track
            && let Some(found) = ask(windows, id, json!({"op": "query", "node": node}))
            && let Some(rect) = found["result"]["nodes"][0]["rect"].as_object()
        {
            frame["x"] = rect["x"].clone();
            frame["y"] = rect["y"].clone();
        }
        if let Some(scrolled) = windows.with(id, |screen| screen.scrolled_under(renderer::geometry::point(x, y))).flatten() {
            frame["scroll"] = json!({"x": scrolled.x, "y": scrolled.y});
        }
        if snaps && let Some((w, h, rgba)) = windows.presented(id) {
            for (name, value) in renderer::inspect::png(w, h, rgba)?.as_object().into_iter().flatten() {
                frame[name] = value.clone();
            }
        }
        frames.push(frame);
        let still = !windows.wanted();
        if lifted && next == samples.len() && (still || tick >= length + after) {
            break;
        }
    }
    windows.set_refresh(60.0);
    windows.set_frame_ms(0.0);
    // What's been taken out but is still drawn going, which `after` should have seen go.
    let leaving = ask(windows, id, json!({"op": "window"})).and_then(|answer| answer["result"]["leaving"].as_u64()).unwrap_or(0);
    let budget = refresh;
    let over = frames.iter().filter(|frame| frame["paint_ms"].as_f64().is_some_and(|ms| ms > budget)).count();
    Ok(json!({"hz": hz, "budget_ms": (budget * 100.0).round() / 100.0, "over": over, "leaving": leaving, "frames": frames, "time": renderer::platform::fake_time(), "paused": true}))
}
