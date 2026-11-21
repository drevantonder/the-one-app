//! What the harness does between the display's frames: an app opened over the Home Screen, and a frame's page started.

use renderer::platform::Windows;

thread_local! {
    /// What the harness does between the display's frames, as a host's app does around its windows: an app's window
    /// opened over the Home Screen as its page asked (`--open`), and what its tools answered as it's due.
    pub(crate) static BETWEEN: std::cell::RefCell<Option<Box<dyn FnMut(&mut Windows)>>> = const { std::cell::RefCell::new(None) };
    /// The frames' pages a window's page asked for since, started and handed to their frames (`framed`).
    pub(crate) static FRAMES: std::cell::RefCell<Option<Box<dyn FnMut(&mut Windows)>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn between(windows: &mut Windows) {
    let hook = BETWEEN.take();
    if let Some(mut hook) = hook {
        hook(windows);
        BETWEEN.set(Some(hook));
    }
    let frames = FRAMES.take();
    if let Some(mut frames) = frames {
        frames(windows);
        FRAMES.set(Some(frames));
    }
}
