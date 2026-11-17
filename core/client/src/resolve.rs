//! What a device fills in of a tool's inputs. A tool says which of its inputs a device fills in, by the input's
//! `format` in the tool's schema, and whatever calls the tool through this client (an app, the CLI, an agent over
//! MCP) gets them filled in the same way. One Core fills in `device` and `device-name`, this device's key and its
//! name, where none is given. Every other format is filled in by the native code of an app the program ships
//! with, an `InputResolver` it hands `Client::open`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};

use crate::Client;

/// What an app's native code fills in on this device: inputs of the formats it names.
pub trait InputResolver: Send + Sync {
    /// The formats it fills in, in the order it does.
    fn formats(&self) -> &'static [&'static str];

    /// What it fills in for `input`, or nothing, which leaves it as it is.
    fn resolve<'a>(&'a self, client: &'a Client, input: Input<'a>) -> BoxFuture<'a, Result<Option<String>>>;
}

/// One input to fill in, of a call of a tool of `app`: its `format`, what its schema `says`, what was given for
/// it (`typed`), the call's other inputs, and whoever the call is for (`subject`), if it says.
pub struct Input<'a> {
    pub app: &'a str,
    pub format: &'a str,
    pub says: &'a Value,
    pub typed: Option<&'a str>,
    pub inputs: &'a Map<String, Value>,
    pub subject: Option<&'a str>,
}

/// What One Core fills in itself, first: an input that's for a device, as a key for one is, is filled in after it.
const OWN: [&str; 2] = ["device", "device-name"];

/// What a client knows of the tools a server listed: the inputs a device fills in, by their tool's name, each with
/// its name and what its schema says of it, and the tools that only read.
#[derive(Default)]
pub(crate) struct Listed {
    fills: HashMap<String, Vec<(String, Value)>>,
    reads: HashSet<String>,
}

/// The methods that only read, besides a tool that says it does.
pub(crate) const READS: [&str; 3] = ["tools/list", "resources/list", "resources/read"];

/// What a client knows of the tools a server listed, as it fills in their inputs of the `formats` it does, in their
/// order.
pub(crate) fn listed(listed: &Value, formats: &[&str]) -> Listed {
    let tools = listed["tools"].as_array().map(Vec::as_slice).unwrap_or_default();
    let filled = |tool: &Value| {
        let inputs = tool["inputSchema"]["properties"].as_object()?;
        let mut fills: Vec<(String, Value)> = inputs
            .iter()
            .filter(|(_, input)| formats.iter().any(|format| input["format"] == *format))
            .map(|(name, input)| (name.clone(), input.clone()))
            .collect();
        fills.sort_by_key(|(_, input)| formats.iter().position(|format| input["format"] == *format));
        (!fills.is_empty()).then(|| Some((tool["name"].as_str()?.to_owned(), fills)))?
    };
    let reads = tools.iter().filter(|tool| tool["annotations"]["readOnlyHint"] == true);
    Listed {
        fills: tools.iter().filter_map(filled).collect(),
        reads: reads.filter_map(|tool| Some(tool["name"].as_str()?.to_owned())).collect(),
    }
}

impl Client {
    /// Every format this device fills in, in the order it does: One Core's own, then each `InputResolver`'s.
    pub(crate) fn formats(&self) -> Vec<&'static str> {
        OWN.into_iter().chain(self.resolvers.iter().flat_map(|resolver| resolver.formats().iter().copied())).collect()
    }

    /// What this client knows of the tools, as a server last listed them: asked for, if it hasn't seen a list.
    async fn listed(&self) -> Result<Arc<Listed>> {
        if let Some(listed) = self.listed.lock().expect("no ask panics").clone() {
            return Ok(listed);
        }
        Box::pin(self.request("tools/list", json!({}))).await?;
        Ok(self.listed.lock().expect("no ask panics").clone().unwrap_or_default())
    }

    /// Whether `request` only reads: one of `READS`, or a call of a tool that says it does.
    pub(crate) async fn reads(&self, request: &Value) -> Result<bool> {
        Ok(match request["method"].as_str() {
            Some("tools/call") => {
                let tool = request["params"]["name"].as_str().unwrap_or_default();
                self.listed().await?.reads.contains(tool)
            }
            Some(method) => READS.contains(&method),
            None => false,
        })
    }

    /// Fills in what this device does of a tool's inputs, if `request` calls a tool that has any.
    pub(crate) async fn resolve(&self, request: &mut Value) -> Result<()> {
        let Some(tool) = request["params"]["name"].as_str().filter(|_| request["method"] == "tools/call") else {
            return Ok(());
        };
        let (tool, listed) = (tool.to_owned(), self.listed().await?);
        let Some(fills) = listed.fills.get(&tool) else { return Ok(()) };
        let app = tool.split('_').next().unwrap_or_default();
        // What the device asks to fill these in is for whoever the call is.
        let subject = request["params"]["_meta"]["subject"].as_str().map(str::to_owned);
        let subject = subject.as_deref();
        if !request["params"]["arguments"].is_object() {
            request["params"]["arguments"] = json!({});
        }
        let inputs = request["params"]["arguments"].as_object_mut().expect("it was made one");
        for (name, says) in fills {
            let format = says["format"].as_str().unwrap_or_default();
            let typed = inputs.get(name).and_then(Value::as_str);
            let filled = match (format, typed) {
                ("device", None) => Some(self.key.public().to_string()),
                ("device-name", None) => Some(crate::name()),
                _ => match self.resolvers.iter().find(|resolver| resolver.formats().contains(&format)) {
                    Some(resolver) => {
                        let input = Input { app, format, says, typed, inputs, subject };
                        resolver.resolve(self, input).await?
                    }
                    None => None,
                },
            };
            if let Some(filled) = filled {
                inputs.insert(name.clone(), filled.into());
            }
        }
        Ok(())
    }
}
