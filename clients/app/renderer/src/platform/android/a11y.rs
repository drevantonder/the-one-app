//! A native screen's AccessKit tree (`platform::a11y`), given to TalkBack by AccessKit's Android adapter, which makes
//! itself the view's accessibility delegate. The adapter calls in while it holds its own lock, so what TalkBack asks is queued and done after, on the UI thread
//! (`NativeView.requestAccessibilityDrain`).

use std::sync::Mutex;

use accesskit::{ActionRequest, TreeUpdate};
use accesskit_android::InjectingAdapter;
use jni::objects::{GlobalRef, JObject};
use jni::{JNIEnv, JavaVM};

use super::WindowId;
use crate::Screen;
use crate::platform::a11y::tree;

static QUEUE: Mutex<Vec<(WindowId, ActionRequest)>> = Mutex::new(Vec::new());

pub struct A11y {
    adapter: InjectingAdapter,
    density: f64,
}

/// Asks the view to drain the queue on the UI thread.
struct Drain {
    vm: JavaVM,
    view: GlobalRef,
    window: WindowId,
}

impl Drain {
    fn request(&self) {
        if let Ok(mut env) = self.vm.attach_current_thread() {
            let _ = env.call_method(&self.view, "requestAccessibilityDrain", "()V", &[]);
        }
    }
}

impl accesskit::ActivationHandler for Drain {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // The tree is given as the screen next draws, which the drain makes it do.
        self.request();
        None
    }
}

impl accesskit::ActionHandler for Drain {
    fn do_action(&mut self, request: ActionRequest) {
        QUEUE.lock().expect("nothing panics holding it").push((self.window, request));
        self.request();
    }
}

impl A11y {
    pub fn new(env: &mut JNIEnv, view: &JObject, window: WindowId) -> A11y {
        let drain = |env: &mut JNIEnv| Drain { vm: env.get_java_vm().expect("a VM"), view: env.new_global_ref(view).expect("a view"), window };
        let (activation, action) = (drain(env), drain(env));
        let density = env
            .call_method(view, "getResources", "()Landroid/content/res/Resources;", &[])
            .and_then(|resources| env.call_method(resources.l()?, "getDisplayMetrics", "()Landroid/util/DisplayMetrics;", &[]))
            .and_then(|metrics| env.get_field(metrics.l()?, "density", "F"))
            .and_then(|density| density.f())
            .unwrap_or(1.0);
        A11y { adapter: InjectingAdapter::new(env, view, activation, action), density: f64::from(density) }
    }

    /// Gives TalkBack the screen as it is, if it's listening.
    pub fn update(&mut self, screen: &Screen) {
        let density = self.density;
        self.adapter.update_if_active(|| tree(screen, density));
    }
}

/// What TalkBack asked of window `window` since it was last asked.
pub fn take(window: WindowId) -> Vec<ActionRequest> {
    let mut queue = QUEUE.lock().expect("nothing panics holding it");
    let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut *queue).into_iter().partition(|(at, _)| *at == window);
    *queue = others;
    mine.into_iter().map(|(_, request)| request).collect()
}
