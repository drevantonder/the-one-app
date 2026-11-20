//! An iPhone's swipe in from the left edge goes back as its Back does: at an app's first screen, the page's move home
//! follows the finger as far across the window as it's come (`grab`, `drag` and `release` in gestures.js), as a pop
//! does, and let go, it carries on if it's past halfway or flicked, and otherwise springs back; deeper in, let go far
//! enough, it goes back a screen. It's on wherever the page says back has somewhere to go (`navigation_gestures`).

use std::cell::OnceCell;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_ui_kit::{UIGestureRecognizerState, UIRectEdge, UIScreenEdgePanGestureRecognizer, UIView};

static WINDOW: OnceLock<tauri::WebviewWindow> = OnceLock::new();
thread_local! {
    // UIKit's, on the main thread, as long as the app.
    static PAN: OnceCell<Retained<UIScreenEdgePanGestureRecognizer>> = const { OnceCell::new() };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OneEdgeSwipe"]
    struct Swipe;

    unsafe impl NSObjectProtocol for Swipe {}

    impl Swipe {
        #[unsafe(method(panned:))]
        fn panned(&self, pan: &UIScreenEdgePanGestureRecognizer) {
            let (Some(window), Some(view)) = (WINDOW.get(), pan.view()) else { return };
            let width = view.bounds().size.width.max(1.0);
            let state: UIGestureRecognizerState = unsafe { msg_send![pan, state] };
            let script = match state {
                UIGestureRecognizerState::Began => "grab()".to_owned(),
                UIGestureRecognizerState::Changed => format!("drag({})", pan.translationInView(Some(&*view)).x / width),
                UIGestureRecognizerState::Ended => format!("release({})", pan.velocityInView(Some(&*view)).x / width),
                // Taken away, as by a call coming in: it goes back where it was.
                UIGestureRecognizerState::Cancelled | UIGestureRecognizerState::Failed => "release(0, 0)".to_owned(),
                _ => return,
            };
            let _ = window.eval(script);
        }
    }
);

pub fn watch(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let _ = WINDOW.set(window.clone());
    window.with_webview(|webview| {
        // Tauri hands over the window's WKWebView on the main thread.
        let main = unsafe { MainThreadMarker::new_unchecked() };
        let web = unsafe { &*webview.inner().cast::<UIView>() };
        let swipe: Retained<Swipe> = unsafe { msg_send![Swipe::alloc(main), init] };
        let target: &AnyObject = &swipe;
        let pan = unsafe { UIScreenEdgePanGestureRecognizer::initWithTarget_action(UIScreenEdgePanGestureRecognizer::alloc(main), Some(target), Some(sel!(panned:))) };
        pan.setEdges(UIRectEdge::Left);
        pan.setEnabled(false);
        web.addGestureRecognizer(&pan);
        // A gesture recognizer holds its target weakly, and this one lasts as long as the app.
        std::mem::forget(swipe);
        PAN.with(|kept| drop(kept.set(pan)));
    })
}

/// Whether the swipe in from the edge is the window's.
pub fn take(on: bool) {
    // Tauri runs a command off the main thread, and UIKit's are the main thread's.
    if let Some(window) = WINDOW.get() {
        let _ = window.run_on_main_thread(move || PAN.with(|pan| pan.get().map(|pan| pan.setEnabled(on))).unwrap_or_default());
    }
}
