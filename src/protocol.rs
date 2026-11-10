//! What the machines, and the languages they're written in, say to each other and must say the same: the names requests
//! are signed under, how long they count, the headers and message kinds between devices, the paths more than one place
//! names, the roles a device has, the names objects have in the bucket and the answers refused requests get. Rust is
//! where each is written; `javascript` is the module the Workers and the web client import (`shared/protocol.js`), made
//! from it whenever they're built, so nothing is copied by hand. Where a name can't be imported (a screen's request for a
//! path, a Go service), `tests/contracts.rs` holds it to what's written here or in the routes.

use serde_json::json;

/// What a device signs its requests to the URL signer under.
pub const URLS: &str = "one/urls";
/// What a device signs its requests to the relay under.
pub const RELAY: &str = "one/relay";
pub use crate::push::DOMAIN as PUSH;
/// What a device signs its requests to auth under.
pub const AUTH_DOMAIN: &str = "one/auth";
/// How long a pairing code works (ms): what a node tells the relay it lasts, which the relay holds it to.
pub const PAIRING_LIFE: u64 = 3_600_000;
/// What a session's endpoint signs to open its relay socket.
pub const SOCKET: &str = "one/socket";
/// How old a session's proof for its relay socket may be, either way: a socket is opened as soon as it's made.
pub const SOCKET_FRESH: u64 = 10_000;

/// The most objects the URL signer signs URLs for at once, and a step range it names may be.
pub const MOST_URLS: usize = 256;

/// The headers of requests between machines, which every language that sends or reads one names by these.
pub mod header {
    /// The database keys a caller hands a node with a request, sealed to it (`keys::Handoff`).
    pub const KEYS: &str = "x-one-keys";
    /// A session's endpoint, certified by its device, so the node takes it as that device.
    pub const ENDPOINT: &str = "x-one-endpoint";
    /// A client that picks its node itself: the node it asks says where else to ask, if it won't run the call.
    pub const ELSEWHERE: &str = "x-one-elsewhere";
    /// A request that asks for a trace of what it did, answered in the same header.
    pub const TRACE: &str = "x-one-trace";
    /// Whose request it is, which a node checks against the device's key.
    pub const USER: &str = "x-one-user";
    /// A signed request that has no body to carry it (a GET's or DELETE's).
    pub const REQUEST: &str = "x-one-request";
    /// How much of who's who a client says it reads (`database::READS`).
    pub const READS: &str = "x-one-reads";
    /// The agent a client's tool call is for, base64url of its name (names aren't all ASCII): credit, which proves nothing.
    pub const AGENT: &str = "x-one-agent";
    /// What a machine's own programs send a node of theirs to say they're its own.
    pub const LOCAL_TOKEN: &str = "x-one-local-token";
    /// The code (`code`) of an error the node answers a running call's ask with, which the app's error then carries.
    pub const CODE: &str = "x-one-code";
}

/// The paths of the local API that more than one place names: what is kept apart by what a request runs (`keys::Route`),
/// and what is the API's own, not an app's (`/<app>/…` is an app's, reached by git).
pub mod path {
    pub const CALL: &str = "/api/call";
    pub const SCREENS: &str = "/api/screens/";
    pub const MCP: &str = "/mcp";
    pub const WIDGETS: &str = "/api/widgets";
    pub const BADGES: &str = "/api/badges";
    pub const APPS: &str = "/api/apps";
    pub const VAULT: &str = "/api/vault";
    pub const OPENAPI: &str = "/api/openapi.json";
    pub const HOOKS_PROMPT: &str = "/hooks/prompt";
    /// The first segments of the API's own routes with more under them (`routes()` holds to it: `api::tests`).
    pub const ROOTS: [&str; 4] = ["api", "mcp", "hooks", "skills"];

    /// The app an app's own request (`/<app>/…`, with something after) is for: none for the API's own.
    pub fn app_request(path: &str) -> Option<&str> {
        let (app, rest) = path.strip_prefix('/')?.split_once('/')?;
        (!app.is_empty() && !rest.is_empty() && !ROOTS.contains(&app)).then_some(app)
    }
}

/// The kinds of message devices send each other over iroh, as `{ kind, body }`: a node's with another, with a service's
/// endpoint, and what a client or the Cloudflare node asks of a node.
pub mod message {
    /// A call to run (`{ call }`), or a user's request to join a network (`{ join }`).
    pub const ASK: &str = "ask";
    /// What a device says of itself to another, which answers what it needs to know of it.
    pub const HELLO: &str = "hello";
    /// The scope database's steps, sent to a service's endpoint that is behind.
    pub const STEPS: &str = "steps";
    /// An app's data changed, which the other devices pass on to their screens.
    pub const CHANGED: &str = "changed";
    /// What a service's endpoint has to say to its own node for insights: its program starting and stopping, and OTLP.
    pub const INSIGHTS: &str = "insights";
}

