//! What the routes for apps' screens and events say of themselves (onet's `listed`).

use serde::Deserialize;

use crate::call::Event;
use crate::says::Op;
use crate::says::Said;
use crate::says::Who;

pub fn listed() -> Vec<Said> {
    vec![
        ("/api/events", Who::Clients, Op::get("What changed, as it happens").events::<Event>()),
        ("/api/screens/{app}/{screen}", Who::Clients, Op::get("An app's screen, as a page").query::<ScreenQuery>().html()),
    ]
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ScreenQuery {
    pub params: Option<String>,
    pub scope: Option<String>,
    /// `@one/ui` and the bridge as the address's files (`ui::screen::page`), for the web client, which has them.
    pub linked: Option<bool>,
}
