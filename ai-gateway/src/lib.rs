//! AI Gateway (GLOSSARY): how apps use AI models with `ctx.ai.run`. This crate is what every build has of it: the
//! input each kind takes and the answer it gives (OpenAI's chat completions; Cloudflare Clef's decisions), which model id
//! runs where (`route`), the requests that sends and how their answers read (`google`, `anthropic` translate; the rest
//! are OpenAI's shape already), what a call's span may say, and its tools' specs. It sends nothing and holds no key.

mod anthropic;
mod google;
pub mod route;

use serde_json::{Map, Value, json};

pub use route::{Company, Route};

pub const NAME: &str = "ai-gateway";
pub const TITLE: &str = "AI Gateway";
pub const DESCRIPTION: &str = "How your apps use AI models: hosted ones with keys in the Vault, and Workers AI";
/// A key's Vault kind starts with this, then the company's id: `AI key: google`.
pub const KEY_KIND: &str = "AI key: ";
/// The most a request may hold: Gemini's inline limit, the smallest of the hosted ones.
pub const MOST_BYTES: usize = 20 << 20;
/// How long a call may take, as `ctx.waitUntil` may.
pub const MOST_SECONDS: u64 = 600;
/// How long an answer kept for a `retryKey` holds.
pub const RETRY_KEPT_MS: u64 = 86_400_000;
/// Each kind's model when neither the caller nor Everyone set one: what every network has from its first deploy.
pub const DEFAULT_CHAT: &str = "workers-ai/@cf/openai/gpt-oss-120b";
pub const DEFAULT_DECIDE: &str = "workers-ai/@cf/cloudflare/clef-flash";

/// What a model answers: chat (OpenAI's chat completions) or decide (Cloudflare Clef's questions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Chat,
    Decide,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Chat => "chat",
            Kind::Decide => "decide",
        }
    }

    /// What each kind takes, besides `model`.
    fn takes(self) -> &'static [&'static str] {
        match self {
            Kind::Chat => &["messages", "response_format", "max_completion_tokens", "temperature", "top_p", "stop", "seed", "reasoning_effort", "service_tier"],
            Kind::Decide => &["state", "questions", "images"],
        }
    }
}

/// Whose key paid for a call: an app's own, the caller's, the network's (Everyone's), or a device's ($0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paid {
    App,
    User,
    Network,
    Device,
}

impl Paid {
    pub fn as_str(self) -> &'static str {
        match self {
            Paid::App => "app",
            Paid::User => "user",
            Paid::Network => "network",
            Paid::Device => "device",
        }
    }
}

/// How a call failed, each a code apps catch (`err.code`) but `Broke`, which is the company's or ours.
#[derive(Clone, Debug, PartialEq)]
pub enum Failed {
    /// Asked for what isn't there or isn't taken: a model, an input, something not yet built.
    Refused(String),
    /// No key pays for that company's models.
    NoKey(String),
    /// The company said too many (429): how long to wait, when it said.
    Limit { said: String, retry_after: Option<u64> },
    /// A `retryKey` used before for other input.
    RetryChanged,
    /// A `retryKey` whose first call was sent and never answered: it may have run.
    Uncertain,
    /// The company failed, or refused its key.
    Broke(String),
}

impl Failed {
    /// Its code, as `protocol::code` names it; none when it broke.
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Failed::Refused(_) => Some("refused"),
            Failed::NoKey(_) => Some("no_key"),
            Failed::Limit { .. } => Some("limit"),
            Failed::RetryChanged => Some("retry_changed"),
            Failed::Uncertain => Some("uncertain"),
            Failed::Broke(_) => None,
        }
    }
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failed::Refused(said) | Failed::NoKey(said) | Failed::Broke(said) => f.write_str(said),
            Failed::Limit { said, retry_after: Some(s) } => write!(f, "{said}: try again in {s}s"),
            Failed::Limit { said, retry_after: None } => f.write_str(said),
            Failed::RetryChanged => f.write_str("That retryKey was used for another input: give a new one"),
            Failed::Uncertain => f.write_str("It may have run: its answer never came back. Give a new retryKey to send it again"),
        }
    }
}

impl std::error::Error for Failed {}

