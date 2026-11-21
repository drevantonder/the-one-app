//! A native page drawn offscreen, the same on every run: a PNG of it and its semantic tree as JSON.
//!
//!     cargo run -p renderer --example screenshot --features fake -- <page's built folder> <out.png> <out.json> \
//!       [--ui core/ui/dist/client] [--answers answers.json] [--size 480x360] [--scale 2] [--dark] [--requests <file>|-]
//!       [--safe-area <top>,<right>,<bottom>,<left>] [--touch] [--reduced-motion] [--panel <corner>] [--measure] [--seed <n>]
//!       [--latency <ms> [--cached <tool>,…]] [--slower <times>] [--memory <GB>] [--time-zone <IANA name>] [--hz <n>]
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
//! stdout, a frame drawn before each: `{"v":1,"id":1,"op":"tap","text":"Backups"}`. What a person does (`tap`, `hover`, `key`,
//! `scroll`, `drag`, `replay`, `away`, the app going to the background and back, a panel's `put_away` and `summon`, the
//! window's `hide`, as a phone's root window under an app, the device's `back`, the system's `appearance` switched and
//! the window's `resize`)
//! comes as the display's input, each event at its own time, a click held down 80 ms,
//! and what it presses scrolled into sight first, as Playwright's click does. Its clock is paused: `clock`'s `step`
//! moves it, the display ticking each frame meanwhile; and after the requests, until what it shows comes to rest but
//! for loops (at most 3 s), which is what's pictured. `changed` is a server saying something changed, as its client
//! tells a page on screen, with what its tools answer from then on (`answers`, as `--answers` has them). A run whose
//! page crashed fails, saying why, as does one whose window was still blank 5 s after it opened (`Event::Blank`), and so does one whose process grows past `HARNESS_MEMORY`, at once, wherever it is. `--touch` draws it for a finger, as a phone's window is (`pointer: coarse`),
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
//! of its own (`Thread::seed`), and one picture in three fails to decode the first time, as a browser's under load.
//! `--latency` has each tool's answer come that many milliseconds of the page's clock after it was asked, as a round
//! trip to a server takes, but for the tools `--cached` names, which a client answers at once from what it keeps
//! (a tool that only reads): so a page that opens on what isn't kept, or asks again for what an answer said, opens
//! slowly here as on a device. The page's clock runs as it opens until it's had every answer, at most 5 s; one due
//! later comes at its time as the clock moves, however it's moved, as a server's answer does.
//! `--slower` has each picture take that many times as long to decode as on this machine, as on a device with a slower
//! CPU, where they come later after the page asks for them: the inspector's `window` says the longest one took
//! (`picture_ms`).
//! `--memory` gives its page the memory it would have on a device with that many GB, as a phone of 4 has 256 MB
//! (`renderer::js::memory_limit`), so one that runs away runs out as soon as it would there.
//! `--time-zone` has the page's clock show times in that time zone, as a device in it does: the clock is `UTC` otherwise.
//! `--hz` is how often its display refreshes from the start, as a device's says before its first frame (60 unless it
//! says): a `replay` at another rate plays at that one, then the display's is its own again.
//! `--measure` says, on stderr after the rest, how quickly it draws, as a line `measured {…}`: its first
//! frame (`first_frame_ms`, from the GPU's start to the frame read back), a frame's time when nothing changes
//! (`idle_ms`), as its first text field takes a character a frame (`typing_ms`) and as its first list scrolls
//! (`scrolling_ms`), each where it has one, each the loop's whole turn as a window takes it, the longest turn of all
//! it drew (`worst_paint_ms`: laying it out, making its scene and handing it to the GPU, which a picture's decoding,
//! in the background, isn't part of), the memory drawing took (`memory_mb`: the GPU, its atlas and the
//! renderer, as the process's resident set grew), and what a second window of its size adds (`window_mb`: a renderer
//! of its own, drawing the same frame, the GPU and the atlas shared).

mod between;
mod input;
mod page;
mod replay;
mod select;

use between::{BETWEEN, FRAMES, between};
use input::{INPUT, input};
use page::{answered, asked_of, built, framed, local};
use select::ask;
use std::path::PathBuf;

