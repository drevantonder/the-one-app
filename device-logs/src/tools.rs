//! Device logs, as the core's tool `logs`: a device's log, or one of its services', read over the network.

use serde_json::json;

use ::api::Feature;

pub fn feature() -> Feature {
    Feature {
        name: "logs",
        title: "Device logs",
        description: "A device's log, or one of its services', over the network".into(),
        guide: String::new(),
        tools: vec![json!({
                "name": "logs", "title": "Read a device's log", "reads": true,
                "description": "Read a device's log over the network, instead of ssh: its node's, or one of its services', or for a phone or computer app, what its app and page say went wrong (read while the app is open and connected). The newest lines (200 by default, at most 1000 or 48 KiB), oldest first. Only the device's user and the network's admins may.",
                "input": { "type": "object", "required": ["device"], "properties": {
                    "device": { "type": "string", "description": "the device's name (devices_requests and groups_list name them; the status command lists them)" },
                    "service": { "type": "string", "description": "a service's name, like whatsapp; omit for the node's own log, and for an app's" },
                    "since": { "type": "string", "description": "only the last this long: 30s, 10m, 2h or 1d" },
                    "grep": { "type": "string", "description": "only lines with this text in them, whatever its case" },
                    "lines": { "type": "integer", "description": "how many of the newest to give" },
                } },
                "output": { "type": "object", "required": ["said"], "properties": { "said": { "type": "string", "description": "the lines, oldest first, then a note when there are none or more than shown" } } },
            })],
    }
}
