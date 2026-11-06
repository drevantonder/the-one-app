//! Files kept in the network on the command line.

use anyhow::{Context, bail};
use serde_json::Value;

use crate::cli::Command;

pub fn commands() -> Command {
    Command {
        help: vec![
            ("file put <file>".into(), "keep a file in the network, for apps and agents; prints its hash".into()),
            ("file get <hash> <file>".into(), "get a kept file from this device or another".into()),
        ],
        run: |g| {
            match g.words().as_slice() {
                ["file", "put", file] => println!("{}", g.node.put_blob(&std::fs::read(file).with_context(|| format!("read {file}"))?)?),
                ["file", "get", hash, file] => {
                    let mut answer = g.node.open(&format!("/v1/blobs/{hash}"), None)?;
                    if !(200..300).contains(&answer.status) {
                        bail!("{}", serde_json::from_reader::<_, Value>(answer.body)?["error"].as_str().unwrap_or("failed"));
                    }
                    std::io::copy(&mut answer.body, &mut std::fs::File::create(file).with_context(|| format!("write {file}"))?)?;
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}