use serde_json::{Value, json};
use renderer::geometry::{DevicePixels, Pixels, Size};
use renderer::js::{Clock, Thread};
use renderer::platform::{Event, WindowOptions, Windows};
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
    // The system's Reduce Motion is on, as a person's setting makes it.
    renderer::platform::fake_reduce(args.iter().any(|arg| arg == "--reduced-motion"));
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
    // Each call the page made, by what it was (`asked_of`), for an `asks` request.
    let calls: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
    let calling = calls.clone();
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
    if let Some(gb) = flag("--memory").and_then(|gb| gb.parse::<f64>().ok()) {
        renderer::js::device_memory_is((gb * f64::from(1u32 << 30)) as usize);
    }
    let cached: Vec<String> = flag("--cached").map(|tools| tools.split(',').map(str::to_owned).collect()).unwrap_or_default();
    // The answers on their way, each with when it comes on the page's clock (`--latency`).
    let coming: std::rc::Rc<std::cell::RefCell<Vec<(f64, Value)>>> = Default::default();
    let sent = coming.clone();

    let folder = std::fs::canonicalize(folder)?;
    let ui = flag("--ui").map(PathBuf::from);
    let (page, files, tokens_page) = built(&folder, ui.clone())?;
    // The app the Home Screen's page opens over it as a phone's app does (`ui/open-link`), from where its icon is:
    // `--open <app>=<its page's built folder>`, with its window's snapshot (`--snapshot <stem>`, as
    // `WindowOptions::snapshot` names one) where it has one.
    let opening: Option<(String, PathBuf)> = flag("--open").and_then(|open| open.split_once('=').map(|(app, folder)| (app.to_owned(), PathBuf::from(folder))));
    let snapshot = flag("--snapshot").map(PathBuf::from);
    let opens: std::rc::Rc<std::cell::RefCell<Vec<Value>>> = Default::default();
    let opens_to = opens.clone();
    let opened_app = opening.as_ref().map(|(app, _)| app.clone());

    let started = std::time::Instant::now();
    let clock = flag("--time-zone").map_or_else(Clock::fixed, |zone| Clock::fixed_in(&zone));
    let js = Thread::start(&page, files, None, clock.clone(), touch)?;
    // What the page asks its client, answered from `answers`, as record.js answers it.
    let (page_folder, page_ui) = (folder.clone(), ui.clone());
    let asked: renderer::Asked = Box::new(move |message, _| {
        if message["method"] == "notifications/message" && message["params"]["level"] == "error" {
            let data = &message["params"]["data"];
            let message = data["message"].as_str().unwrap_or_default();
            if !expected.iter().any(|words| message.contains(words.as_str())) {
                // Where it threw, in the page's source, by its bundles' sourcemaps beside them, as a client tells Telemetry.
                let stack = data["stack"].as_str().unwrap_or_default();
                let maps = renderer::js::stack_files(stack).into_iter().filter_map(|file| Some((file, std::fs::read(local(&page_folder, page_ui.as_deref(), &Url::parse(&format!("{file}.map")).ok()?)).ok()?))).collect();
                let place = renderer::js::source_mapped(stack, &maps).1.map(|place| format!("{}:{}", place.file, place.line));
                throwing.borrow_mut().push(format!("{message} ({})", place.unwrap_or_else(|| format!("{}:{}", data["file"].as_str().unwrap_or("?"), data["line"]))));
            }
        }
        let answers = answering.borrow();
        let id = message.get("id")?.clone();
        let (method, params) = (message["method"].as_str().unwrap_or_default(), &message["params"]);
        if method == "ui/open-link" && params["url"].as_str().and_then(|url| url.strip_prefix("one:")).zip(opened_app.as_deref()).is_some_and(|(url, app)| url.starts_with(app)) {
            if params["behind"] != true {
                opens_to.borrow_mut().push(params["from"].clone());
            }
            return Some(json!({"jsonrpc": "2.0", "id": id, "result": {}}));
        }
        let answer = answered(&answers, id, method, params);
        if let (true, Some(tool)) = (method == "tools/call", params["name"].as_str()) {
            calling.borrow_mut().push(asked_of(params).unwrap_or_else(|| tool.to_owned()));
        }
        match params["name"].as_str() {
            Some(tool) if method == "tools/call" && latency > 0.0 && !cached.iter().any(|kept| kept == tool) => {
                sent.borrow_mut().push((renderer::platform::fake_time() + latency, answer));
                None
            }
            _ => Some(answer),
        }
    });
    let mut screen = Screen::new(js, Some(asked));
    // Another app's page in a frame of its own (`Frame`), its app's client built beside the page at `<app>/client/`,
    // as a scene's `uses` has it: started between the display's frames and handed to the frame, as a native host
    // starts one on its own thread and hands it over (`clients/app/src/native`), its tools answered from `answers`.
    let framing: std::rc::Rc<std::cell::RefCell<Vec<(renderer::tree::Id, String)>>> = Default::default();
    let asking = framing.clone();
    screen.make_frame = Some(Box::new(move |node, src, _| asking.borrow_mut().push((node, src.to_owned()))));
    screen.insets = insets;
    screen.page = tokens_page;
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
    screen.seed(seed);
    // A window of the fake platform's, drawn by the loop every host's windows are (`Windows::turn`).
    let gpu = std::time::Instant::now();
    let resident = || memory_stats::memory_stats().map_or(0, |memory| memory.physical_mem);
    let before = resident();
    let hz: f64 = flag("--hz").and_then(|hz| hz.parse().ok()).unwrap_or(60.0);
    let mut windows = Windows::fake(scale, hz, clock.clone())?;
    windows.prepared();
    // `--dark` is the system's, as a phone's or a computer's Dark mode is, which the app follows, so it can change.
    windows.appearance(if dark { Appearance::Dark } else { Appearance::Light });
    // A phone's window (`--touch`) is the whole of its screen, as every window there but a panel is.
    if touch && panel.is_none() {
        windows.phone(Size { width: Pixels(width), height: Pixels(height) });
    }
    // With a seed, adversarial as a busy device and a person are: refreshes late, input as things move.
    windows.seed(seed);
    // A page that opens another app over it is a phone's Home Screen, its root window.
    let options = WindowOptions { title: "screenshot".into(), app_id: "screenshot".into(), size: Size { width: Pixels(width), height: Pixels(height) }, tag: json!({}), panel, places: None, frame: None, root: opening.is_some(), icon: None, owner: None, snapshot: None, title_row: false };
    let said = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let saying = said.clone();
    // Why its page crashed, if it did, which fails the run: a page never dies quietly.
    let crashed = std::rc::Rc::new(std::cell::RefCell::new(None));
    let crashing = crashed.clone();
    // Why its window was blank a while after it opened, if it was (`Event::Blank`), which fails it too.
    let blank = std::rc::Rc::new(std::cell::RefCell::new(None));
    let blanking = blank.clone();
    let id = windows.open(options, Ok(screen), move |event| match event {
        // A window would open it in a web view instead.
        Event::Mismatched(why) => saying.borrow_mut().push(format!("it opens in a web view: {why}")),
        Event::Failed(why) => saying.borrow_mut().push(format!("its page didn't start: {why}")),
        Event::Crashed(why) => *crashing.borrow_mut() = Some(why),
        Event::Blank(why) => *blanking.borrow_mut() = Some(why),
        _ => {}
    })?;
    // Each answer on its way (`--latency`) comes at its own time as the clock moves, however it's moved, as a server's
    // does, what it sets off going out then; one still on its way is a request `settle` waits for.
    windows.answering(id, coming.clone());
    {
        let (folder, ui, clock, answers) = (folder.clone(), ui.clone(), clock.clone(), answers.clone());
        FRAMES.set(Some(Box::new(move |windows: &mut Windows| {
            for (node, src) in framing.borrow_mut().drain(..) {
                let page = src.split_once('#').map_or(src.as_str(), |(page, _)| page);
                let made = framed(&folder, page, ui.clone(), clock.clone(), touch, answers.clone());
                windows.with(id, |parent| match made {
                    Ok(screen) => parent.framed(node, &src, screen),
                    Err(error) => parent.failed = Some(format!("its frame's page, {src}, didn't start: {error:#}")),
                });
            }
        })));
    }
    // Its JS runs on a thread of its own, as in a window: each step is drawn once what it set off has run. A panel is
    // summoned as it opens, its page still starting, as a launcher's first summon can come as the app starts.
    if panel.is_some() {
        windows.summon(id, std::time::Instant::now(), None);
    }
    windows.wait();
    // The app `--open` names, opened over the Home Screen as its page asks, from its icon, as a phone's app opens an
    // app's window (`clients/app/src/native.rs`): the size it asks for is a computer's, which a phone's window isn't,
    // and its tools answer `--latency` ms after they're asked, as the first page's do.
    if let Some((app, built_in)) = opening {
        let (page, files, tokens_page) = built(&built_in, ui.clone())?;
        let mut files = Some(files);
        let coming: std::rc::Rc<std::cell::RefCell<Vec<(f64, Value)>>> = Default::default();
        let mut opened: Option<renderer::platform::WindowId> = None;
        let clock = clock.clone();
        let answers = answers.clone();
        BETWEEN.set(Some(Box::new(move |windows: &mut Windows| {
            let now = renderer::platform::fake_time();
            if let Some(opened) = opened {
                let due: Vec<Value> = {
                    let mut coming = coming.borrow_mut();
                    let (due, later) = coming.drain(..).partition(|(at, _)| *at <= now);
                    *coming = later;
                    due.into_iter().map(|(_, answer)| answer).collect()
                };
                for answer in &due {
                    windows.with(opened, |screen| screen.message(answer));
                }
            }
            let Some(from) = opens.borrow_mut().pop() else { return };
            let Some(files) = files.take() else { return };
            let Ok(js) = Thread::start(&page, files, None, clock.clone(), touch) else { return };
            let (answers, sent) = (answers.clone(), coming.clone());
            let asked: renderer::Asked = Box::new(move |message, _| {
                let id = message.get("id")?.clone();
                let (method, params) = (message["method"].as_str().unwrap_or_default(), &message["params"]);
                let answers = answers.borrow();
                let answer = answered(&answers, id, method, params);
                if method == "tools/call" && latency > 0.0 {
                    sent.borrow_mut().push((renderer::platform::fake_time() + latency, answer));
                    return None;
                }
                Some(answer)
            });
            let mut screen = Screen::new(js, Some(asked));
            screen.page = tokens_page;
                    screen.touch = touch;
            screen.text_size = touch.then_some(17.0);
            let at = |edge: &str| from[edge].as_f64().unwrap_or_default() as f32;
            let icon = from.is_object().then(|| renderer::geometry::Bounds { origin: renderer::geometry::point(at("x"), at("y")), size: Size { width: at("width"), height: at("height") } });
            let options = WindowOptions { title: app.clone(), app_id: "screenshot".into(), size: Size { width: Pixels(1024.0), height: Pixels(720.0) }, tag: json!({"entrypoint": app}), panel: None, places: None, frame: None, root: false, icon, owner: None, snapshot: snapshot.clone(), title_row: false };
            // Its window first, its page given it once started, as the app opens one (`Windows::opening`).
            opened = windows.opening(options, |_| {}).ok();
            if let Some(id) = opened {
                windows.run(1000.0 / 60.0);
                windows.attach(id, screen);
            }
        })));
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
            if coming.borrow().is_empty() && windows.with(id, |screen| !screen.starting()).unwrap_or(true) {
                first_content = Some(now - opened);
                break;
            }
            if now - opened > 5000.0 {
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
            between(windows);
            windows.wait();
            windows.refresh_next();
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
        // What the page has asked its client so far, each call as `asked_of` names it, as a check counts them.
        if request.op == "asks" {
            println!("{}", renderer::inspect::answer(&request.id, Ok(json!({"asks": *calls.borrow()}))));
            continue;
        }
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
            // On a phone's screen, it's the window on top's, an app's over the Home Screen, as a finger lands on it.
            let top = windows.top().unwrap_or(id);
            let answered = input(&mut windows, top, &request);
            println!("{}", renderer::inspect::answer(&request.id, answered));
            continue;
        }
        let deadline = request.f64("deadline").unwrap_or(5000.0);
        let tap = request.op == "tap";
        let answered = std::rc::Rc::new(std::cell::RefCell::new(None));
        let to = answered.clone();
        windows.inspect(id, request, Box::new(move |answer| *to.borrow_mut() = Some(answer)));
        windows.wait();
        windows.turn();
        // A `settle` not settled at once is told so at its deadline, as the loop waits for what's due; a tap waiting for
        // what it presses (`Screen::tap_ready`) is told once it's pressed it, its time running meanwhile, a frame at a
        // time, as a device's does while an agent waits, where `settle` says what's pending on a paused clock.
        let asked = std::time::Instant::now();
        while answered.borrow().is_none() && asked.elapsed().as_secs_f64() * 1000.0 < deadline + 1000.0 {
            if tap {
                windows.run(1000.0 / 60.0);
                continue;
            }
            std::thread::sleep(windows.due().unwrap_or(std::time::Duration::from_millis(10)).min(std::time::Duration::from_millis(100)));
            // The display refreshes meanwhile, as a device's does, its clock still: what the page posted since the last
            // frame (an answer its JS thread ran a moment after it) is drawn, not left changed and undrawn.
            windows.wait();
            windows.tick();
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
    while (windows.revealing(id) || windows.with(id, |screen| screen.arriving()).unwrap_or(false) || !coming.borrow().is_empty()) && renderer::platform::fake_time() - settling < SETTLE_MS {
        between(&mut windows);
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
    if let Some(why) = blank.borrow().as_ref() {
        anyhow::bail!("its window was blank: {why}");
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
        second.draw(scene, &[[0.0, 0.0]]);
        second.read(0);
        let window_mb = mb(resident().saturating_sub(alone));
        second.destroy();
        eprintln!(
            "measured {}",
            json!({"first_frame_ms": ms(first_frame_ms), "first_content_ms": ms(first_content), "worst_paint_ms": ms(Some(worst_paint_ms)), "idle_ms": ms(idle), "typing_ms": ms(typing), "scrolling_ms": ms(scrolling), "memory_mb": memory_mb, "window_mb": window_mb})
        );
    }
    Ok(())
}

/// The longest it's run for, at the end, for what it shows to come to rest.
const SETTLE_MS: f64 = 3000.0;
