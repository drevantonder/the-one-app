//! Timing an app window's opening and saying how it went, in the app's log and to Telemetry, and how smoothly what moves on it moved.

use super::*;

/// How a window's page opened, step by step, from being asked to its first frame, by its view
/// (`<subject>|<entrypoint>`): which says, in the log and to Telemetry, where its time went (`Opening::drawn`).
pub(super) static OPENING: std::sync::LazyLock<Mutex<HashMap<String, Opening>>> = std::sync::LazyLock::new(Default::default);

/// A window's page opening: when it was asked to, each step's time since then, the module files it read, and each
/// request it asked as it opened, by its id, with what it was and when. Its first frame may come before what it
/// asked is answered, which it draws as it's waiting for: it's ready once it's drawn and its page says what it asked
/// for has settled (`notifications/one/drawn`, the UI library's `tellDrawn`), as a browser's page is shown at its
/// Largest Contentful Paint.
pub(super) struct Opening {
    pub(super) started: Instant,
    pub(super) steps: Vec<(String, u64, Option<u64>, Option<String>)>,
    pub(super) asking: HashMap<String, (String, Instant)>,
    /// The module files it read: how many, the slowest, and when the last was read, since it was asked to open.
    pub(super) files: (u32, u64, u64),
    pub(super) live: Option<u64>,
    pub(super) settled: bool,
    pub(super) app: String,
}

impl Opening {
    /// `view` begins opening, asked to at `started`.
    pub(super) fn begin(view: &str, started: Instant) {
        let opening = Opening { started, steps: Vec::new(), asking: HashMap::new(), files: (0, 0, 0), live: None, settled: false, app: String::new() };
        OPENING.lock().expect("nothing panics holding it").insert(view.to_owned(), opening);
    }

    /// Does `what` to `view`'s opening, while it's opening.
    pub(super) fn with(view: &str, what: impl FnOnce(&mut Opening)) {
        if let Some(opening) = OPENING.lock().expect("nothing panics holding it").get_mut(view) {
            what(opening)
        }
    }

    /// A step of `view`'s opening, now.
    pub(super) fn step(view: &str, step: &str) {
        Self::with(view, |opening| opening.steps.push((step.to_owned(), opening.started.elapsed().as_millis() as u64, None, None)));
    }

    /// `view`'s page asked `id`, `what` it is, as it opened.
    pub(super) fn asked(view: &str, id: &str, what: String) {
        Self::with(view, |opening| drop(opening.asking.insert(id.to_owned(), (what, Instant::now()))));
    }

    /// `view`'s page says what it asked for as it opened has settled.
    pub(super) fn settled(view: &str) {
        Self::step(view, "settled");
        Self::with(view, |opening| opening.settled = true);
        Self::ready(view);
    }

    /// `view`'s page was answered `id`, by the server this client asks (`Client::asking`), or read a module file.
    pub(super) fn answered(view: &str, id: &str) {
        Self::with(view, |opening| {
            let Some((what, sent)) = opening.asking.remove(id) else { return };
            let took = sent.elapsed().as_millis() as u64;
            if what == "file" {
                opening.files = (opening.files.0 + 1, opening.files.1.max(took), opening.started.elapsed().as_millis() as u64);
                return;
            }
            let at = sent.duration_since(opening.started).as_millis() as u64;
            let server = crate::views().ok().and_then(|views| views.client.asking());
            opening.steps.push((what, at, Some(took), server));
        });
    }

    /// `view`'s first frame, its snapshot's where it has one.
    pub(super) fn drawn(view: &str, app: &str) {
        Self::step(view, "drawn");
        Self::with(view, |opening| opening.app = app.to_owned());
    }

    /// `view`'s page's first frame, and when the last of its module's files it read before it was read.
    pub(super) fn live(view: &str) {
        Self::with(view, |opening| {
            let (files, slowest, last) = opening.files;
            opening.steps.push((format!("bundle read, {files} files, the slowest in {slowest} ms"), last, None, None));
            let ms = opening.started.elapsed().as_millis() as u64;
            opening.steps.push(("live".to_owned(), ms, None, None));
            opening.live = Some(ms);
        });
        Self::ready(view);
    }

