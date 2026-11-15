use std::process::ExitCode;

/// One program: asked nothing, it's the app, and asked anything, it's the CLI and MCP. Built without its window, it's
/// only those.
fn main() -> ExitCode {
    #[cfg(feature = "window")]
    if std::env::args().len() == 1 {
        shell::run();
        return ExitCode::SUCCESS;
    }
    cli::main()
}