/// The default model of each kind, as the caller's own and Everyone's settings give them.
#[derive(Clone, Debug, Default)]
pub struct Defaults {
    pub chat: Option<String>,
    pub decide: Option<String>,
}

impl Defaults {
    pub fn model(&self, kind: Kind) -> String {
        match kind {
            Kind::Chat => self.chat.clone().unwrap_or_else(|| DEFAULT_CHAT.into()),
            Kind::Decide => self.decide.clone().unwrap_or_else(|| DEFAULT_DECIDE.into()),
        }
    }
}

/// A request, built and not yet filled: its placeholders are the Vault's.
#[derive(Clone, Debug)]
pub struct Outgoing {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A call as asked: its model resolved, its input checked.
#[derive(Clone, Debug)]
pub struct Asked {
    pub route: Route,
    pub kind: Kind,
    pub input: Map<String, Value>,
}

/// Reads `ctx.ai.run`'s input: `model` or the kind's default (by `messages` or `questions`), and only what that kind
/// takes. What isn't built yet is refused, saying so.
pub fn ask(input: Value, defaults: &Defaults) -> Result<Asked, Failed> {
    let Value::Object(mut input) = input else { return Err(Failed::Refused("ctx.ai.run takes an object: { model, messages } to chat, { model, state, questions } to decide".into())) };
    if serde_json::to_vec(&input).map_or(0, |b| b.len()) > MOST_BYTES {
        return Err(Failed::Refused(format!("that input is over {} MB, the most a model call takes", MOST_BYTES >> 20)));
    }
    let model = match input.remove("model") {
        Some(Value::String(m)) => m,
        Some(_) => return Err(Failed::Refused("model is a model id, like workers-ai/@cf/cloudflare/clef-flash".into())),
        None if input.contains_key("questions") => defaults.model(Kind::Decide),
        None => defaults.model(Kind::Chat),
    };
    let route = Route::parse(&model)?;
    let kind = route.kind();
    for key in input.keys() {
        let not_yet = match key.as_str() {
            "stream" => Some("streaming"),
            "tools" | "tool_choice" | "functions" => Some("tools (function calling)"),
            "modalities" | "audio" => Some("audio and images out"),
            "input" | "dimensions" => Some("embeddings"),
            _ => None,
        };
        if let Some(what) = not_yet {
            return Err(Failed::Refused(format!("AI Gateway doesn't do {what} yet: ask for one answer")));
        }
        if !kind.takes().contains(&key.as_str()) {
            return Err(Failed::Refused(format!("{model} is a {} model, which takes {}, not {key}", kind.as_str(), kind.takes().join(", "))));
        }
    }
    let needs = match kind {
        Kind::Chat => "messages",
        Kind::Decide => "questions",
    };
    if !input.contains_key(needs) {
        return Err(Failed::Refused(format!("{model} is a {} model: give {needs}", kind.as_str())));
    }
    Ok(Asked { route, kind, input })
}

impl Asked {
    /// What a `retryKey` is held to: the same model and input, as JSON with its keys in order.
    pub fn canonical(&self) -> String {
        json!({ "model": self.route.id(), "input": self.input }).to_string()
    }

    /// The request to send, paid with the Vault item `key`.
    pub fn request(&self, key: &str) -> Result<Outgoing, Failed> {
        let body = match self.route.company {
            Company::Google => google::request(&self.input)?,
            Company::Anthropic => anthropic::request(&self.route.model, &self.input)?,
            Company::WorkersAi if self.kind == Kind::Decide => Value::Object(self.input.clone()),
            Company::OpenAi | Company::OpenRouter | Company::WorkersAi => {
                let mut body = self.input.clone();
                body.insert("model".into(), json!(self.route.model));
                Value::Object(body)
            }
        };
        Ok(self.route.request(key, body))
    }

