//! The API's document as this build makes it (`api::openapi`), of every feature it lists and every route that says
//! what it is.

use serde_json::Value;

/// The document for a caller who has `apps`, as `GET /api/apps` gives them. Only this machine's own programs see what only
/// they may ask.
pub fn document(apps: &[Value], this_machine: bool) -> Value {
    ::api::openapi::document(&crate::core_tools::all(), crate::says::listed(), apps, this_machine)
}
