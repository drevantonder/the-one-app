use std::process::ExitCode;

/// One program: asked nothing, it's the app, and asked anything, it's the CLI and MCP. Built without its window, it's
/// only those. A `one:` link alone is the app's: Linux and Windows open a link with its program by giving it as the
/// only argument.
fn main() -> ExitCode {
    // An app's own window, as the app's window or the system starts it (`app::host`).
    #[cfg(all(feature = "window", not(mobile)))]
    if let (Some(entrypoint), 1) = (std::env::var("ONE_APP").ok(), std::env::args().len()) {
        let link = std::env::var("ONE_OPEN").unwrap_or_else(|_| entrypoint.clone());
        return app::host::run(entrypoint, std::env::var("ONE_SUBJECT").ok(), link);
    }
    #[cfg(feature = "window")]
    if std::env::args().len() == 1 || std::env::args().len() == 2 && std::env::args().nth(1).is_some_and(|arg| arg.starts_with("one:")) {
        app::run();
        return ExitCode::SUCCESS;
    }
    cli::main()
}
