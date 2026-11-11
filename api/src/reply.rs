//! How the API answers: the body a route takes, what failed as JSON, and an answer named by its content.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

pub struct Error(pub anyhow::Error);

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        if let Some(e) = self.0.downcast_ref::<Elsewhere>() {
            return (StatusCode::MISDIRECTED_REQUEST, Json(json!({ "error": e.why, "elsewhere": e.runtimes }))).into_response();
        }
        let status = match self.0.downcast_ref::<crate::handed::Needs>() {
            Some(_) => StatusCode::PRECONDITION_REQUIRED,
            None => StatusCode::BAD_REQUEST,
        };
        let mut answer = (status, Json(failed(&self.0))).into_response();
        if self.0.downcast_ref::<crate::handed::Needs>().is_none() {
            answer.extensions_mut().insert(RouteFailed { code: custom_build::protocol::Coded::of(&self.0), said: format!("{:#}", self.0), invalid: self.0.is::<Invalid>() });
        }
        answer
    }
}

/// A call a client device picked this device for that it can't run: the API answers 421 with the devices that can,
/// in the order to ask them (`{ error, elsewhere }`), which the client asks itself, handing each its own keys.
#[derive(Debug)]
pub struct Elsewhere {
    pub why: String,
    pub runtimes: Vec<String>,
}

impl std::fmt::Display for Elsewhere {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.why)
    }
}

impl std::error::Error for Elsewhere {}

/// How a route failed, for the span that says so: what goes back to its caller is only `failed`.
#[derive(Clone)]
pub struct RouteFailed {
    pub code: Option<String>,
    pub said: String,
    pub invalid: bool,
}

/// What was asked isn't something a route takes: the caller's to change, so nothing broke.
#[derive(Debug)]
pub struct Invalid(pub String);

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Invalid {}

/// Why something failed, as the API says it: `{ error }`, with `{ code }` for the kinds callers act on (`protocol::code`),
/// `{ trace }` for what to look up in insights, and `{ need, to, seal }` for keys a call wasn't handed.
pub fn failed(err: &anyhow::Error) -> Value {
    let mut said = match err.downcast_ref::<crate::handed::Needs>() {
        Some(n) => return n.json(),
        None => json!({ "error": format!("{err:#}") }),
    };
    if let Some(code) = custom_build::protocol::Coded::of(err) {
        said["code"] = json!(code);
    }
    if let Some(trace) = crate::trace::current() {
        said["trace"] = json!(trace.trace_id());
    }
    said
}

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(e: E) -> Self {
        Error(e.into())
    }
}

pub type Reply = Result<Json<Value>, Error>;

pub fn done() -> Reply {
    Ok(Json(json!({})))
}

/// A request's JSON body as `T`, whose type says every field a route takes (`#[serde(deny_unknown_fields)]`), so a
/// word it doesn't take (one renamed, say) fails rather than being passed over: a call or service meant for one device
/// running on another.
pub struct Body<T>(pub T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned> axum::extract::FromRequest<S> for Body<T> {
    type Rejection = Error;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Error> {
        let Json(body) = <Json<Value> as axum::extract::FromRequest<S>>::from_request(req, state).await.map_err(|e| anyhow::anyhow!("{}", e.body_text()))?;
        Ok(Body(serde_json::from_value(body).map_err(|e| anyhow::anyhow!("{e}"))?))
    }
}

/// `value`, trimmed, or what to give when it's blank.
pub fn given<'a>(value: &'a str, key: &str) -> Result<&'a str, Error> {
    Some(value.trim()).filter(|v| !v.is_empty()).ok_or_else(|| anyhow::anyhow!("give {key}").into())
}

/// An answer named by its content (`tag`, its ETag): to a client that has it already, only that.
pub fn tagged(headers: &axum::http::HeaderMap, tag: String, answer: impl IntoResponse) -> Response {
    if headers.get(axum::http::header::IF_NONE_MATCH).is_some_and(|had| had.as_bytes() == tag.as_bytes()) {
        return (StatusCode::NOT_MODIFIED, [(axum::http::header::ETAG, tag)]).into_response();
    }
    ([(axum::http::header::ETAG, tag)], answer).into_response()
}
