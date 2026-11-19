//! A native screen's AccessKit tree (`platform::a11y`), given to VoiceOver by AccessKit's iOS adapter, which
//! subclasses the window's view. What VoiceOver asks is queued and done once the windows aren't in use.

use std::cell::RefCell;
use std::ffi::c_void;

use accesskit::{ActionRequest, TreeUpdate};
use accesskit_ios::SubclassingAdapter;

use super::{WindowId, wake};
use crate::Screen;
use crate::platform::a11y::tree;

thread_local! {
    /// What VoiceOver asked, of which window, since the windows were last used.
    static ASKED: RefCell<Vec<(WindowId, ActionRequest)>> = const { RefCell::new(Vec::new()) };
}

/// A window's bridge to VoiceOver.
pub struct A11y(SubclassingAdapter);

/// What VoiceOver tells a window: that it's listening, which the next turn gives the tree, and what it asks.
struct Asks(WindowId);

impl accesskit::ActivationHandler for Asks {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        wake();
        None
    }
}

impl accesskit::ActionHandler for Asks {
    fn do_action(&mut self, request: ActionRequest) {
        ASKED.with(|asked| asked.borrow_mut().push((self.0, request)));
        wake();
    }
}

impl accesskit::DeactivationHandler for Asks {
    fn deactivate_accessibility(&mut self) {}
}

impl A11y {
    /// VoiceOver's bridge to window `window`'s `view`, a `UIView`, before it's first shown.
    pub fn new(view: *mut c_void, window: WindowId) -> A11y {
        A11y(unsafe { SubclassingAdapter::new(view, Asks(window), Asks(window), Asks(window)) })
    }

    /// Gives VoiceOver the screen as it is, if it's listening, drawn at `scale` device pixels a point.
    pub fn update(&self, screen: &Screen, scale: f32) {
        if let Some(events) = self.0.update_if_active(|| tree(screen, f64::from(scale))) {
            events.raise();
        }
    }
}

/// What VoiceOver asked of window `window` since it was last asked.
pub fn take(window: WindowId) -> Vec<ActionRequest> {
    ASKED.with(|asked| {
        let mut asked = asked.borrow_mut();
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut *asked).into_iter().partition(|(at, _)| *at == window);
        *asked = others;
        mine.into_iter().map(|(_, request)| request).collect()
    })
}
