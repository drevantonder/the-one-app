//! The API (GLOSSARY): what every client asks a device through, at `/api/*`. Here so far: the paths more than one place
//! names (`path`), the wire between a running call and its runtime (`call`), the keys a caller hands for a call
//! (`handed`), and the module the Workers and the web client import (`protocol_js`).

pub mod call;
pub mod handed;
pub mod protocol_js;

/// The paths of the local API that more than one place names: what is kept apart by what a request runs (`keys::Route`),
/// and what is the API's own, not an app's (`/<app>/…` is an app's, reached by git).
pub mod path {
    pub const CALL: &str = "/api/call";
    pub const SCREENS: &str = "/api/screens/";
    pub const MCP: &str = "/mcp";
    pub const WIDGETS: &str = "/api/widgets";
    pub const BADGES: &str = "/api/badges";
    pub const APPS: &str = "/api/apps";
    pub const VAULT: &str = "/api/vault";
    pub const OPENAPI: &str = "/api/openapi.json";
    pub const HOOKS_PROMPT: &str = "/hooks/prompt";
    /// The first segments of the API's own routes with more under them (`routes()` holds to it: `api::tests`).
    pub const ROOTS: [&str; 4] = ["api", "mcp", "hooks", "skills"];

    /// The app an app's own request (`/<app>/…`, with something after) is for: none for the API's own.
    pub fn app_request(path: &str) -> Option<&str> {
        let (app, rest) = path.strip_prefix('/')?.split_once('/')?;
        (!app.is_empty() && !rest.is_empty() && !ROOTS.contains(&app)).then_some(app)
    }
}
