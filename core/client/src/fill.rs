//! What a device fills in of a tool's inputs. A tool says which of its inputs a device fills in, by the input's
//! `format` in the tool's schema, and whatever calls the tool through this client (an app, the CLI, an agent over
//! MCP) gets them filled in the same way. One Core fills in `device` and `device-name`, this device's key and its
//! name, where none is given. Every other format is filled in by the native code of a plugin the program ships
//! with, a `Fill` it hands `Client::open`.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};

use crate::Client;

/// What a plugin's native code fills in on this device: inputs of the formats it names.
pub trait Fill: Send + Sync {
    /// The formats it fills in, in the order it does.
    fn formats(&self) -> &'static [&'static str];

    /// What it fills in for `input`, or nothing, which leaves it as it is.
    fn fill<'a>(&'a self, client: &'a Client, input: Input<'a>) -> BoxFuture<'a, Result<Option<String>>>;
}

/// One input to fill in, of a call of a tool of `plugin`: its `format`, what its schema `says`, what was given for
/// it (`typed`), the call's other inputs, and whoever the call is for (`who`), if it says.
pub struct Input<'a> {
    pub plugin: &'a str,
    pub format: &'a str,
    pub says: &'a Value,
    pub typed: Option<&'a str>,
    pub inputs: &'a Map<String, Value>,
    pub who: Option<&'a str>,
}

/// What One Core fills in itself, first: an input that's for a device, as a key for one is, is filled in after it.
const OWN: [&str; 2] = ["device", "device-name"];

/// The inputs a device fills in, by their tool's name: each one's name, and what its schema says of it.
pub(crate) type Fills = HashMap<String, Vec<(String, Value)>>;

/// The inputs a device fills in of each tool a server listed, of the `formats` it fills in, in their order.
pub(crate) fn fills(listed: &Value, formats: &[&str]) -> Fills {
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
    tools.iter().filter_map(filled).collect()
}

impl Client {
    /// Every format this device fills in, in the order it does: One Core's own, then each `Fill`'s.
    pub(crate) fn formats(&self) -> Vec<&'static str> {
        OWN.into_iter().chain(self.fillers.iter().flat_map(|filler| filler.formats().iter().copied())).collect()
    }

    /// The inputs a device fills in, as a server last listed its tools: asked for, if this client hasn't seen a list.
    async fn fills(&self) -> Result<Arc<Fills>> {
        if let Some(fills) = self.fills.lock().expect("no ask panics").clone() {
            return Ok(fills);
        }
        Box::pin(self.request("tools/list", json!({}))).await?;
        Ok(self.fills.lock().expect("no ask panics").clone().unwrap_or_default())
    }

    /// Fills in what this device does of a tool's inputs, if `request` calls a tool that has any.
    pub(crate) async fn fill(&self, request: &mut Value) -> Result<()> {
        let Some(tool) = request["params"]["name"].as_str().filter(|_| request["method"] == "tools/call") else {
            return Ok(());
        };
        let (tool, fills) = (tool.to_owned(), self.fills().await?);
        let Some(fills) = fills.get(&tool) else { return Ok(()) };
        let plugin = tool.split('_').next().unwrap_or_default();
        // What the device asks to fill these in is for whoever the call is.
        let who = request["params"]["_meta"]["for"].as_str().map(str::to_owned);
        let who = who.as_deref();
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
                _ => match self.fillers.iter().find(|filler| filler.formats().contains(&format)) {
                    Some(filler) => {
                        let input = Input { plugin, format, says, typed, inputs, who };
                        filler.fill(self, input).await?
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