    /// `view`'s page is ready, once its page is drawn and what it asked as it opened is answered: how it opened, in
    /// the log, a line, and to Telemetry, each step's milliseconds as a histogram by its view and step
    /// (`one.client.open.step`: how long since it was asked to open, or for a request how long it took), and `ready`'s,
    /// so each step's percentiles are read per device; and the open as a span to its page's first frame, with every
    /// step, which Telemetry's `open` alert reads.
    pub(super) fn ready(view: &str) {
        let mut openings = OPENING.lock().expect("nothing panics holding it");
        if !openings.get(view).is_some_and(|opening| opening.live.is_some() && opening.settled) {
            return;
        }
        let Some(opening) = openings.remove(view) else { return };
        drop(openings);
        let (ms, app) = (opening.started.elapsed().as_millis() as u64, opening.app);
        let said: Vec<String> = opening
            .steps
            .iter()
            .map(|(step, at, took, server)| match (took, server) {
                (Some(took), server) => format!("{step} at {at} ms took {took} ms ({})", server.as_deref().unwrap_or("no server")),
                (None, _) => format!("{step} {at} ms"),
            })
            .collect();
        eprintln!("{app} was ready in {ms} ms: {}", said.join(", "));
        let Ok(views) = crate::views() else { return };
        let live = opening.live.unwrap_or(ms);
        views.client.report(json!({"name": "one.client.native.open", "took": live, "attributes": {"one.view": app, "one.steps": said.join(", ")}}));
        let steps = opening.steps.into_iter().map(|(step, at, took, _server)| (step, took.unwrap_or(at))).chain([("ready".to_owned(), ms)]);
        for (step, ms) in steps {
            // What's said after its name, as how many files it read, isn't the step's.
            let step = step.split(',').next().unwrap_or_default();
            views.client.report(json!({"name": "one.client.open.step", "value": ms, "attributes": {"one.view": app, "one.step": step}}));
        }
    }
}

/// Tells Telemetry, as an error, that `app`'s window at `view` was still blank a while after it opened (`Event::Blank`):
/// why, the last step its opening reached (`one.client.open.step`'s, `ready` where it finished), and the build of the
/// server it asks (`one.server.version`, as `ping` says it, `unknown` where none answers), beside the client's own
/// (`service.version`), so a blank that one pair of builds shows is told apart. Its message says them too, as what
/// Telemetry keeps of an hour gone by is an error's message, not all it said.
pub(super) fn blank(view: &str, app: &str, why: &str) {
    let step = OPENING.lock().expect("nothing panics holding it").get(view).map_or("ready".to_owned(), |opening| opening.steps.last().map_or("asked".to_owned(), |(step, ..)| step.split(',').next().unwrap_or_default().to_owned()));
    eprintln!("{app}'s window is blank: {why}, its opening's last step {step}");
    let Ok(views) = crate::views() else { return };
    let (app, why) = (app.to_owned(), why.to_owned());
    crate::spawn(async move {
        let pinged = tokio::time::timeout(std::time::Duration::from_secs(5), views.client.request("ping", json!({}))).await;
        let server = pinged.ok().and_then(Result::ok).and_then(|pong| pong["_meta"]["io.modelcontextprotocol/serverInfo"]["version"].as_str().map(str::to_owned));
        let server = server.as_deref().unwrap_or("unknown");
        let error = format!("{why}, its opening's last step {step}; this app's build {}, its server's {server}", ::client::BUILD);
        let attributes = json!({"one.view": app, "one.step": step, "one.server.version": server});
        views.client.report(json!({"name": "one.client.native.blank", "error": error, "attributes": attributes}));
    });
}

/// Tells Telemetry an app's window didn't open, and why, as it's said in the window, where it could open one.
pub(crate) fn reported(entrypoint: &str, started: Instant, why: &str) {
    eprintln!("{entrypoint} didn't open: {why}");
    if let Ok(views) = crate::views() {
        let app = entrypoint.split('/').next().unwrap_or_default();
        let took = started.elapsed().as_millis() as u64;
        views.client.report(json!({"name": "one.client.native.open", "took": took, "error": why, "attributes": {"one.view": app}}));
    }
}