    /// Its answer, in the kind's own shape, from what the company said: its status, `retry-after` and body.
    pub fn answer(&self, status: u16, retry_after: Option<u64>, body: &[u8]) -> Result<Value, Failed> {
        let title = self.route.company.title();
        if status == 429 {
            let day = String::from_utf8_lossy(body).to_lowercase().contains("day") || retry_after.is_some_and(|s| s > 3600);
            return Err(Failed::Limit { said: format!("AI limit: {} ({title})", if day { "day" } else { "minute" }), retry_after });
        }
        if !(200..300).contains(&status) {
            let text: String = String::from_utf8_lossy(body).chars().take(200).collect();
            return Err(Failed::Broke(format!("{title} answered {status}: {text}")));
        }
        let got: Value = serde_json::from_slice(body).map_err(|_| Failed::Broke(format!("{title} answered with what isn't JSON")))?;
        let answer = match (self.route.company, self.kind) {
            (Company::Google, _) => google::answer(&self.route.model, got),
            (Company::Anthropic, _) => anthropic::answer(got),
            (Company::WorkersAi, Kind::Decide) => got.get("result").cloned().unwrap_or(got),
            _ => got,
        };
        let fits = match self.kind {
            Kind::Chat => answer["choices"].is_array(),
            Kind::Decide => answer["answers"].is_object(),
        };
        if !fits {
            return Err(Failed::Broke(format!("{title} answered in a shape that isn't a {} answer", self.kind.as_str())));
        }
        Ok(answer)
    }
}

/// What an answer says it used.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub finish_reasons: Vec<String>,
    pub model: Option<String>,
}

pub fn usage(kind: Kind, answer: &Value) -> Usage {
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let u = &answer["usage"];
    match kind {
        Kind::Chat => Usage {
            input: n(&u["prompt_tokens"]),
            output: n(&u["completion_tokens"]),
            cache_read: n(&u["prompt_tokens_details"]["cached_tokens"]),
            finish_reasons: answer["choices"].as_array().into_iter().flatten().filter_map(|c| c["finish_reason"].as_str().map(str::to_string)).collect(),
            model: answer["model"].as_str().map(str::to_string),
        },
        Kind::Decide => Usage { input: n(&u["input_tokens"]), output: n(&u["output_tokens"]), ..Usage::default() },
    }
}

/// The attributes a call's GenAI span may have, and only these: OTEL's `gen_ai.*` and ours. Never a prompt or an answer.
pub const SPAN_ATTRS: &[&str] = &[
    "gen_ai.operation.name",
    "gen_ai.provider.name",
    "gen_ai.request.model",
    "gen_ai.response.model",
    "gen_ai.usage.input_tokens",
    "gen_ai.usage.output_tokens",
    "gen_ai.usage.cache_read.input_tokens",
    "gen_ai.response.finish_reasons",
    "gen_ai.output.type",
    "gen_ai.request.max_tokens",
    "gen_ai.request.temperature",
    "server.address",
    "error.type",
    "one.code",
    "one.paid",
    "one.tier",
    "one.replayed",
    // Only a metric's: which tokens it counts.
    "gen_ai.token.type",
];

/// A call's span's attributes: what it asked, who paid, and what its answer used (none for a replay or a failure).
pub fn span_attrs(asked: &Asked, paid: Option<Paid>, used: Option<&Usage>, replayed: bool) -> Vec<(&'static str, Value)> {
    let mut out = vec![
        ("gen_ai.operation.name", json!(asked.kind.as_str())),
        ("gen_ai.provider.name", json!(asked.route.company.otel())),
        ("gen_ai.request.model", json!(asked.route.id())),
        ("server.address", json!(asked.route.company.host())),
    ];
    if let Some(paid) = paid {
        out.push(("one.paid", json!(paid.as_str())));
    }
    if asked.input.get("service_tier").and_then(Value::as_str) == Some("flex") {
        out.push(("one.tier", json!("flex")));
    }
    if let Some(n) = asked.input.get("max_completion_tokens").and_then(Value::as_u64) {
        out.push(("gen_ai.request.max_tokens", json!(n)));
    }
    if let Some(t) = asked.input.get("temperature").and_then(Value::as_f64) {
        out.push(("gen_ai.request.temperature", json!(t)));
    }
    if asked.kind == Kind::Chat {
        let typed = asked.input.get("response_format").is_some_and(|f| f["type"] != "text");
        out.push(("gen_ai.output.type", json!(if typed { "json" } else { "text" })));
    }
    if replayed {
        out.push(("one.replayed", json!(true)));
    }
    if let Some(u) = used {
        out.push(("gen_ai.usage.input_tokens", json!(u.input)));
        out.push(("gen_ai.usage.output_tokens", json!(u.output)));
        if u.cache_read > 0 {
            out.push(("gen_ai.usage.cache_read.input_tokens", json!(u.cache_read)));
        }
        if !u.finish_reasons.is_empty() {
            out.push(("gen_ai.response.finish_reasons", json!(u.finish_reasons)));
        }
        if let Some(m) = &u.model {
            out.push(("gen_ai.response.model", json!(m)));
        }
    }
    out
}

