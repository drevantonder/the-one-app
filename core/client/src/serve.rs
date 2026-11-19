//! What `one serve run` (`clients/cli/src/serving.rs`) and the server it keeps running (`OneCore.Release`) say to
//! each other: a line of JSON each, one of the messages here, which both read and write by this one definition, the
//! server's Elixir through its NIF (`core/server/nif/one_core_iroh`).
//!
//! The program tells the server what it is (`Told`), as it starts it, in `ONE_SERVER_STATUS`, and on its input as that
//! changes, and asks it for the latest builds. The server says on its output that it's running, that it's asked to
//! update, that it can reach the network again and the latest builds, and a build being checked says which it is
//! (`Said`). Taking a build asks nothing of any other server:
//! what to take is read by this one, and whether it answers is asked of it by its key.

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
/// on, and checking the build starts (preparing), checked and waiting to be told to go, and verifying them.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServerUpdateStatus {
    #[default]
    UpToDate,
    Downloading,
    Checked,
    Verifying,
}

/// Where downloading is once it's downloaded, checking the build starts, and where verifying is, starting it in the
/// server's place.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServerUpdateStage {
    Checking,
    Starting,
}

/// What a server is, all of it, each time the program says it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Status {
    pub server_membership: ServerMembership,
    pub server_daemon_status: ServerDaemonStatus,
    pub server_update_status: ServerUpdateStatus,
    /// Where verifying is, while it's verifying.
    pub server_update_stage: Option<ServerUpdateStage>,
    /// The build being prepared, checked or verified.
    pub next_build: Option<String>,
    /// The builds' name of the `one` that runs it, which it takes the latest of: `one-cli/linux-x86_64`.
    pub program_name: Option<String>,
    /// The build of the `one` running it, and of the one being prepared, checked or verified with `next_build`.
    pub program: Option<String>,
    pub next_program: Option<String>,
    /// Why the last update didn't take, which the server says until one does.
    pub went_back: Option<String>,
}

/// What the program tells the server.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(untagged)]
pub enum Told {
    /// Say the latest builds for this kind of machine (`Said::Latest`, `Said::NoLatest`).
    Latest { latest: String },
    /// What it is now.
    Status(Status),
}

/// What the server, or a build being checked, says.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Said {
    /// It has started, and answers by this key.
    Running { key: String },
    /// Its network's updater told it to prepare the latest builds: download and check them, and say how it takes them.
    Prepare,
    /// Its network's updater told it to take the builds it has checked.
    Go,
    /// The network's edge took what it said it is, as this build.
    Reported { build: String },
    /// The latest builds for the kind of machine it was asked, by what they are (`server-code`, `one`), each with a
    /// link to download it by, as it read them from the network's bucket.
    Latest { builds: serde_json::Map<String, serde_json::Value> },
    /// It couldn't read the latest builds.
    NoLatest { why: String },
    /// The build being checked started, loaded its NIFs, and is this one (`OneCore.Release.check/0`).
    Checked { build: String },
}

impl Said {
    /// The message a line the server said is, or `None` for anything else it says, as what it logs.
    pub fn read(line: &str) -> Option<Self> {
        serde_json::from_str(line).ok()
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
