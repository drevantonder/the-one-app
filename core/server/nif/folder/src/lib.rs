//! A folder's files as a client deploys them (`client::files`), so a plugin a server ships with is deployed with
//! what a deploy from that folder would send.

use std::path::Path;

#[rustler::nif(schedule = "DirtyIo")]
fn files(folder: String) -> Result<Vec<String>, String> {
    client::files(Path::new(&folder)).map_err(|error| format!("{error:#}"))
}

include!(concat!(env!("OUT_DIR"), "/init.rs"));
