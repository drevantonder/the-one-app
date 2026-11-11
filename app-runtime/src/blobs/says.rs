//! What the routes for files kept in the network say of themselves (api/features/src/says.rs).

use serde_json::json;

use ::api::says::Op;
use ::api::says::Said;
use ::api::says::Who;

pub fn listed() -> Vec<Said> {
    vec![
        ("/api/blobs", Who::Clients, Op::post("Keep a file, read as it comes").takes_file().answers_as(json!({ "type": "object", "required": ["blob", "size"], "properties": { "blob": { "type": "string", "description": "its hash" }, "size": { "type": "integer", "description": "bytes" } } }))),
        ("/api/blobs/{hash}", Who::Clients, Op::get("Read back a kept file").file()),
    ]
}
