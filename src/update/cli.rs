//! `one update`, at the command line.

use crate::cli::Command;
use crate::config::CLI;

pub fn commands() -> Command {
    Command {
        help: vec![("update".into(), "put this device on the newest release, and restart it".into())],
        run: |g| {
            if g.words().as_slice() != ["update"] {
                return Ok(false);
            }
            match super::run() {
                Ok(said) => println!("{said}"),
                // Printed here rather than returned: the usual error path asks the background program for its tools' names, which could be what's stuck.
                Err(err) => {
                    eprintln!("{CLI}: {err:#}");
                    std::process::exit(1);
                }
            }
            Ok(true)
        },
    }
}