/// Tells Telemetry how smoothly something moved on `app`'s window, and says it in the app's log: how many frames, their
/// intervals' median and 95th percentile, how many were long (shown twice or more) and where, the display's refresh,
/// and each part of a frame's 95th percentile (`renderer::platform::Frame`). Each is a histogram by the app, what
/// moved it and the display's rate, so `metric` reads them over time.
pub(super) fn frames_reported(app: &str, frames: &renderer::platform::Frames) {
    let ms = |time: std::time::Duration| (time.as_secs_f64() * 100_000.0).round() / 100.0;
    let hz = (1.0 / frames.refresh.as_secs_f64()).round();
    let load = Load::now();
    let gpu = frames.gpu.map_or("unknown".to_owned(), |gpu| format!("{} ms (blurs {} ms)", ms(gpu), ms(frames.blur.unwrap_or_default())));
    // The first long frame, by its parts, and where each long one was, so a cost one moment pays shows where it is.
    let first = frames.first_long.map_or(String::new(), |frame| {
        let gpu = frame.gpu.map_or("unknown".to_owned(), |gpu| format!("{} ms", ms(gpu)));
        format!(" at {:?}; the first's paint {} ms, draw {} ms (waiting {} ms for an image, submitting {} ms, presenting {} ms), GPU {gpu}", frames.long_at, ms(frame.paint), ms(frame.draw), ms(frame.acquire), ms(frame.submit), ms(frame.present))
    });
    // The longest gap, where it's long: how much of it this thread spent applying the page's changes and on the loop's
    // turns, the rest elsewhere (an iPhone's main thread, the system's work on it).
    let gap = frames.longest_gap.map_or(String::new(), |(gap, frame)| format!("; the longest gap {} ms: applying the page's changes {} ms, turns {} ms", ms(gap), ms(frame.applied), ms(frame.turned)));
    eprintln!(
        "{app}'s {} took {} ms: {} frames, interval p50 {} ms, p95 {} ms, longest {} ms, {} long at {hz} Hz{first}; p95 paint {} ms, draw {} ms, waiting for an image {} ms, submitting {} ms, presenting {} ms, GPU {gpu}{gap}; {load}",
        frames.kind,
        ms(frames.took),
        frames.frames,
        ms(frames.p50),
        ms(frames.p95),
        ms(frames.longest),
        frames.long,
        ms(frames.paint),
        ms(frames.draw),
        ms(frames.acquire),
        ms(frames.submit),
        ms(frames.present),
    );
    let Ok(views) = crate::views() else { return };
    let attributes = json!({"one.view": app, "one.motion": frames.kind, "one.display.hz": hz, "one.system.busy": load.busy()});
    if let Some(average) = load.average {
        views.client.report(json!({"name": "system.cpu.load_average.1m", "value": average, "attributes": attributes}));
    }
    let gpu = frames.gpu.map(|gpu| ("one.client.native.frame.gpu.p95", ms(gpu)));
    let blur = frames.blur.map(|blur| ("one.client.native.frame.gpu.blur.p95", ms(blur)));
    let records = [
        ("one.client.native.frames", f64::from(frames.frames)),
        ("one.client.native.frames.long", f64::from(frames.long)),
        ("one.client.native.frame.interval.p95", ms(frames.p95)),
        ("one.client.native.frame.paint.p95", ms(frames.paint)),
        ("one.client.native.frame.draw.p95", ms(frames.draw)),
        ("one.client.native.frame.acquire.p95", ms(frames.acquire)),
        ("one.client.native.frame.submit.p95", ms(frames.submit)),
        ("one.client.native.frame.present.p95", ms(frames.present)),
    ];
    let gap = frames.longest_gap.into_iter().flat_map(|(gap, frame)| {
        [("one.client.native.frame.gap", ms(gap)), ("one.client.native.frame.gap.applied", ms(frame.applied)), ("one.client.native.frame.gap.turned", ms(frame.turned))]
    });
    for (name, value) in records.into_iter().chain(gpu).chain(blur).chain(gap) {
        views.client.report(json!({"name": name, "value": value, "attributes": attributes}));
    }
    // Where in a motion each long frame was, 0 the interval after its first frame.
    for at in &frames.long_at {
        views.client.report(json!({"name": "one.client.native.frame.long.at", "value": at, "attributes": attributes}));
    }
}

/// How busy the device is as something's timed, so a machine others load isn't taken for slow drawing: its 1-minute
/// load average, where its system says one (not Windows), and its logical cores.
pub(super) struct Load {
    pub(super) average: Option<f64>,
    pub(super) cores: usize,
}

impl Load {
    pub(super) fn now() -> Load {
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux"))]
        let average = {
            let mut averages = [0.0f64; 3];
            (unsafe { libc::getloadavg(averages.as_mut_ptr(), 3) } >= 1).then_some(averages[0])
        };
        #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
        let average = None;
        Load { average, cores: std::thread::available_parallelism().map_or(1, usize::from) }
    }

    /// More is waiting to run than it has cores.
    pub(super) fn busy(&self) -> bool {
        self.average.is_some_and(|average| average > self.cores as f64)
    }
}

impl std::fmt::Display for Load {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.average {
            Some(average) => write!(f, "load {average:.1} on {} cores", self.cores),
            None => write!(f, "{} cores", self.cores),
        }
    }
}
