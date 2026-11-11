//! A set of the API's routes, each with who may ask it (`Who`): what the guard lets in.

use axum::Router;
use axum::routing::MethodRouter;

use crate::says::Who;

/// Routes over the state `S` their handlers read, each with who may ask it. A feature's are over `runtime::Rt`, the
/// runtime it's in; the runtime's own guard lets in who each says.
pub struct Routes<S> {
    pub router: Router<S>,
    pub who: Vec<(String, Who)>,
}

impl<S: Clone + Send + Sync + 'static> Default for Routes<S> {
    fn default() -> Self {
        Routes { router: Router::new(), who: Vec::new() }
    }
}

impl<S: Clone + Send + Sync + 'static> Routes<S> {
    pub fn route(mut self, path: &str, who: Who, handler: MethodRouter<S>) -> Routes<S> {
        self.router = self.router.route(path, handler);
        self.who.push((path.to_string(), who));
        self
    }

    pub fn merge(mut self, other: Routes<S>) -> Routes<S> {
        self.router = self.router.merge(other.router);
        self.who.extend(other.who);
        self
    }

    /// Every route's path, with who may ask it.
    pub fn who(&self) -> &[(String, Who)] {
        &self.who
    }
}