/// What a database keeps in the bucket beside its steps (`<prefix>/<n>`): the newest step's hint, where its steps
/// were deleted past (`tidied`), and the copy of its steps up to one, in a folder of its own.
pub const NEWEST: &str = "newest";
/// The bucket's folder of what `one-dev update` publishes, and the path it's read at from the network's address (src/latest.rs).
pub const UPDATES: &str = "updates";
pub const TIDIED: &str = "tidied";
pub const COPY: &str = "copy";
/// The digits of a step's position in its name, so names sort in order.
pub const STEP_DIGITS: usize = 12;

/// Where iroh's relay protocol is reached at the network's address: the Workers' gateway sends these, and everything under
/// `/relay`, to the relay.
pub const IROH_PATHS: [(&str, &str); 3] = [("relay", "/relay"), ("ping", "/ping"), ("portal", "/generate_204")];

/// What a request from a device that reads less of who's who than this build (`database::READS`) is refused with, status 426:
/// `{yours}` is the network's name as users say it.
pub const UPDATE_REFUSAL: &str = "Update this device: {yours}'s other devices have moved on to what this one doesn't read yet.";

/// The refusal as a sentence, for `yours`.
pub fn update_refusal(yours: &str) -> String {
    UPDATE_REFUSAL.replace("{yours}", yours)
}

/// What a push of a note whose text the phone can't open shows, as Apple's service shows it for the app that can't
/// (and the browser's, which can't open it either).
pub const NOTE_FALLBACK: &str = "New notification";

/// How long a widget may take to draw beside the rest of its page. One that takes longer comes as `late`, showing
/// `LATE_WIDGET`, so the others aren't held back; asked for alone (`kind=<app>.<widget>`), it takes what it takes.
pub const WIDGET_LIMIT_MS: u64 = 1_000;
pub const LATE_WIDGET: &str = "Still drawing.";

