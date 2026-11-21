//! A page in a real window of this computer's, drawn and presented on its display as the app's windows are, with a
//! wheel's notches played on it, and each motion's frames as the app's log says them (`Event::Frames`), as JSON lines:
//! how long apart they came, and each part of a frame's 95th percentile, its painting, drawing, waiting for an image,
//! submitting and presenting. It exits 1 where a motion's frames took longer than a refresh of a `--hz` display (60
//! unless it says) at their 95th percentile: this thread's painting and drawing, or the GPU's. How far apart they came
//! isn't judged: a wheel's notches each move it at once, so its frames come as often as the notches do.
//!
//!     cargo run -p renderer --example window -- <page's built folder> --trace <trace.json> \
//!       [--ui core/ui/dist/client] [--answers answers.json] [--size 1100x720] [--rounds 3] [--panel <corner>]
//!       [--before <requests.jsonl>] [--hz 60]
//!
//! on a desktop: X11 (`xvfb-run -a` where there's none), or Windows in its person's session (over ssh, a scheduled
//! task's `/IT`, as the qa skill starts the app), where Direct3D 12 presents through DirectComposition as the app does.
//! The headless harness (`screenshot`) draws offscreen and never presents, so what presenting costs shows only here.
//! A trace is a scene's (`apps/<app>/client/traces/`): its `wheel` events, played at their times, each `rounds` times.
//! `--before` gives the inspector's requests run first, a second apart (a scene's `taps`, `{"op":"tap","name":"…"}`).
//! `--panel` opens it as a launcher's panel, summoned, its corners `<corner>` round: each pane of the system's glass its
//! page asks for a window of its own, drawn by a renderer of its own (`Display::arrange`), as the app's launcher.

use std::path::PathBuf;
use std::sync::mpsc;

