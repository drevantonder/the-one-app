//! A native page drawn offscreen, the same on every run: a PNG of it and its semantic tree as JSON.
//!
//!     cargo run -p renderer --example screenshot --features fake -- <page's built folder> <out.png> <out.json> \
//!       [--ui core/ui/dist/client] [--answers answers.json] [--size 480x360] [--scale 2] [--dark] [--requests <file>|-]
//!       [--safe-area <top>,<right>,<bottom>,<left>] [--touch] [--panel <corner>] [--measure] [--seed <n>]
//!       [--latency <ms> [--cached <tool>,…]] [--slower <times>] [--time-zone <IANA name>]
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
//! `scroll`, `drag`, `replay`, `away`, the app going to the background and back, a panel's `put_away` and `summon`, and the
//! device's `back`)
//! comes as the display's input, each event at its own time, a click held down 80 ms,
//! and what it presses scrolled into sight first, as Playwright's click does. Its clock is paused: `clock`'s `step`
//! moves it, the display ticking each frame meanwhile; and after the requests, until what it shows comes to rest but
//! for loops (at most 3 s), which is what's pictured. `changed` is a server saying something changed, as its client
//! tells a page on screen, with what its tools answer from then on (`answers`, as `--answers` has them). A run whose
//! page crashed fails, saying why, and so does one whose process grows past `HARNESS_MEMORY`, at once, wherever it is. `--touch` draws it for a finger, as a phone's window is (`pointer: coarse`),
//! and presses are a finger's, as the display's touches, which its window takes as every phone's host does
//! (`view::touch`): its slop, pans, a sheet taking the finger, and the swipe back from the left edge.
//! `--panel` opens it as a computer's panel is (a Mac's launcher, its corners `<corner>` round), summoned: what its page
//! puts on the system's glass is a window of its own, each drawn by a renderer of its own where the panel arranges it,
//! over the system's glass, a flat grey here, and its picture is theirs put together.
//! `--seed` makes it adversarial, the same way each time for the same seed, as GPUI's randomized executor is: one
//! refresh in five comes late (up to three in a row), what a person does comes a moment after what came before, as
//! things are on their way somewhere, a click is held from 30 ms to a quarter of a second, one batch in four of
//! what its page changed reaches the window one to three of its turns late, as a page slower than its window's does,
//! and of what the window tells its page together, which its thread runs in one turn, now and then one waits for a turn
//! of its own (`Thread::seed`).
//! `--latency` has each tool's answer come that many milliseconds of the page's clock after it was asked, as a round
//! trip to a server takes, but for the tools `--cached` names, which a client answers at once from what it keeps
//! (a tool that only reads): so a page that opens on what isn't kept, or asks again for what an answer said, opens
//! slowly here as on a device. The page's clock runs as it opens until it's had every answer, at most 5 s.
//! `--slower` has each picture take that many times as long to decode as on this machine, as on a device with a slower
//! CPU, where they come later after the page asks for them: the inspector's `window` says the longest one took
//! (`picture_ms`).
//! `--time-zone` has the page's clock show times in that time zone, as a device in it does: the clock is `UTC` otherwise.
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
use renderer::touch::Phase;
use renderer::render::renderer::Renderer;
use renderer::{Appearance, Screen};
use url::Url;

/// The most the harness's process may hold: a run takes 200 to 500 MB, so one past it is running away, and is stopped
/// before it takes a machine others share (20 Nov: a page out of memory took 14 GB and swap with it).
const HARNESS_MEMORY: usize = 2 << 30;