/// The codes the node's errors carry beside their text (`{ error, code }`), which callers act on, never on the text.
/// An app's error may carry one too (an error with a `code`), the Workers and clients of other languages read the
/// same ones from `shared/protocol.js`'s `CODES`, each as its value in camelCase.
pub mod code {
    macro_rules! codes {
        ($($(#[$doc:meta])* $name:ident = $value:literal,)*) => {
            $($(#[$doc])* pub const $name: &str = $value;)*
            /// Every code.
            pub const ALL: &[&str] = &[$($value),*];
        };
    }

    codes! {
        /// A device that reads less of who's who than this build.
        UPDATE = "update",
        /// A device the network doesn't list.
        NOT_LET_IN = "not_let_in",
        /// What was asked isn't the caller's to do, or needs `--confirm`.
        REFUSED = "refused",
        /// A request to the node that didn't show the token in its home, as the machine's own programs do: the caller
        /// knows which home it read (`api::token_refused`), the node doesn't.
        NOT_THIS_MACHINE = "not_this_machine",
        /// What was asked needs an account or a sign-in that isn't there.
        NOT_SIGNED_IN = "not_signed_in",
        /// No device runs what was asked now: its node or service is starting, asking again in a moment answers.
        RESTARTING = "restarting",
        /// An app that isn't added, or isn't on the user's pages.
        NOT_ADDED = "not_added",
        /// An app in Recently Deleted: a client says so where its screen would be, and offers Restore.
        APP_DELETED = "app_deleted",
        /// A step signed for a position of who's who that another device's step has taken: sign it again for the next.
        TAKEN = "taken",
        /// A signed request the relay already took once (it takes each `(by, at)` once): sign it again with a new `at`.
        REPLAYED = "replayed",
        /// What was asked is there already: a key the relay lists, a key sealed to a device.
        ALREADY = "already",
        /// A device or key to take out that the network doesn't list.
        NOT_LISTED = "not_listed",
        /// A pairing code that was used already or has expired.
        PAIRING_USED = "pairing_used",
        /// Not a pairing code: copied in part, or not one.
        BAD_PAIRING_CODE = "bad_pairing_code",
        /// A device that already belongs to a network, asked to join another.
        ALREADY_IN_NETWORK = "already_in_network",
        /// The device a pairing code is for couldn't be reached.
        UNREACHABLE = "unreachable",
    }

    /// `not_let_in` as `notLetIn`, as `shared/protocol.js` names it.
    pub fn camel(code: &str) -> String {
        code.split('_').enumerate().map(|(n, w)| if n == 0 { w.to_string() } else { w[..1].to_uppercase() + &w[1..] }).collect()
    }
}

/// An error that says what kind it is, as `{ error, code }` on the wire: the node's answers, and what a device passes
/// on to the one that asked.
#[derive(Debug)]
pub struct Coded {
    pub code: String,
    pub message: String,
}

impl Coded {
    pub fn new(code: &str, message: impl std::fmt::Display) -> Coded {
        Coded { code: code.into(), message: message.to_string() }
    }

    /// What isn't allowed: `refused`.
    pub fn refused(message: impl std::fmt::Display) -> Coded {
        Coded::new(code::REFUSED, message)
    }

    pub fn error(code: &str, message: impl std::fmt::Display) -> anyhow::Error {
        Coded::new(code, message).into()
    }

    pub fn of(err: &anyhow::Error) -> Option<String> {
        err.downcast_ref::<Coded>().map(|c| c.code.clone())
    }

    pub fn or_plain(code: Option<String>, message: impl std::fmt::Display) -> anyhow::Error {
        match code {
            Some(code) => Coded::error(&code, message),
            None => anyhow::anyhow!("{message}"),
        }
    }

    pub fn is(err: &anyhow::Error, code: &str) -> bool {
        Coded::of(err).as_deref() == Some(code)
    }
}

/// `ensure!` for who may do what: what isn't allowed fails as `refused`, with the same words, which the command line
/// exits 3 on.
macro_rules! allowed {
    ($allowed:expr, $($said:tt)+) => {
        if !$allowed {
            return Err($crate::protocol::Coded::refused(format!($($said)+)).into());
        }
    };
}
pub(crate) use allowed;

impl std::fmt::Display for Coded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Coded {}

/// `shared/protocol.js`: the constants above, and what the Workers need to say names of the bucket in them.
pub fn javascript() -> String {
    format!(
        "// What the Workers and the web client say that the Rust does too, written by src/protocol.rs when they're built:\n\
         // never edit it, and it isn't kept in git.\n\
         export const HEADER = {header};\n\
         export const URLS = {urls};\n\
         export const RELAY = {relay};\n\
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
        header = json!({ "keys": header::KEYS, "endpoint": header::ENDPOINT, "elsewhere": header::ELSEWHERE, "trace": header::TRACE, "user": header::USER, "request": header::REQUEST, "code": header::CODE }),
        urls = json!(URLS),
        relay = json!(RELAY),
        push = json!(PUSH),
        signed_note = json!(crate::push::SIGNED_NOTE),
        auth = json!(AUTH_DOMAIN),
        endpoint = json!(crate::keys::ENDPOINT),
        socket = json!(SOCKET),
        fresh = crate::keys::FRESH,
        session = crate::keys::SESSION,
        socket_fresh = SOCKET_FRESH,
        reads = crate::database::READS,
        id_token_algs = json!(crate::onboarding::sign_in_proof::ID_TOKEN_ALGS),
        pairing_life = PAIRING_LIFE,
        most_urls = MOST_URLS,
        most_needed = crate::keys::MOST_NEEDED,
        behind = json!(crate::keys::BEHIND),
        iroh = json!(IROH_PATHS.iter().copied().collect::<std::collections::BTreeMap<_, _>>()),
        roles = json!(crate::database::scopes::Kind::ALL.map(crate::database::scopes::Kind::as_str)),
        newest = json!(NEWEST),
        updates = json!(UPDATES),
        tidied = json!(TIDIED),
        copy = json!(COPY),
        digits = STEP_DIGITS,
        node_app = json!(crate::database::scopes::APP),
        main = json!(crate::database::scopes::APP_DATABASE),
        vault = json!(crate::database::scopes::CoreDb::Vault.name()),
        log = json!(crate::database::scopes::CoreDb::Log.name()),
        blobs = json!(crate::database::scopes::CoreDb::Blobs.name()),
        fallback = json!(NOTE_FALLBACK),
        widget_limit = WIDGET_LIMIT_MS,
        late_widget = json!(LATE_WIDGET),
        yours = json!(crate::config::YOURS),
        codes = json!(code::ALL.iter().map(|c| (code::camel(c), c)).collect::<std::collections::BTreeMap<_, _>>()),
        update = json!(UPDATE_REFUSAL),
        call = crate::call::javascript(),
    )
}

/// `T`'s JSON Schema as the OpenAPI document (src/api/openapi.rs) says it, its description at the top: a whole object, with
/// no `$defs` to resolve.
pub fn schema_of<T: schemars::JsonSchema>() -> serde_json::Value {
    let mut settings = schemars::generate::SchemaSettings::draft2020_12();
    settings.inline_subschemas = true;
    let mut schema = settings.into_generator().into_root_schema_for::<T>();
    schema.remove("$schema");
    schema.remove("title");
    schema.to_value()
}
