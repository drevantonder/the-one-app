//! A lock that refuses the thread already holding it, rather than wait on itself for good.

use super::*;

/// A lock that refuses the thread already holding it. A std `Mutex` asked for again on the thread holding it waits on
/// itself for good, and every thread asking after that waits on it too: once those are all the runtime's workers, the
/// app runtime takes no connection and answers nothing, though it still listens. Refused instead, and said where.
pub(super) struct Exclusive<T>(pub(super) Mutex<T>);

pub(super) struct Held<'a, T>(std::sync::MutexGuard<'a, T>, usize);

impl<T> Exclusive<T> {
    pub(super) const fn new(value: T) -> Self {
        Exclusive(Mutex::new(value))
    }

    pub(super) fn lock(&self) -> Result<Held<'_, T>> {
        let me = self as *const Self as usize;
        if HOLDING.with_borrow(|h| h.contains(&me)) {
            let at = std::backtrace::Backtrace::force_capture();
            tracing::error!("the network was asked for on a thread that holds it, which would stop this app runtime answering: {at}");
            debug_assert!(false, "the network asked for on a thread that holds it: {at}");
            bail!("this device couldn't answer that: a bug in it, which its log shows");
        }
        let held = Held(self.0.lock().unwrap(), me);
        HOLDING.with_borrow_mut(|h| h.push(me));
        Ok(held)
    }
}

impl<T> std::ops::Deref for Held<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> std::ops::DerefMut for Held<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> Drop for Held<'_, T> {
    fn drop(&mut self) {
        HOLDING.with_borrow_mut(|h| h.retain(|l| *l != self.1));
    }
}
