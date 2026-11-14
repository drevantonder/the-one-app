use std::process::ExitCode;

/// One program: asked nothing, it's the app, and asked anything, it's the CLI and MCP.
fn main() -> ExitCode {
    if std::env::args().len() > 1 {
        return client::cli::main();
    }
    shell::run();
    ExitCode::SUCCESS
}