/// A `data:<mime>;base64,<data>` URL's two parts.
fn data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

/// A chat message's content as parts: text alone is one part.
fn parts(content: &Value) -> Vec<Value> {
    match content {
        Value::String(text) => vec![json!({ "type": "text", "text": text })],
        Value::Array(all) => all.clone(),
        _ => Vec::new(),
    }
}

/// A message's text, for a system message, which is text only.
fn text_of(content: &Value) -> String {
    parts(content).iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n")
}

/// The tools' specs: core tools, `ai-gateway_<tool>`.
pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "run", "title": "Ask a model",
            "description": "Ask an AI model what an app's ctx.ai.run would, as you, from no app: for trying a model or a prompt. Chat models take OpenAI's chat completions input (messages, response_format, max_completion_tokens, temperature, top_p, stop, seed, reasoning_effort, service_tier \"flex\") and answer OpenAI's ChatCompletion; decide models (workers-ai/@cf/cloudflare/clef, clef-flash) take state and questions and answer { answers }. With no model, the default of the kind its input says. Every answer has one: { model, ran, paid, replayed }. retry_key replays an answer a call with the same key and input had, for a day, without paying again. It's paid with a key in the Vault: ai-gateway_models says whose.",
            "input": { "type": "object", "additionalProperties": true, "properties": {
                "model": { "type": "string", "description": "a model id, as ai-gateway_models lists them: workers-ai/@cf/cloudflare/clef-flash, google/gemini-3.5-flash-lite" },
                "messages": { "type": "array", "items": { "type": "object" }, "description": "chat: OpenAI's messages, each { role, content }" },
                "state": { "type": "object", "description": "decide: the named fields the questions are about" },
                "questions": { "type": "object", "description": "decide: { <name>: { type: noul|choice|score, instructions, criteria?, levels? } }" },
                "response_format": { "type": "object", "description": "chat: { type: json_schema, json_schema: { name, schema } } for a typed answer" },
                "retry_key": { "type": "string", "description": "the same key again replays the first answer, unpaid" },
            } },
            "output": { "type": "object", "additionalProperties": true, "description": "the model's answer in its kind's shape, with one: { model, ran, paid, replayed }" },
        }),
        json!({
            "name": "models", "title": "List the models you can use", "reads": true,
            "description": "Every AI model you can use now, by where it runs, with whose key pays for it (or that none does), and each kind's default model: yours if you set one, else Everyone's.",
            "input": { "type": "object", "properties": {} },
            "output": { "type": "object", "additionalProperties": true, "properties": {
                "companies": { "type": "array", "items": { "type": "object" }, "description": "each { id, title, kind, paid: network|null, models }" },
                "defaults": { "type": "object", "description": "{ chat, decide }, each { model, whose: yours|everyone|built_in }" },
            } },
        }),
        json!({
            "name": "set", "title": "Set the default models",
            "description": "Set the model apps get when they don't name one: chat's or decide's, your own, or with everyone (admins only) Everyone's. An empty model clears it.",
            "input": { "type": "object", "properties": {
                "chat": { "type": "string", "description": "the default chat model's id" },
                "decide": { "type": "string", "description": "the default decide model's id" },
                "everyone": { "type": "boolean", "description": "Everyone's default rather than your own (admins)" },
            } },
            "output": { "type": "object", "required": ["said"], "properties": { "said": { "type": "string" } } },
        }),
    ]
}

/// What `ctx.ai.run`'s caller is told, as the guide every client lists with its tools.
pub const GUIDE: &str = "Apps call models with ctx.ai.run({ model, ...input }, { retryKey }): chat models take OpenAI's chat completions input and answer its ChatCompletion; decide models (Clef) take state and questions. Keys stay in the Vault, as items of kind \"AI key: <company>\" in Everyone's Vault; Workers AI's has account_id too. Errors carry code: refused, no_key, limit, retry_changed, uncertain.";
