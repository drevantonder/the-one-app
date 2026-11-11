//! What the routes for apps' screens and events say of themselves (api/features/src/says.rs).

use serde::Deserialize;

use ::api::call::Event;
use ::api::says::Op;
use ::api::says::Said;
use ::api::says::Who;

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
