//! A tap on an iPhone's status bar scrolls the app that's showing to its top, as it does in every iPhone app. iOS
//! scrolls the one scroll view that asks to be (`scrollsToTop`), which in the web view is its page, and that doesn't
//! scroll: its apps' frames do. So the page's says no, and an unseen scroll view a point high along the window's top
//! is asked instead (`scrollViewShouldScrollToTop:`). It says no too, and has the page scroll the app that's
//! showing to its top (`scrollToTop` in gestures.js).

use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_ui_kit::{UIScrollView, UIScrollViewDelegate, UIView, UIViewAutoresizing};

static WINDOW: OnceLock<tauri::WebviewWindow> = OnceLock::new();

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OneStatusBarTap"]
    struct Tap;

    unsafe impl NSObjectProtocol for Tap {}

    unsafe impl UIScrollViewDelegate for Tap {
        #[unsafe(method(scrollViewShouldScrollToTop:))]
        fn scroll_view_should_scroll_to_top(&self, _view: &UIScrollView) -> bool {
            if let Some(window) = WINDOW.get() {
                let _ = window.eval("scrollToTop()");
            }
            false
        }
    }
);

pub fn watch(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let _ = WINDOW.set(window.clone());
    window.with_webview(|webview| {
        // Tauri hands over the window's WKWebView on the main thread.
        let main = unsafe { MainThreadMarker::new_unchecked() };
        let web = unsafe { &*webview.inner().cast::<UIView>() };
        let page: Retained<UIScrollView> = unsafe { msg_send![web, scrollView] };
        page.setScrollsToTop(false);
        let width = web.bounds().size.width;
        let tapped = UIScrollView::initWithFrame(UIScrollView::alloc(main), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, 1.0)));
        tapped.setAutoresizingMask(UIViewAutoresizing::FlexibleWidth);
        // Never at its top, so it's always one there is to scroll.
        tapped.setContentSize(CGSize::new(width, 2.0));
        tapped.setContentOffset(CGPoint::new(0.0, 1.0));
        tapped.setScrollsToTop(true);
        let tap: Retained<Tap> = unsafe { msg_send![Tap::alloc(main), init] };
        unsafe { tapped.setDelegate(Some(ProtocolObject::from_ref(&*tap))) };
        web.addSubview(&tapped);
        // A scroll view holds its delegate weakly, and this one lasts as long as the app.
        std::mem::forget(tap);
    })
}
