//! What the page's JS calls, as a native host's sandbox has them (`globalThis.mutations`, `host.post`), and how a JS
//! value is read as JSON.

use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use serde_json::Value;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use super::thread::{channel, wake};
use crate::js::Posted;
use crate::tree::{Kind, Mutation};

// What the page's JS calls, as a native host's sandbox has them (`globalThis.mutations`, `host.post`), each queued for
// the screen to apply after its turn.

fn posted(posted: Posted) {
    channel(None, |channel| channel.posted.push(posted));
    wake(None);
}

#[wasm_bindgen(js_name = create)]
pub fn create(id: f64, kind: &str, value: Option<String>) -> Result<(), JsError> {
    let kind = Kind::deserialize(kind.into_deserializer()).map_err(|error: serde::de::value::Error| JsError::new(&error.to_string()))?;
    posted(Posted::Mutation(Mutation::Create { id: id as u64, kind, value }));
    Ok(())
}

/// A value that holds itself, or is nested past what's read, is the page's error, thrown to it, as JSON.stringify throws
/// one, as a native host's sandbox throws it.
#[wasm_bindgen(js_name = set)]
pub fn set(id: f64, name: String, value: JsValue) -> Result<(), JsValue> {
    let value = data(&value, &mut Vec::new()).map_err(|why| JsValue::from(js_sys::TypeError::new(why)))?;
    posted(Posted::Mutation(Mutation::Set { id: id as u64, name, value }));
    Ok(())
}

#[wasm_bindgen(js_name = insert)]
pub fn insert(parent: f64, id: f64, before: Option<f64>) {
    posted(Posted::Mutation(Mutation::Insert { parent: parent as u64, id: id as u64, before: before.map(|before| before as u64) }));
}

#[wasm_bindgen(js_name = remove)]
pub fn remove(parent: f64, id: f64) {
    posted(Posted::Mutation(Mutation::Remove { parent: parent as u64, id: id as u64 }));
}

#[wasm_bindgen(js_name = drop)]
pub fn drop_node(id: f64) {
    posted(Posted::Mutation(Mutation::Drop { id: id as u64 }));
}

/// `host.post`: a JSON-RPC message for the page's client, as JSON.
#[wasm_bindgen(js_name = post)]
pub fn post(json: String) {
    posted(Posted::Message(json));
}

/// How deep a value's arrays and objects are read at most, as each is read on the stack.
const NESTED_MOST: usize = 512;

/// A JS value as JSON's data model, read from it directly, as a native host's sandbox reads one (`js::quickjs`): a
/// function, a symbol or `undefined` is `null`, and a number that's whole is an integer. `within` is the arrays and
/// objects it's in. → why it can't be: it holds itself, or it's nested too deep.
pub(super) fn data(value: &JsValue, within: &mut Vec<JsValue>) -> Result<Value, &'static str> {
    if let Some(bool) = value.as_bool() {
        return Ok(Value::Bool(bool));
    }
    if let Some(number) = value.as_f64() {
        return Ok(if number.fract() == 0.0 && number.abs() < 9_007_199_254_740_992.0 { Value::from(number as i64) } else { serde_json::Number::from_f64(number).map_or(Value::Null, Value::Number) });
    }
    if let Some(text) = value.as_string() {
        return Ok(Value::String(text));
    }
    if !value.is_object() || value.is_function() {
        return Ok(Value::Null);
    }
    if within.iter().any(|outer| js_sys::Object::is(outer, value)) {
        return Err("cyclic object value");
    }
    if within.len() >= NESTED_MOST {
        return Err("object nested too deeply");
    }
    within.push(value.clone());
    let read = if let Some(array) = value.dyn_ref::<js_sys::Array>() {
        array.iter().map(|item| data(&item, within)).collect::<Result<_, _>>().map(Value::Array)
    } else {
        js_sys::Object::entries(value.unchecked_ref())
            .iter()
            .filter_map(|entry| {
                let entry: js_sys::Array = entry.unchecked_into();
                let field = entry.get(1);
                (!field.is_undefined() && !field.is_function()).then(|| Ok((entry.get(0).as_string().unwrap_or_default(), data(&field, within)?)))
            })
            .collect::<Result<_, _>>()
            .map(Value::Object)
    };
    within.pop();
    read
}
