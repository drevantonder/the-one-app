use super::*;

impl Native {
    /// The windows' thread, started the first time it's asked for.
    pub fn get() -> anyhow::Result<Native> {
        STARTED.get_or_init(|| start().map_err(|error| format!("{error:#}"))).clone().map_err(|error| anyhow::anyhow!(error))
    }

    /// The windows' thread, if it's been started.
    pub fn started() -> Option<Native> {
        STARTED.get()?.clone().ok()
    }

    /// Runs `job` on the windows' thread, where every window and its screen are.
    pub fn run(&self, job: impl FnOnce(&mut Windows) + Send + 'static) {
        let _ = self.0.send(Box::new(job));
    }

    /// Whether this device's windows are a phone's: each the whole screen, the Home Screen the root window under every
    /// app's, closed only as its system closes it. The fake platform's are where the app harness says so (`ONE_PHONE`).
    pub fn phone() -> bool {
        #[cfg(feature = "fake")]
        return phone_screen().is_some();
        #[cfg(not(feature = "fake"))]
        cfg!(any(target_os = "android", target_os = "ios"))
    }
}

/// The fake platform's screen, where it's a phone's: `ONE_PHONE`, its size in points, as `390x844`.
#[cfg(feature = "fake")]
fn phone_screen() -> Option<Size<Pixels>> {
    let (width, height) = std::env::var("ONE_PHONE").ok()?.split_once('x').map(|(width, height)| (width.parse().ok(), height.parse().ok()))?;
    Some(Size { width: Pixels(width?), height: Pixels(height?) })
}

#[cfg(all(target_os = "windows", not(feature = "fake")))]
use super::windows::start;

/// The fake platform's windows for an app that asks for them as it would a system's, as the app does built with it
/// (`clients/app`'s `fake`, the app harness): a thread of their own, as a host's, on real time, whose display refreshes
/// at 60 Hz while something moves, its pages' posts and its jobs taken in between, as each host's loop does. With a
/// seed (`ONE_SEED`), it's as busy as a loaded device's: one job in four waits up to 2 s for the thread, as a Surface's
/// launcher waited 4.5 s to be made as the app started (21 Nov). Its screen is a phone's where `ONE_PHONE` says so
/// (`Native::phone`). A harness that drives them itself, frame by frame, has
/// them by `Windows::fake` instead (`examples/screenshot/main.rs`).
#[cfg(feature = "fake")]
fn start() -> anyhow::Result<Native> {
    let (sender, jobs) = std::sync::mpsc::channel::<Job>();
    let (ready_to, ready) = std::sync::mpsc::channel();
    let seed: u64 = std::env::var("ONE_SEED").ok().and_then(|seed| seed.parse().ok()).unwrap_or(0);
    std::thread::Builder::new().name("native".into()).spawn(move || {
        let mut windows = match Windows::fake(1.0, 60.0, crate::js::Clock::real()) {
            Ok(windows) => windows,
            Err(error) => return drop(ready_to.send(Err(error))),
        };
        windows.seed(seed);
        if let Some(screen) = phone_screen() {
            windows.phone(screen);
        }
        let _ = ready_to.send(Ok(()));
        let mut ticked = web_time::Instant::now();
        loop {
            match jobs.recv_timeout(windows.due().unwrap_or(FRAME).min(FRAME)) {
                Ok(job) => {
                    if seed != 0 && windows.random(4) == 0 {
                        std::thread::sleep(std::time::Duration::from_millis(windows.random(2000)));
                    }
                    job(&mut windows)
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
            while let Ok(job) = jobs.try_recv() {
                job(&mut windows);
            }
            windows.wait();
            if windows.wanted() && ticked.elapsed() >= FRAME {
                ticked = web_time::Instant::now();
                windows.tick();
            }
            windows.turn();
        }
    })?;
    ready.recv()??;
    Ok(Native(sender))
}

#[cfg(target_os = "android")]
use super::android::start;
#[cfg(target_os = "ios")]
use super::ios::start;
#[cfg(target_arch = "wasm32")]
use super::web::start;

#[cfg(all(any(target_os = "linux", target_os = "macos"), not(feature = "fake")))]
fn start() -> anyhow::Result<Native> {
    let (sender, channel) = calloop::channel::channel::<Job>();
    let (ready_to, ready) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("native".into()).spawn(move || {
        // It takes the person's input and draws what they see, so it runs ahead of work nobody's waiting on, as an
        // app's main thread does: as far as the system lets it.
        let _ = thread_priority::set_current_thread_priority(thread_priority::ThreadPriority::Max);
        let mut event_loop = match calloop::EventLoop::<Windows>::try_new() {
            Ok(event_loop) => event_loop,
            Err(error) => return drop(ready_to.send(Err(anyhow::anyhow!("{error}")))),
        };
        let windows = Display::connect(&event_loop.handle()).and_then(Windows::new);
        let mut windows = match windows {
            Ok(windows) => windows,
            Err(error) => return drop(ready_to.send(Err(error))),
        };
        let inserted = event_loop.handle().insert_source(channel, |event, _, windows: &mut Windows| {
            if let calloop::channel::Event::Msg(job) = event {
                job(windows);
            }
        });
        if let Err(error) = inserted {
            return drop(ready_to.send(Err(anyhow::anyhow!("{error}"))));
        }
        let _ = ready_to.send(Ok(()));
        loop {
            let timeout = windows.due();
            if let Err(error) = event_loop.dispatch(timeout, &mut windows) {
                log::error!("native windows: {error}");
            }
            windows.turn();
        }
    })?;
    ready.recv()??;
    Ok(Native(sender))
}
