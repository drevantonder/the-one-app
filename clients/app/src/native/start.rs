//! Starting a page: its JS on a thread of its own, its files asked for over its window's channel, and its screen.

use super::*;

/// Where a page is drawn: in its own window, or in a frame, by its node, of a window's page.
#[derive(Clone)]
pub(super) enum Place {
    Window(Arc<OnceLock<WindowId>>),
    Frame(Arc<OnceLock<WindowId>>, renderer::tree::Id),
}

/// A page whose JS has started, on its thread, and its channel to its app's window, with what it's asked over it.
pub(super) struct Started {
    pub(super) js: anyhow::Result<Thread>,
    pub(super) to: UnboundedSender<Value>,
    pub(super) asked: Arc<Mutex<HashMap<String, Asked>>>,
    pub(super) page: Url,
    /// Which page it is, of every one started on this device (`STARTED`).
    pub(super) number: u64,
}

/// Starts `entrypoint`'s page, its `module`, at `link`, for `subject`, drawn at `place`: its channel to its app's
/// window, and its JS on a thread of its own, its files each asked for over that as the sandbox imports it.
pub(super) fn start(entrypoint: &str, subject: Option<&str>, link: &str, launcher: bool, place: Place, module: &Url, page: Url) -> anyhow::Result<Started> {
    let native = Native::get()?;
    let asked: Arc<Mutex<HashMap<String, Asked>>> = Arc::default();
    let to = channel(entrypoint, subject, link, launcher, asked.clone(), native, place)?;
    Ok(started(entrypoint, subject, to, asked, module, page))
}

/// Starts a page, its `module`, on a thread of its own, over its window's channel `to`, its files each asked for over
/// that as the sandbox imports it.
pub(super) fn started(entrypoint: &str, subject: Option<&str>, to: UnboundedSender<Value>, asked: Arc<Mutex<HashMap<String, Asked>>>, module: &Url, page: Url) -> Started {
    let number = STARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let Ok(home) = crate::app().shell.home() else {
        return Started { js: Err(anyhow::anyhow!("the client has no home folder")), to, asked, page, number };
    };
    let kept = home.join("native");
    // Pages' compiled JS is kept for the QuickJS that compiled it (`renderer::js::QUICKJS`), so a newer build on the
    // same QuickJS starts its pages from it, and an update's first start doesn't compile each again (V8's code cache is
    // kept so): what another QuickJS kept goes, once, as this one first starts a page, behind it, as it may be a folder
    // of every app's bytecode, which no page waits for.
    static PRUNED: std::sync::Once = std::sync::Once::new();
    PRUNED.call_once(|| {
        let kept = kept.clone();
        std::thread::spawn(move || {
            for before in std::fs::read_dir(&kept).into_iter().flatten().flatten().filter(|entry| entry.file_name() != renderer::js::QUICKJS) {
                if let Err(error) = std::fs::remove_dir_all(before.path()) {
                    eprintln!("{} wasn't removed: {error}", before.path().display());
                }
            }
        });
    });
    let cache = kept.join(renderer::js::QUICKJS);
    let (files_to, files_asked, files_view) = (to.clone(), asked.clone(), format!("{}|{entrypoint}", subject.unwrap_or_default()));
    let next = std::sync::atomic::AtomicU64::new(0);
    let files: Files = Box::new(move |url: &Url| {
        let id = format!("f{number}.{}", next.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let (answer_to, answer) = mpsc::channel();
        files_asked.lock().expect("nothing panics holding it").insert(id.clone(), Asked::File(answer_to));
        Opening::asked(&files_view, &id, "file".to_owned());
        let _ = files_to.send(json!({"id": id, "file": url.as_str()}));
        let file = answer.recv()?.map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok(String::from_utf8(file)?)
    });
    let js = Thread::start(module, files, Some(cache), Clock::real(), cfg!(mobile));
    Started { js, to, asked, page, number }
}

impl Started {
    /// Its screen, on the native thread: what its page asks answered as `state` says, `insets` of its window its own
    /// (`Screen::insets`), and a picture it names by an address fetched as one of its app's files, as a web view's page
    /// loads it.
    pub(super) fn screen(self, mut state: State, insets: renderer::geometry::Edges<f32>) -> anyhow::Result<Screen> {
        let Started { js, to, asked, page, number } = self;
        state.page = number;
        let state = Arc::new(Mutex::new(state));
        let (respond_to, respond_asked) = (to.clone(), asked.clone());
        let asked_by_app: renderer::Asked = Box::new(move |message, appearance| respond(message, appearance, &state, &respond_to, &respond_asked));
        let mut screen = Screen::new(js?, Some(asked_by_app));
        screen.insets = insets;
        let next_picture = std::cell::Cell::new(0u64);
        screen.fetch = Some(Box::new(move |src: &str| {
            let Ok(address) = page.join(src) else { return };
            let id = format!("i{number}.{}", next_picture.replace(next_picture.get() + 1));
            asked.lock().expect("nothing panics holding it").insert(id.clone(), Asked::Picture(src.to_owned()));
            let _ = to.send(json!({"id": id, "file": address.as_str()}));
        }));
        Ok(screen)
    }
}

/// How many pages have started on this device, which numbers each, its window's requests named by it (`State::page`).
pub(super) static STARTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
