//! `shared/protocol.js`, which the Workers and the web client import: written from the Rust that says the same.

use custom_build::protocol::*;
use serde_json::json;

/// `shared/protocol.js`: the constants above, and what the Workers need to say names of the bucket in them.
pub fn javascript() -> String {
    format!(
        "// What the Workers and the web client say that the Rust does too, written by custom-build/src/protocol.rs when they're built:\n\
         // never edit it, and it isn't kept in git.\n\
         export const HEADER = {header};\n\
         export const URLS = {urls};\n\
         export const RELAY = {relay};\n\
         export const TELEMETRY = {telemetry};\n\
         export const PUSH = {push};\n\
         export const SIGNED_NOTE = {signed_note};\n\
         export const AUTH_DOMAIN = {auth};\n\
         export const ENDPOINT = {endpoint};\n\
         export const SOCKET = {socket};\n\
         export const FRESH = {fresh};\n\
         export const SESSION = {session};\n\
         export const SOCKET_FRESH = {socket_fresh};\n\
         export const READS = {reads};\n\
         export const ID_TOKEN_ALGS = {id_token_algs};\n\
         export const PAIRING_LIFE = {pairing_life};\n\
         export const MOST_URLS = {most_urls};\n\
         export const MOST_NEEDED = {most_needed};\n\
         export const BEHIND = {behind};\n\
         export const IROH_PATHS = {iroh};\n\
         export const ROLES = {roles};\n\
         export const NEWEST = {newest};\n\
         export const UPDATES = {updates};\n\
         export const TIDIED = {tidied};\n\
         export const COPY = {copy};\n\
         export const STEP_DIGITS = {digits};\n\
         export const NODE_APP = {node_app};\n\
         export const MAIN = {main};\n\
         export const VAULT = {vault};\n\
         export const LOG = {log};\n\
         export const BLOBS = {blobs};\n\
         export const NOTE_FALLBACK = {fallback};\n\
         export const WIDGET_LIMIT_MS = {widget_limit};\n\
         export const LATE_WIDGET = {late_widget};\n\
         export const CODES = {codes};\n\
         export const KEPT_SPANS = {kept_spans};\n\
         export const YOURS = {yours};\n\
         export const Yours = YOURS[0].toUpperCase() + YOURS.slice(1);\n\
         \n\
         export const folder = (scope) => scope.replace(\":\", \"-\");\n\
         export const databasePrefix = (scope, app, database) => `${{folder(scope)}}/${{app}}/${{database}}`;\n\
         const at = (n) => String(n).padStart(STEP_DIGITS, \"0\");\n\
         export const stepName = (prefix, n) => `${{prefix}}/${{at(n)}}`;\n\
         export const copyName = (prefix, n) => `${{prefix}}/${{COPY}}/${{at(n)}}`;\n\
         export const tidiedName = (prefix) => `${{prefix}}/${{TIDIED}}`;\n\
         export const newestName = (prefix) => `${{prefix}}/${{NEWEST}}`;\n\
         export const updateRefusal = (yours) => {update}.replace(\"{{yours}}\", yours);\n\
         {call}",
        header = json!({ "keys": header::KEYS, "endpoint": header::ENDPOINT, "elsewhere": header::ELSEWHERE, "trace": header::TRACE, "traceparent": header::TRACEPARENT, "user": header::USER, "request": header::REQUEST, "code": header::CODE }),
        kept_spans = json!({ "slow": ::custom_build::protocol::SLOW_MS, "sampledOneIn": ::custom_build::protocol::SAMPLED_ONE_IN }),
        urls = json!(URLS),
        relay = json!(RELAY),
        telemetry = json!(TELEMETRY),
        push = json!(PUSH),
        signed_note = json!(::custom_build::protocol::SIGNED_NOTE),
        auth = json!(AUTH_DOMAIN),
        endpoint = json!(key::ENDPOINT),
        socket = json!(SOCKET),
        fresh = key::FRESH,
        session = key::SESSION,
        socket_fresh = SOCKET_FRESH,
        reads = ::custom_build::protocol::READS,
        id_token_algs = json!(scope::sign_in_proof::ID_TOKEN_ALGS),
        pairing_life = PAIRING_LIFE,
        most_urls = MOST_URLS,
        most_needed = crate::handed::MOST_NEEDED,
        behind = json!(crate::handed::BEHIND),
        iroh = json!(IROH_PATHS.iter().copied().collect::<std::collections::BTreeMap<_, _>>()),
        roles = json!(scope::scopes::Kind::ON_THE_RELAY.map(scope::scopes::Kind::as_str)),
        newest = json!(NEWEST),
        updates = json!(UPDATES),
        tidied = json!(TIDIED),
        copy = json!(COPY),
        digits = STEP_DIGITS,
        node_app = json!(key::APP),
        main = json!(key::APP_DATABASE),
        vault = json!(key::CoreDb::Vault.name()),
        log = json!(key::CoreDb::Log.name()),
        blobs = json!(key::CoreDb::Blobs.name()),
        fallback = json!(NOTE_FALLBACK),
        widget_limit = WIDGET_LIMIT_MS,
        late_widget = json!(LATE_WIDGET),
        yours = json!(::custom_build::config::YOURS),
        codes = json!(code::ALL.iter().map(|c| (code::camel(c), c)).collect::<std::collections::BTreeMap<_, _>>()),
        update = json!(UPDATE_REFUSAL),
        call = crate::call::javascript(),
    )
}
