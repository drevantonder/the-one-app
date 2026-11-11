//! Groups, as an app runtime runs it: its two tools (groups/src/tools.rs).

mod tools;

use api::runtime::Runs;

pub fn runs() -> Runs {
    Runs { tools: tools::served(), ..Runs::named("groups") }
}
