//! `one system`: what the system keeps running in its person's session, as `one serve` keeps its server
//! (`client::system`), for a server's services, which ask the `one` that runs their server (`OneCore.System`): a
//! NIF would never hear how `systemctl`, `launchctl` or `schtasks` ended, as the VM ignores SIGCHLD. A program is JSON,
//! as `Program` says; `install` says whether it changed, `true` or `false`.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use serde::Deserialize;

#[derive(Deserialize)]
struct Program {
    name: String,
    key: String,
    description: String,
    command: Vec<String>,
    env: Vec<(String, String)>,
    folder: PathBuf,
    desktop: bool,
    /// In milliseconds, as `stop_within`.
    start_within: Option<u64>,
    stop_within: u64,
}

impl From<Program> for client::system::Program {
    fn from(program: Program) -> Self {
        Self {
            name: program.name,
            key: program.key,
            description: program.description,
            command: program.command,
            env: program.env,
            folder: program.folder,
            desktop: program.desktop,
            start_within: program.start_within.map(Duration::from_millis),
            stop_within: Duration::from_millis(program.stop_within),
        }
    }
}

pub fn run(args: &[String]) -> Result<String> {
    let program = |json: &str| -> Result<client::system::Program> { Ok(serde_json::from_str::<Program>(json)?.into()) };
    Ok(match args {
        [install, json] if install == "install" => client::system::install(&program(json)?)?.to_string(),
        [start, json] if start == "start" => client::system::start(&program(json)?).map(|()| String::new())?,
        [restart, json] if restart == "restart" => client::system::restart(&program(json)?).map(|()| String::new())?,
        [stop, name, key] if stop == "stop" => client::system::stop(name, key).map(|()| String::new())?,
        [remove, name, key] if remove == "remove" => client::system::remove(name, key).map(|()| String::new())?,
        _ => bail!("one system install|start|restart <program as JSON>, or stop|remove <name> <key>"),
    })
}
