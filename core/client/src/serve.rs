//! What `one serve run` (`clients/cli/src/serving.rs`) and the server it keeps running (`OneCore.Release`) say to
//! each other: a line of JSON each, one of the messages here, which both read and write by this one definition, the
//! server's Elixir through its NIF (`core/server/nif/one_core_iroh`).
//!
//! The program tells the server what it is (`Told`), as it starts it, in `ONE_SERVER_STATUS`, and on its input as that
//! changes, and has it load a build. The server says on its output that it's running, that it's asked to update, and
//! whether it loaded a build, and a build being checked says which it is and how it's taken (`Said`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Whether the device is one of its network's servers: joining while `one serve on` makes it one, until it answers
/// by its key, active, and leaving while `one serve off` drains it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServerMembership {
    #[default]
    Inactive,
    Joining,
    Active,
    Leaving,
}

/// The program the system keeps running: starting until the server answers by its key, running, and stopping.
/// Stopped is what the network's edge says of one that's gone.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServerDaemonStatus {
    Starting,
    Running,
    Stopping,
}

/// Where an update is: up to date, downloading the latest server build and `one` together while the server answers
/// on, and verifying them.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServerUpdateStatus {
    #[default]
    UpToDate,
    Downloading,
    Verifying,
}

/// Where verifying is: checking the build starts and how it's taken, then loading it or starting it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServerUpdateStage {
    Checking,
    Loading,
    Starting,
}

/// How a build is taken, as checking chose (`OneCore.Release.compare/2`): loaded into the server while it runs, as
/// a build that changed only One Core's code is, or started in its place, with `one` if that changed too.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMethod {
    HotLoad,
    Restart,
}

/// What a server is, all of it, each time the program says it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Status {
    pub server_membership: ServerMembership,
    pub server_daemon_status: ServerDaemonStatus,
    pub server_update_status: ServerUpdateStatus,
    /// Where verifying is, while it's verifying.
    pub server_update_stage: Option<ServerUpdateStage>,
    /// How the last update was taken, or is being.
    pub update_method: Option<UpdateMethod>,
    /// Why the last update didn't take, which the server says until one does.
    pub went_back: Option<String>,
}

/// What the program tells the server.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(untagged)]
pub enum Told {
    /// Load the build in this folder while it runs, and say whether it did (`Said::Loaded`, `Said::NotLoaded`).
    Load { load: PathBuf },
    /// What it is now.
    Status(Status),
}

/// What the server, or a build being checked, says.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Said {
    /// It has started, and answers by this key.
    Running { key: String },
    /// Someone asked it to take the latest builds.
    UpdateAsked,
    /// It loaded the build it was told to, and is that build now.
    Loaded { build: String },
    /// It couldn't load the build it was told to, and is on the code it had.
    NotLoaded { build: String, why: String },
    /// The build being checked started, loaded its NIFs, and is this one (`OneCore.Release.check/0`).
    Checked { build: String },
    /// How a server on another build takes this one (`OneCore.Release.compare/2`), and why, where it restarts.
    Compared { update_method: UpdateMethod, why: Option<String> },
}

impl Said {
    /// The message a line the server said is, or `None` for anything else it says, as what it logs.
    pub fn read(line: &str) -> Option<Self> {
        serde_json::from_str(line).ok().or_else(|| Self::read_text(line.trim()))
    }

    /// The message a line is in the text a server said before these were JSON, which one still says to a program
    /// that doesn't say all of what the server is (`Told::Status`).
    fn read_text(line: &str) -> Option<Self> {
        if let Some(key) = line.strip_prefix("A server is running: ") {
            Some(Self::Running { key: key.to_owned() })
        } else if line == "Update asked" {
            Some(Self::UpdateAsked)
        } else if let Some(build) = line.strip_prefix("Loaded ") {
            Some(Self::Loaded { build: build.to_owned() })
        } else if let Some(not) = line.strip_prefix("Didn't load ") {
            let (build, why) = not.split_once(": ").unwrap_or((not, ""));
            Some(Self::NotLoaded { build: build.to_owned(), why: why.to_owned() })
        } else if line == "hot_load" {
            Some(Self::Compared { update_method: UpdateMethod::HotLoad, why: None })
        } else {
            let why = line.strip_prefix("restart: ")?;
            Some(Self::Compared { update_method: UpdateMethod::Restart, why: Some(why.to_owned()) })
        }
    }
}

impl std::fmt::Display for Said {
    /// The line it's said in.
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&serde_json::to_string(self).map_err(|_| std::fmt::Error)?)
    }
}

impl std::fmt::Display for Told {
    /// The line it's told in.
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&serde_json::to_string(self).map_err(|_| std::fmt::Error)?)
    }
}
