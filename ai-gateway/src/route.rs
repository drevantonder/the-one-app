//! Which model id runs where: `<company>/<model>`, the first part naming where it's sent and whose key kind pays; or
//! `local/<model>`, a model on one of the network's devices, which no key pays for.

use serde_json::Value;

use crate::{Failed, Kind};

/// Where a hosted model runs. Not GLOSSARY's "provider" (sign-in's): what a user reads is the company's name (`title`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Company {
    OpenAi,
    Google,
    Anthropic,
    OpenRouter,
    WorkersAi,
    /// The network's own devices: a service that declares `ai` answers with a model it has (`here`, `chat`, `decide`).
    Local,
}

/// The companies whose models run hosted, each paid for with a key.
pub const ALL: [Company; 5] = [Company::WorkersAi, Company::Google, Company::Anthropic, Company::OpenAi, Company::OpenRouter];

impl Company {
    /// As a model id's first part, and as its key's kind says it (`AI key: google`).
    pub fn id(self) -> &'static str {
        match self {
            Company::OpenAi => "openai",
            Company::Google => "google",
            Company::Anthropic => "anthropic",
            Company::OpenRouter => "openrouter",
            Company::WorkersAi => "workers-ai",
            Company::Local => "local",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Company::OpenAi => "OpenAI",
            Company::Google => "Google",
            Company::Anthropic => "Anthropic",
            Company::OpenRouter => "OpenRouter",
            Company::WorkersAi => "Workers AI",
            Company::Local => "your devices",
        }
    }

    /// `gen_ai.provider.name`: OTEL's name where it has one.
    pub fn otel(self) -> &'static str {
        match self {
            Company::OpenAi => "openai",
            Company::Google => "gcp.gemini",
            Company::Anthropic => "anthropic",
            Company::OpenRouter => "openrouter",
            Company::WorkersAi => "workers-ai",
            Company::Local => "local",
        }
    }

    /// The one host its key may be sent to: none for the network's devices.
    pub fn host(self) -> &'static str {
        match self {
            Company::OpenAi => "api.openai.com",
            Company::Google => "generativelanguage.googleapis.com",
            Company::Anthropic => "api.anthropic.com",
            Company::OpenRouter => "openrouter.ai",
            Company::WorkersAi => "api.cloudflare.com",
            Company::Local => "",
        }
    }

    /// The Vault kind of the item whose key pays for it.
    pub fn key_kind(self) -> String {
        format!("{}{}", crate::KEY_KIND, self.id())
    }

    pub fn from_kind(kind: &str) -> Option<Company> {
        let id = kind.strip_prefix(crate::KEY_KIND)?;
        ALL.into_iter().find(|c| c.id() == id)
    }
}

/// A model id, split: where it runs and the model's own id there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub company: Company,
    pub model: String,
}

impl Route {
    pub fn parse(id: &str) -> Result<Route, Failed> {
        let refused = || Failed::Refused(format!("{id} isn't a model id: they're <company>/<model>, as ai-gateway_models lists them ({})", ALL.map(|c| c.id()).join(", ")));
        let (first, model) = id.split_once('/').ok_or_else(refused)?;
        if first == "local" && !model.is_empty() {
            return Ok(Route { company: Company::Local, model: model.to_string() });
        }
        let company = ALL.into_iter().find(|c| c.id() == first).ok_or_else(refused)?;
        if model.is_empty() || (company == Company::WorkersAi && !model.starts_with("@cf/")) {
            return Err(refused());
        }
        Ok(Route { company, model: model.to_string() })
    }

    pub fn id(&self) -> String {
        format!("{}/{}", self.company.id(), self.model)
    }

    /// What it answers. Cloudflare's Clef models decide; every other hosted one is chat; a device's is whichever its
    /// input asks (its device knows, and refuses what the model doesn't do).
    pub fn kind(&self) -> Option<Kind> {
        match self.company {
            Company::Local => None,
            Company::WorkersAi if self.model.starts_with("@cf/cloudflare/clef") => Some(Kind::Decide),
            _ => Some(Kind::Chat),
        }
    }

    /// The request for `body` (the company's own shape, built already), with `key` the Vault item that pays: its fields
    /// are placeholders the Vault fills (`{{<id>:api_key}}`), never the key itself.
    pub fn request(&self, key: &str, kind: Kind, body: Value) -> crate::Outgoing {
        let bearer = vec![("authorization".to_string(), format!("Bearer {{{{{key}:api_key}}}}"))];
        let (url, headers) = match self.company {
            Company::OpenAi => ("https://api.openai.com/v1/chat/completions".to_string(), bearer),
            Company::OpenRouter => ("https://openrouter.ai/api/v1/chat/completions".to_string(), bearer),
            Company::Google => (format!("https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent", self.model), vec![("x-goog-api-key".to_string(), format!("{{{{{key}:api_key}}}}"))]),
            Company::Local => (String::new(), Vec::new()),
            Company::Anthropic => ("https://api.anthropic.com/v1/messages".to_string(), vec![("x-api-key".to_string(), format!("{{{{{key}:api_key}}}}")), ("anthropic-version".to_string(), "2023-06-01".to_string())]),
            Company::WorkersAi => {
                let account = format!("https://api.cloudflare.com/client/v4/accounts/{{{{{key}:account_id}}}}/ai");
                match kind {
                    Kind::Chat => (format!("{account}/v1/chat/completions"), bearer),
                    Kind::Decide => (format!("{account}/run/{}", self.model), bearer),
                }
            }
        };
        let mut headers = headers;
        headers.push(("content-type".to_string(), "application/json".to_string()));
        crate::Outgoing { url, headers, body: body.to_string().into_bytes() }
    }
}
