//! Widgets, as an app runtime runs them: every widget and badge of the apps a page shows, drawn for who's looking
//! (`draw`), their tool (`tools`) and their routes (`routes`).

mod draw;
mod routes;
mod tools;

use api::runtime::Runs;

pub fn runs() -> Runs {
    Runs { tools: tools::served(), routes: routes::routes(), ..Runs::named("widgets") }
}