fn main() -> anyhow::Result<()> {
    // Watched from a thread of its own, as whatever runs away may be holding the one that would notice.
    std::thread::spawn(|| loop {
        let held = memory_stats::memory_stats().map_or(0, |memory| memory.physical_mem);
        if held > HARNESS_MEMORY {
            eprintln!("Error: the harness grew to {} MB, past its {} MB, so it stopped", held >> 20, HARNESS_MEMORY >> 20);
            std::process::exit(1);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    });
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
    // What its tools answer now, which a `changed` request moves on.
    let answers = std::rc::Rc::new(std::cell::RefCell::new(answers));
    let answering = answers.clone();
    // What the page logged as an error (an uncaught one, a promise nothing handled, what an `<Errored>` caught), as it
    // tells its client (`notifications/message`): the run fails with them, as a browser's console fails a test, but
    // for those `--errors` (a JSON list of words) says a scene shows on purpose, as a page that can't reach its server.
    let expected: Vec<String> = flag("--errors").and_then(|errors| serde_json::from_str(&errors).ok()).unwrap_or_default();
    let threw: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
    let throwing = threw.clone();

    let latency: f64 = flag("--latency").and_then(|ms| ms.parse().ok()).unwrap_or(0.0);
    if let Some(times) = flag("--slower").and_then(|times| times.parse().ok()) {
        renderer::decoding_takes(times);
    }
    let cached: Vec<String> = flag("--cached").map(|tools| tools.split(',').map(str::to_owned).collect()).unwrap_or_default();
    // The answers on their way, each with when it comes on the page's clock (`--latency`).
    let coming: std::rc::Rc<std::cell::RefCell<Vec<(f64, Value)>>> = Default::default();
    let sent = coming.clone();

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
    let clock = flag("--time-zone").map_or_else(Clock::fixed, |zone| Clock::fixed_in(&zone));
    let js = Thread::start(&page, files, None, clock.clone(), touch)?;
    // What the page asks its client, answered from `answers`, as record.js answers it.
    let asked: renderer::Asked = Box::new(move |message, _| {
        if message["method"] == "notifications/message" && message["params"]["level"] == "error" {
            let data = &message["params"]["data"];
            let message = data["message"].as_str().unwrap_or_default();
            if !expected.iter().any(|words| message.contains(words.as_str())) {
                throwing.borrow_mut().push(format!("{message} ({}:{})", data["file"].as_str().unwrap_or("?"), data["line"]));
            }
        }
        let answers = answering.borrow();
        let id = message.get("id")?.clone();
        let (method, params) = (message["method"].as_str().unwrap_or_default(), &message["params"]);
        let answer = match (method, params["name"].as_str()) {
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
        };
        match params["name"].as_str() {
            Some(tool) if method == "tools/call" && latency > 0.0 && !cached.iter().any(|kept| kept == tool) => {
                sent.borrow_mut().push((renderer::platform::fake_time() + latency, answer));
                None
            }
            _ => Some(answer),
        }
    });
    let mut screen = Screen::new(js, Some(asked), Size { width, height }, insets);
    screen.page = tokens_page;
    screen.theme = Some(if dark { Appearance::Dark } else { Appearance::Light });
    screen.touch = touch;
    // A phone's, as an iPhone's host says its person's text size: its body text's 17 pt, which the `phone` tokens are made for.
    screen.text_size = touch.then_some(17.0);
    screen.panel = panel.map(|corner| renderer::Panel { corner, fill: false, whole: false });
    // A picture its page names by its path (Settings' `updates.svg`) is one of its files, read as an app's window
    // fetches it from its app's (`OneCore.Entrypoint` serves them as deployed), a `data:` address, or none where it
    // isn't there.
    let (arrived, pictures) = (screen.arrived.clone(), folder.clone());
    screen.fetch = Some(Box::new(move |src: &str| {
        let data = Url::parse("file:///index.html").and_then(|page| page.join(src)).ok().and_then(|url| {
            let path = pictures.join(url.path().trim_start_matches('/'));
            let kind = match path.extension().and_then(|kind| kind.to_str()).unwrap_or_default() {
                "svg" => "image/svg+xml",
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                _ => "application/octet-stream",
            };
            std::fs::read(path).ok().map(|bytes| format!("data:{kind};base64,{}", data_encoding::BASE64.encode(&bytes)))
        });
        arrived.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((src.to_owned(), data));
    }));
    // With a seed, a page slower than its window: one batch in four of what it changed held back one to three of the
    // window's turns, the window drawing and telling it what it has meanwhile.
    let seed: u64 = flag("--seed").and_then(|seed| seed.parse().ok()).unwrap_or(0);
    if seed != 0 {
        let mut state = seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1;
        let mut random = move |below: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % below
        };
        screen.late = Some(renderer::Late::new(Box::new(move || if random(4) == 0 { 1 + random(3) as u32 } else { 0 })));
    }
    // Its JS runs on a thread of its own, as in a window: each step is drawn once what it set off has run. A panel is
    // summoned as it opens, its page still starting, as a launcher's first summon can come as the app starts.
    if panel.is_none() {
        screen.idle();
    }
    screen.seed(seed);
    // A window of the fake platform's, drawn by the loop every host's windows are (`Windows::turn`).
    let gpu = std::time::Instant::now();
    let resident = || memory_stats::memory_stats().map_or(0, |memory| memory.physical_mem);
    let before = resident();
    let mut windows = Windows::fake(scale, 60.0, clock)?;
    // With a seed, adversarial as a busy device and a person are: refreshes late, input as things move.
    windows.seed(seed);
    let options = WindowOptions { title: "screenshot".into(), app_id: "screenshot".into(), size: Size { width: Pixels(width), height: Pixels(height) }, tag: json!({}), panel, places: None, frame: None, root: false, icon: None, owner: None, snapshot: None };
    let said = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let saying = said.clone();
    // Why its page crashed, if it did, which fails the run: a page never dies quietly.
    let crashed = std::rc::Rc::new(std::cell::RefCell::new(None));
    let crashing = crashed.clone();
    let id = windows.open(options, Ok(screen), move |event| match event {
        // A window would open it in a web view instead.
        Event::Mismatched(why) => saying.borrow_mut().push(format!("it opens in a web view: {why}")),
        Event::Failed(why) => saying.borrow_mut().push(format!("its page didn't start: {why}")),
        Event::Crashed(why) => *crashing.borrow_mut() = Some(why),
        _ => {}
    })?;
    if panel.is_some() {
        windows.summon(id, std::time::Instant::now(), None);
        windows.wait();
    }
    let ran = started.elapsed();
    if let Some(why) = windows.with(id, |screen| screen.didnt_start().map(str::to_owned)).flatten() {
        anyhow::bail!("its page didn't start: {why}");
    }
    // With `--latency`, the page's clock runs as it opens, each answer coming when it's due, until it's had them all:
    // its first content is then, on that clock.
    let opened = renderer::platform::fake_time();
    let mut first_content = None;
    if latency > 0.0 {
        loop {
            windows.run(1000.0 / 60.0);
            let now = renderer::platform::fake_time();
            let due: Vec<Value> = {
                let mut coming = coming.borrow_mut();
                let (due, later) = coming.drain(..).partition(|(at, _)| *at <= now);
                *coming = later;
                due.into_iter().map(|(_, answer)| answer).collect()
            };
            for answer in &due {
                windows.with(id, |screen| screen.message(answer));
            }
            if !due.is_empty() {
                first_content = Some(now - opened);
            }
            if (coming.borrow().is_empty() && windows.with(id, |screen| !screen.starting()).unwrap_or(true)) || now - opened > 5000.0 {
                break;
            }
        }
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
        let request = match renderer::inspect::Request::parse(line) {
            Ok(request) => request,
            Err(refused) => {
                println!("{refused}");
                continue;
            }
        };
        // A server said something changed, as a client tells a page on screen (`notifications/one/changed`), with what
        // its tools answer now (`answers`), as what the network keeps moved on.
        if request.op == "changed" {
            if let Some(now) = request.inputs.get("answers").and_then(Value::as_object) {
                let mut answers = answers.borrow_mut();
                for (tool, answer) in now {
                    answers[tool] = answer.clone();
                }
            }
            windows.with(id, |screen| screen.message(&json!({"jsonrpc": "2.0", "method": "notifications/one/changed"})));
            windows.run(0.0);
            println!("{}", renderer::inspect::answer(&request.id, Ok(json!({}))));
            continue;
        }
        // What a person does comes as the display's input, on the platform's time, as a host's does; with a seed, a
        // moment or two after what came before, so it lands as things are on their way somewhere.
        // As an agent's request reaches a device's window, by the inspector's socket (`"inspector": true`), it goes to
        // the window as such a request does, a press to what it selects, though nobody could, as on a panel put away.
        if INPUT.contains(&request.op.as_str()) && !request.bool("inspector") {
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
    if let Some(why) = crashed.borrow().as_ref() {
        anyhow::bail!("its page crashed: {why}");
    }
    // What the display showed that it mustn't have, last, as its failure.
    let mut wrong = windows.wrong();
    wrong.extend(threw.borrow().iter().map(|error| format!("the page threw: {error}")));
    if !wrong.is_empty() {
        anyhow::bail!("{}", wrong.join("; "));
    }
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
            json!({"first_frame_ms": ms(first_frame_ms), "first_content_ms": ms(first_content), "worst_paint_ms": ms(Some(worst_paint_ms)), "idle_ms": ms(idle), "typing_ms": ms(typing), "scrolling_ms": ms(scrolling), "memory_mb": memory_mb, "window_mb": window_mb})
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
/// moved on with the display ticking through it. On a phone's window (`--touch`), a press is a finger's. And what the
/// platform's keyboard was told (`keyboard`), and what its menu for text and magnifier were asked to show, with the
/// handles drawn (`text_selection`), which only the fake platform knows.
const INPUT: [&str; 16] = ["tap", "down", "up", "key", "scroll", "drag", "replay", "trace", "clock", "wait_for", "away", "put_away", "summon", "back", "keyboard", "text_selection"];

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
    // What isn't there yet, its page busy, is waited for, a second at most, as Playwright's click waits for its locator.
    let started = renderer::platform::fake_time();
    let node = loop {
        let found = ask(windows, id, query.clone()).ok_or_else(|| anyhow::anyhow!("its window didn't answer"))?;
        if let Some(node) = found["result"]["nodes"].get(nth).and_then(|node| node["node"].as_u64()) {
            break node;
        }
        if renderer::platform::fake_time() - started >= 1000.0 {
            anyhow::bail!("no node matches that");
        }
        windows.run(1000.0 / 60.0);
    };
    // Scrolled into sight first where it isn't, as a person scrolls to it, and Playwright's click does, then found
    // again as it's drawn now: what's pressed is what's there, though the page drew it afresh meanwhile.
    if windows.with(id, |screen| screen.reveal(node)).unwrap_or(false) {
        windows.run(0.0);
    }
    // Where it's drawn: what was drawn afresh and isn't yet is waited for, a second at most, as Playwright waits for
    // what it clicks to be there and still, drawn in the same place two frames running, and seen, as a person presses
    // what they see.
    let started = renderer::platform::fake_time();
    let mut was = Value::Null;
    let rect = loop {
        let found = ask(windows, id, query.clone()).ok_or_else(|| anyhow::anyhow!("its window didn't answer"))?;
        let seen = |node: &&Value| node["opacity"].as_f64().is_some_and(|opacity| opacity > 0.0);
        if let Some(rect) = found["result"]["nodes"].get(nth).filter(seen).map(|node| node["rect"].clone()).filter(Value::is_object) {
            if rect == was {
                break rect;
            }
            was = rect;
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
            press(windows, id, x, y, Phase::Moved);
            windows.run(0.0);
            // With a seed, held as long as a person might: from a quick tap to a quarter of a second.
            let held = request.f64("hold").unwrap_or(CLICK_MS + windows.random(170) as f64 - windows.random(50) as f64);
            for _ in 0..clicks {
                match request.bool("shift") {
                    true => windows.pointer_down(id, x, y, 0, true),
                    false => press(windows, id, x, y, Phase::Down),
                }
                windows.run(held);
                press(windows, id, x, y, Phase::Up);
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
        // The app goes to the background for `ms` and comes back, as a phone's does (`Windows::away`): `drawn` is how
        // many frames it drew meanwhile, which a phone's GPU refuses.
        "away" => Ok(json!({"drawn": windows.away(request.f64("ms").unwrap_or(1000.0))})),
        // A panel (`--panel`) put away, as its shortcut or a click outside it does a launcher, and summoned again.
        "put_away" => {
            windows.put_away(id, false);
            windows.run(0.0);
            Ok(json!({}))
        }
        "summon" => {
            windows.summon(id, web_time::Instant::now(), None);
            windows.run(0.0);
            Ok(json!({}))
        }
        // The device's Back, as Android's Back key is (`Windows::back`): what's open over the page shuts, or the window
        // leaves; `went` is whether it took it.
        "back" => {
            let went = windows.back(id);
            windows.run(0.0);
            Ok(json!({"went": went}))
        }
        // What the input with the keyboard asked the platform's keyboard for (Flutter's `TextInputConfiguration`:
        // `textCapitalization`, `autocorrect`, `inputType`…), as a phone's keyboard is told; null where none has it.
        "keyboard" => Ok(json!({"keyboard": windows.keyboard(id)})),
        // What the system's menu for text was asked to show (its items, what it's for and where the system put it),
        // where its magnifier shows, and the handles drawn, as a phone's are, with the window's safe area.
        "text_selection" => Ok(windows.text_selection(id)),
        // As Playwright's `locator.waitFor` and its assertions do: what's selected comes to be `state` (`shown`: drawn
        // whole and in the window; `gone`: not there; `focused`: with the keyboard; `there`: there at all; `drawn`: at
        // `opacity`), with its
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
                    // Drawn at `opacity` (within a hundredth), as its look says it is now.
                    "drawn" => node.is_some_and(|node| node["opacity"].as_f64().zip(request.f64("opacity")).is_some_and(|(drawn, want)| (drawn - want).abs() <= 0.01)),
                    _ => node.is_some_and(|node| node["opacity"].as_f64().is_some_and(|opacity| opacity >= 0.99) && node["seen"].as_f64().is_some_and(|seen| seen >= 0.99) && states(node)),
                };
                let waited = renderer::platform::fake_time() - started;
                if met || waited >= timeout {
                    return Ok(json!({"met": met, "ms": waited, "node": node}));
                }
                // A hair over a frame, as the display's refresh is a hair over 1000/60 ms, so a frame is always drawn
                // between one look and the next.
                windows.run(1000.0 / 60.0 * 1.01);
            }
        }
        // A button held down where `tap` would press it, and let go where it was held, for what shows mid-press.
        "down" => {
            let (x, y) = point_of(windows, id, request)?;
            press(windows, id, x, y, Phase::Moved);
            match request.bool("shift") {
                true => windows.pointer_down(id, x, y, 0, true),
                false => press(windows, id, x, y, Phase::Down),
            }
            windows.run(0.0);
            Ok(json!({"down": {"x": x, "y": y}}))
        }
        "up" => {
            let (x, y) = PRESSED.get().ok_or_else(|| anyhow::anyhow!("nothing is held down"))?;
            press(windows, id, x, y, Phase::Up);
            windows.run(0.0);
            Ok(json!({"up": {"x": x, "y": y}}))
        }
        // A trace of input as a device delivered it (`events`, each at its `ms` from the first): a trackpad's scrolls
        // with their phases and momentum, the pointer's moves, presses and keys, played as the display's events at their
        // own times, then `after` ms (1000 unless said) to come to rest.
        "trace" => {
            let events = request.inputs.get("events").and_then(Value::as_array).cloned().ok_or_else(|| anyhow::anyhow!("say what it plays: `events`, each {{ms, kind, …}}"))?;
            let begun = renderer::platform::fake_time();
            for event in &events {
                let ms = event["ms"].as_f64().unwrap_or(0.0);
                windows.run((begun + ms - renderer::platform::fake_time()).max(0.0));
                deliver(windows, id, event)?;
            }
            windows.run(request.f64("after").unwrap_or(1000.0));
            Ok(json!({"played": events.len()}))
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
            press(windows, id, x, y, Phase::Moved);
            press(windows, id, x, y, Phase::Down);
            for step in 1..=steps {
                windows.run(ms / steps as f64);
                let part = step as f32 / steps as f32;
                press(windows, id, x + dx * part, y + dy * part, Phase::Moved);
            }
            press(windows, id, x + dx, y + dy, Phase::Up);
            windows.run(0.0);
            Ok(json!({"dragged": {"x": dx, "y": dy}}))
        }
        _ => replay(windows, id, request),
    }
}

/// The inspector's `replay`, as the display's events: the press goes down, moves along `moves` (`{dx, dy, ms}`,
/// sampled `input_hz` times a second) and lifts unless `lift` is false, or a trace's `events` (as `trace` takes them), on a display of `hz`, each of its frames
/// reported as `replay` reports one (`renderer::inspect`'s), until `after` ms past its end or nothing moves.
fn replay(windows: &mut Windows, id: renderer::platform::WindowId, request: &renderer::inspect::Request) -> anyhow::Result<Value> {
    // A trace's own events (`events`, as `trace` takes them), or a press along `moves`.
    let traced = request.inputs.get("events").and_then(Value::as_array).cloned();
    let (x, y) = match &traced {
        Some(events) if !request.selects() && request.f32("x").is_none() => events.first().map_or((0.0, 0.0), |first| (first["x"].as_f64().unwrap_or(0.0) as f32, first["y"].as_f64().unwrap_or(0.0) as f32)),
        _ => point_of(windows, id, request)?,
    };
    let moves: Vec<(f32, f32, f64)> = request.inputs.get("moves").and_then(Value::as_array).or(traced.as_ref().map(|_| &EMPTY)).ok_or_else(|| anyhow::anyhow!("say how it moves: `moves`, each {{dx, dy, ms}}, or its `events`"))?
        .iter()
        .map(|one| (one["dx"].as_f64().unwrap_or(0.0) as f32, one["dy"].as_f64().unwrap_or(0.0) as f32, one["ms"].as_f64().unwrap_or(0.0).max(0.0)))
        .collect();
    let lift = request.inputs.get("lift").and_then(Value::as_bool) != Some(false);
    let hz = request.f64("hz").unwrap_or(60.0).clamp(1.0, 480.0);
    let every = 1000.0 / request.f64("input_hz").unwrap_or(hz).clamp(1.0, 1000.0);
    let after = request.f64("after").unwrap_or(2000.0).max(0.0);
    let snaps = renderer::inspect::snaps(request);
    let mut track = match request.inputs.get("track") {
        Some(Value::Object(selector)) => {
            let mut query = json!({"op": "query"});
            for (name, value) in selector {
                query[name] = value.clone();
            }
            Some(query)
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
    let samples: Vec<(f64, Value)> = match &traced {
        Some(events) => events.iter().map(|event| (event["ms"].as_f64().unwrap_or(0.0), event.clone())).collect(),
        None => {
            let mut events = vec![(0.0, json!({"kind": "move", "x": x, "y": y})), (0.0, json!({"kind": "down", "x": x, "y": y}))];
            events.extend((1..=(length / every).ceil() as usize).map(|n| (n as f64 * every).min(length)).map(|t| (t, at(t))).map(|(t, (mx, my))| (t, json!({"kind": "move", "x": mx, "y": my}))));
            if lift {
                let (ux, uy) = at(length);
                events.push((length, json!({"kind": "up", "x": ux, "y": uy})));
            }
            events
        }
    };
    let length = samples.last().map_or(0.0, |(t, _)| *t);
    windows.set_refresh(hz);
    // With `frame_ms`, each frame takes that long to draw, as on a device others load, which presses it as a window's
    // late frames do.
    windows.set_frame_ms(request.f64("frame_ms").unwrap_or(0.0));
    let refresh = 1000.0 / hz;
    let begun = renderer::platform::fake_time();
    let (mut next, mut frames, mut drawn) = (0, Vec::new(), windows.frames(id));
    // Its first frame's cost is what's done from here.
    windows.cost(id);
    let elapsed = || renderer::platform::fake_time() - begun;
    loop {
        // The display's next refresh, and each move before it at its own time, as a device delivers them.
        // At least a microsecond on, so a clock a hair short of a refresh isn't moved by nothing.
        let now = renderer::platform::fake_time();
        let boundary = ((now / refresh).floor() + 1.0) * refresh;
        let tick = if boundary - now < 0.001 { boundary + refresh } else { boundary } - begun;
        while let Some((t, event)) = samples.get(next).filter(|(t, _)| *t <= tick) {
            windows.run((t - elapsed()).max(0.0));
            deliver(windows, id, event)?;
            next += 1;
        }
        // What the display shows at a refresh is the frame drawn last before it (on the refresh before, or at once as
        // input came since), so each is read just before its refresh, and the refresh then taken.
        windows.run((tick - JUST_BEFORE - elapsed()).max(0.0));
        let drew = windows.frames(id) > drawn;
        drawn = windows.frames(id);
        // What it cost since the frame before, every frame drawn meanwhile and its JS, which a frame that drew nothing
        // costs too.
        let mut frame = windows.cost(id).unwrap_or_default();
        if !drew && let Some(frame) = frame.as_object_mut() {
            frame.remove("gpu_ms");
        }
        if !drew {
            frame["draws"] = json!(0);
            frame["work"] = json!({"quads": 0, "shadows": 0, "sprites": 0, "backdrops": 0, "rasters": 0, "copied": 0, "uploaded": 0});
        }
        frame["at"] = json!((tick * 100.0).round() / 100.0);
        frame["pressure"] = json!(windows.pressure());
        frame["drawn"] = json!(drew);
        // What's tracked, where it's drawn, or that it's gone, as a page that's left takes it with it: the node the
        // selector first found, not another it finds once that's gone (an App Library's Notes, as Home's goes).
        if let Some(query) = &mut track {
            if query.get("node").is_none()
                && let Some(node) = ask(windows, id, query.clone()).and_then(|found| found["result"]["nodes"][0]["node"].as_u64())
            {
                *query = json!({"op": "query", "node": node});
            }
            match ask(windows, id, query.clone()).and_then(|found| found["result"]["nodes"][0]["rect"].as_object().cloned()) {
                Some(rect) => {
                    frame["x"] = rect["x"].clone();
                    frame["y"] = rect["y"].clone();
                }
                None => frame["gone"] = json!(true),
            }
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
        windows.run((tick - elapsed()).max(0.0));
        let still = !windows.wanted();
        if next == samples.len() && (still || tick >= length + after) {
            break;
        }
    }
    windows.set_refresh(60.0);
    windows.set_frame_ms(0.0);
    // What's been taken out but is still drawn going, which `after` should have seen go.
    let leaving = ask(windows, id, json!({"op": "window"})).and_then(|answer| answer["result"]["leaving"].as_u64()).unwrap_or(0);
    let budget = refresh;
    let over = renderer::inspect::over(&frames, budget);
    Ok(json!({"hz": hz, "budget_ms": (budget * 100.0).round() / 100.0, "over": over, "leaving": leaving, "frames": frames, "time": renderer::platform::fake_time(), "paused": true}))
}

/// No moves: a trace's replay has its events instead.
static EMPTY: Vec<Value> = Vec::new();

/// One event of a trace, as the display's (`trace`'s `events`): a wheel's or a trackpad's scroll (`dx`, `dy`, its
/// `phase` and `momentum`), the pointer moved, a button down or up (a finger's on a phone's window), a finger
/// (`touch`, its `phase`, `down`, `moved`, `up` or `cancelled`, and which `finger`, as a phone's screen says it), each
/// at `x`, `y`, or a `key`.
fn deliver(windows: &mut Windows, id: renderer::platform::WindowId, event: &Value) -> anyhow::Result<()> {
    let at = |name: &str| event[name].as_f64().unwrap_or(0.0) as f32;
    let (x, y) = (at("x"), at("y"));
    match event["kind"].as_str().unwrap_or_default() {
        "wheel" => windows.wheel(id, (x, y), (at("dx"), at("dy")), event["phase"].as_str(), event["momentum"].as_bool().unwrap_or(false)),
        "move" => press(windows, id, x, y, Phase::Moved),
        "down" => press(windows, id, x, y, Phase::Down),
        "up" => press(windows, id, x, y, Phase::Up),
        "touch" => {
            let phase = event["phase"].as_str().and_then(Phase::named).ok_or_else(|| anyhow::anyhow!("say a touch's phase: down, moved, up or cancelled"))?;
            windows.finger(id, event["finger"].as_u64().unwrap_or(0), x, y, phase);
        }
        "key" => {
            let key = renderer::inspect::Request::parse(&json!({"v": renderer::inspect::VERSION, "op": "key", "key": event["key"]}).to_string()).map_err(|refused| anyhow::anyhow!("{refused}"))?;
            windows.key(id, &key)?;
        }
        kind => anyhow::bail!("there's no kind of event {kind}: wheel, move, down, up, touch and key are"),
    }
    Ok(())
}

thread_local! {
    /// Where the main button or the finger is held down, if it is, for `up`.
    static PRESSED: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
}

/// A press at `x`, `y`: on a phone's window (`--touch`) a finger's, as its screen says it; elsewhere the main button's,
/// a move the pointer's.
fn press(windows: &mut Windows, id: renderer::platform::WindowId, x: f32, y: f32, phase: Phase) {
    match phase {
        Phase::Down | Phase::Moved if PRESSED.get().is_some() || phase == Phase::Down => PRESSED.set(Some((x, y))),
        Phase::Up | Phase::Cancelled => PRESSED.set(None),
        _ => {}
    }
    if windows.with(id, |screen| screen.touch).unwrap_or(false) {
        return windows.finger(id, 0, x, y, phase);
    }
    match phase {
        Phase::Down => windows.pointer_down(id, x, y, 0, false),
        Phase::Moved => windows.pointer_moved(id, x, y),
        Phase::Up | Phase::Cancelled => windows.pointer_up(id, x, y, 0),
    }
}

/// How long before a refresh what the display shows at it is read: what's drawn after is shown at the next.
const JUST_BEFORE: f64 = 0.01;