use renderer::geometry::{Pixels, Size};
use renderer::js::{Clock, Files, Thread};
use renderer::platform::{Event, Native, WindowOptions};
use renderer::{Appearance, Screen};
use serde_json::{Value, json};
use url::Url;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().position(|arg| arg == name).and_then(|at| args.get(at + 1)).cloned();
    let folder = std::fs::canonicalize(args.first().ok_or_else(|| anyhow::anyhow!("window <page's built folder> --trace <trace.json> …"))?)?;
    let trace: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(flag("--trace").ok_or_else(|| anyhow::anyhow!("say what to play: --trace"))?)?)?;
    let (width, height) = flag("--size").and_then(|size| size.split_once('x').map(|(w, h)| (w.parse().unwrap_or(1100.0), h.parse().unwrap_or(720.0)))).unwrap_or((1100.0, 720.0));
    let rounds: usize = flag("--rounds").and_then(|rounds| rounds.parse().ok()).unwrap_or(3);
    let panel: Option<f32> = flag("--panel").and_then(|corner| corner.parse().ok());
    let budget = 1000.0 / flag("--hz").and_then(|hz| hz.parse::<f64>().ok()).unwrap_or(60.0);
    let answers: Value = match flag("--answers") {
        Some(answers) => serde_json::from_str(&std::fs::read_to_string(answers)?)?,
        None => json!({}),
    };
    let ui = flag("--ui").map(PathBuf::from);

    let native = Native::get()?;
    let (said_to, said) = mpsc::channel::<Option<Value>>();
    let (opened_to, opened) = mpsc::channel();
    native.run(move |windows| {
        let options = WindowOptions { title: "Window check".into(), app_id: "window-check".into(), size: Size { width: Pixels(width), height: Pixels(height) }, tag: json!({}), panel, places: None, frame: None, root: false, icon: None, owner: None, snapshot: None, title_row: false };
        let to = said_to.clone();
        let screen = screen(folder, ui, answers, panel).map_err(|error| format!("{error:#}"));
        let id = windows.open(options, screen, move |event| match event {
            Event::Drawn => drop(to.send(None)),
            Event::Frames(frames) => {
                let ms = |time: std::time::Duration| (time.as_secs_f64() * 100_000.0).round() / 100.0;
                let said = json!({
                    "kind": frames.kind, "frames": frames.frames, "took_ms": ms(frames.took), "refresh_ms": ms(frames.refresh),
                    "p50_ms": ms(frames.p50), "p95_ms": ms(frames.p95), "longest_ms": ms(frames.longest), "long": frames.long,
                    "paint_ms": ms(frames.paint), "draw_ms": ms(frames.draw), "acquire_ms": ms(frames.acquire),
                    "submit_ms": ms(frames.submit), "present_ms": ms(frames.present), "gpu_ms": frames.gpu.map(ms),
                    "passes": frames.passes, "most_passes": frames.most_passes,
                });
                let _ = to.send(Some(said));
            }
            _ => {}
        });
        if let (Ok(id), Some(_)) = (&id, panel) {
            windows.summon(*id, std::time::Instant::now(), None);
        }
        let _ = opened_to.send(id.map_err(|error| format!("{error:#}")));
    });
    let id = opened.recv()?.map_err(anyhow::Error::msg)?;
    // Its first frame, then its page's first answers settled, before anything's played; and what's to be done first.
    // Settled is two seconds with nothing said, ten at most, as a page that moves by itself draws on.
    let quiet = |said: &mpsc::Receiver<Option<Value>>| -> Vec<Value> {
        let (started, mut heard) = (std::time::Instant::now(), Vec::new());
        while started.elapsed() < std::time::Duration::from_secs(10) {
            match said.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(one) => heard.extend(one),
                Err(_) => break,
            }
        }
        heard
    };
    quiet(&said);
    for line in flag("--before").map(std::fs::read_to_string).transpose()?.unwrap_or_default().lines().filter(|line| !line.trim().is_empty()) {
        let request = renderer::inspect::Request::parse(line).map_err(|refused| anyhow::anyhow!("{refused}"))?;
        native.run(move |windows| windows.inspect(id, request, Box::new(|answer| eprintln!("{answer}"))));
        quiet(&said);
    }
    let mut reports = Vec::new();
    for _ in 0..rounds {
        let started = std::time::Instant::now();
        for event in trace.iter().filter(|event| event["kind"] == "wheel") {
            let at = std::time::Duration::from_secs_f64(event["ms"].as_f64().unwrap_or(0.0) / 1000.0);
            std::thread::sleep(at.saturating_sub(started.elapsed()));
            let request = json!({"v": 1, "id": 1, "op": "scroll", "x": event["x"], "y": event["y"], "dx": event["dx"], "dy": event["dy"]});
            let request = renderer::inspect::Request::parse(&request.to_string()).map_err(|refused| anyhow::anyhow!("{refused}"))?;
            native.run(move |windows| windows.inspect(id, request, Box::new(|_| {})));
        }
        // Its motion said once it's been still a moment (`frame_timing::STILL`).
        reports.extend(quiet(&said));
        // Back to its top, for the next round.
        let back = renderer::inspect::Request::parse(&json!({"v": 1, "id": 1, "op": "scroll", "x": trace[0]["x"], "y": trace[0]["y"], "dx": -100_000, "dy": -100_000}).to_string()).map_err(|refused| anyhow::anyhow!("{refused}"))?;
        native.run(move |windows| windows.inspect(id, back, Box::new(|_| {})));
        quiet(&said);
    }
    let mut long = false;
    for report in &reports {
        println!("{report}");
        let ms = |part: &str| report[part].as_f64().unwrap_or(0.0);
        long |= ms("paint_ms") + ms("draw_ms") > budget || ms("gpu_ms") > budget;
    }
    if reports.is_empty() {
        anyhow::bail!("no motion was said: nothing it played moved anything");
    }
    std::process::exit(i32::from(long));
}

/// The page in `folder` as a window's screen, its UI library from `ui` and what it asks answered
/// from `answers`, as `screenshot` makes it: on the windows' thread, which a screen stays on.
fn screen(folder: PathBuf, ui: Option<PathBuf>, answers: Value, panel: Option<f32>) -> anyhow::Result<Screen> {
    // The page's files at `file:///`, as its own folder, with the UI library's under /ui/client/, as `screenshot`.
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
    let js = Thread::start(&page, files, None, Clock::real(), false)?;
    // What the page asks its client, answered from `answers`, as `screenshot` answers it.
    let asked: renderer::Asked = Box::new(move |message, _| {
        let id = message.get("id")?.clone();
        let (method, tool) = (message["method"].as_str().unwrap_or_default(), message["params"]["name"].as_str());
        Some(match (method, tool) {
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
    let mut screen = Screen::new(js, Some(asked));
    screen.page = tokens_page;
    screen.theme = Some(Appearance::Light);
    screen.panel = panel.map(|corner| renderer::Panel { corner, fill: false, whole: false });
    Ok(screen)

}
