//! A computer's window of web content discards its page once it's been minimised or hidden a while, as a browser
//! discards a tab and a phone ends an app in the background: its web view goes, and with it its web content process,
//! and the page loads again as the window comes back. Each app's own window has one (`host`), and the launcher
//! (`computer`).

use std::time::{Duration, Instant};

use tao::event::WindowEvent;
use tao::event_loop::ControlFlow;
use tao::window::Window;

/// How long a window is minimised or hidden before its page is discarded.
pub(crate) const DISCARD_AFTER: Duration = Duration::from_secs(60);

/// A window's page, made again each time it's loaded.
pub(crate) trait Page {
    /// The page, in a web view's builder.
    fn make<'a>(&mut self, builder: wry::WebViewBuilder<'a>) -> wry::WebViewBuilder<'a>;

    /// Its web view is made, for what's set on one once it is.
    fn loaded(&mut self, _webview: &wry::WebView) {}

    /// Its web view is gone: what it asked and wasn't answered is no one's.
    fn discarded(&mut self) {}
}

/// A window's page, which is discarded while the window's out of sight a while (`DISCARD_AFTER`).
pub(crate) struct Discarding<P> {
    page: P,
    loaded: Option<Loaded>,
    /// Since when the window may have been out of sight.
    away: Option<Instant>,
    /// Where its web view keeps its data, if not where wry does.
    data_directory: Option<std::path::PathBuf>,
    /// Where in a Linux window its page goes, if not in the window's own box.
    #[cfg(target_os = "linux")]
    parent: Option<gtk::Container>,
}

/// A web view, and the web context it was made in, which goes with it: WebKitGTK keeps a custom scheme's handler in
/// the context, for the page that registered it.
struct Loaded {
    webview: wry::WebView,
    _context: Box<wry::WebContext>,
}

impl<P: Page> Discarding<P> {
    /// The window's page, loaded, keeping its data in `data_directory`.
    pub(crate) fn new(window: &Window, page: P, data_directory: Option<std::path::PathBuf>) -> anyhow::Result<Self> {
        let mut discarding = Self {
            page,
            loaded: None,
            away: None,
            data_directory,
            #[cfg(target_os = "linux")]
            parent: None,
        };
        discarding.load(window)?;
        Ok(discarding)
    }

    /// Has the page load in `parent` from now on, where the window has moved it (a GNOME header bar's overlay).
    #[cfg(target_os = "linux")]
    pub(crate) fn load_in(&mut self, parent: gtk::Container) {
        self.parent = Some(parent);
    }

    /// Its web view, unless it's discarded.
    pub(crate) fn webview(&self) -> Option<&wry::WebView> {
        self.loaded.as_ref().map(|loaded| &loaded.webview)
    }

    /// Whether the page is on screen, as it says itself or as whatever shows and hides the window says: back on
    /// screen, a discarded page loads again.
    pub(crate) fn shown(&mut self, window: &Window, on: bool) {
        if !on {
            self.away = Some(Instant::now());
            return;
        }
        self.away = None;
        if self.loaded.is_none()
            && let Err(error) = self.load(window)
        {
            eprintln!("this window's page didn't load again: {error:#}");
        }
    }

    /// Loads a discarded page to do something, though its window may stay out of sight, where it's discarded again
    /// in time.
    pub(crate) fn needed(&mut self, window: &Window) {
        if self.loaded.is_some() {
            return;
        }
        if let Err(error) = self.load(window) {
            eprintln!("this window's page didn't load again: {error:#}");
        }
        if window.is_minimized() || !window.is_visible() {
            self.away = Some(Instant::now());
        }
    }

    /// What the window heard: losing focus, which it does as it's minimised, may be it going out of sight, and
    /// focus, a new size or place, which it has as it's brought back, is it back if it's on screen.
    pub(crate) fn heard(&mut self, window: &Window, event: &WindowEvent) {
        match event {
            WindowEvent::Focused(false) => self.shown(window, false),
            WindowEvent::Focused(true) | WindowEvent::Resized(_) | WindowEvent::Moved(_)
                if self.loaded.is_none() && window.is_visible() && !window.is_minimized() =>
            {
                self.shown(window, true)
            }
            _ => {}
        }
    }

    /// Discards the page once the window's been minimised or hidden long enough. → How the event loop waits: until
    /// then, if it may be.
    pub(crate) fn discard_due(&mut self, window: &Window) -> ControlFlow {
        let Some(since) = self.away.filter(|_| self.loaded.is_some()) else { return ControlFlow::Wait };
        if since.elapsed() < DISCARD_AFTER {
            return ControlFlow::WaitUntil(since + DISCARD_AFTER);
        }
        // Out of focus or behind others, it's still there to see.
        if window.is_minimized() || !window.is_visible() {
            self.loaded = None;
            self.page.discarded();
        }
        self.away = None;
        ControlFlow::Wait
    }

    fn load(&mut self, window: &Window) -> anyhow::Result<()> {
        let mut context = Box::new(wry::WebContext::new(self.data_directory.clone()));
        // WebDriver drives it as it does Tauri's windows, which tauri-driver says by this.
        context.set_allows_automation(std::env::var("TAURI_WEBVIEW_AUTOMATION").as_deref() == Ok("true"));
        let builder = self.page.make(wry::WebViewBuilder::new_with_web_context(&mut context));
        #[cfg(windows)]
        let webview = builder.build(window)?;
        // A Mac's first page is made before the event loop runs, where no autorelease pool is drained, so what AppKit
        // and WebKit autorelease as it's made would keep its WKWebView, with its web content process, alive after it's
        // discarded: it's made in a pool of its own.
        #[cfg(target_os = "macos")]
        let webview = objc2::rc::autoreleasepool(|_| builder.build(window))?;
        #[cfg(target_os = "linux")]
        let webview = {
            use gtk::prelude::*;
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewBuilderExtUnix;
            match &self.parent {
                Some(parent) => builder.build_gtk(parent)?,
                None => builder.build_gtk(
                    window.default_vbox().expect("tao gives a window a box").upcast_ref::<gtk::Container>(),
                )?,
            }
        };
        self.page.loaded(&webview);
        self.loaded = Some(Loaded { webview, _context: context });
        Ok(())
    }
}
