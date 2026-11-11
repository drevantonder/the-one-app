//! Device logs, as an app runtime runs them: the tool `logs`, which asks the device named for its log through
//! `api::AppRuntime`.

mod tools;

pub fn runs() -> api::runtime::Runs {
    api::runtime::Runs { tools: tools::served(), ..api::runtime::Runs::named("logs") }
}
