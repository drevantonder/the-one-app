use std::process::ExitCode;

/// One program: asked nothing, it's the app, and asked anything, it's the CLI and MCP. Built without its window, it's
/// only those. A `one:` link alone is the app's: Linux and Windows open a link with its program by giving it as the
/// only argument.
fn main() -> ExitCode {
    #[cfg(feature = "window")]
    if std::env::args().len() == 1 || std::env::args().len() == 2 && std::env::args().nth(1).is_some_and(|arg| arg.starts_with("one:")) {
        app::run();
        return ExitCode::SUCCESS;
    }
    cli::main()
}
