//! What the machines, and the languages they're written in, say to each other and must say the same: the names requests
//! are signed under, how long they count, the headers and message kinds between devices, the paths more than one place
//! names, the roles a device has, the names objects have in the bucket and the answers refused requests get. Rust is
//! where each is written; `javascript` is the module the Workers and the web client import (`shared/protocol.js`), made
//! from it whenever they're built, so nothing is copied by hand. Where a name can't be imported (a screen's request for a
//! path, a Go service), `tests/contracts.rs` holds it to what's written here or in the routes.


/// What a device signs its requests to the URL signer under.
pub const URLS: &str = "one/urls";
/// What a device signs its requests to the relay under.
pub const RELAY: &str = "one/relay";
/// What a device signs to write or read telemetry.
pub const TELEMETRY: &str = "one/telemetry";
/// What a device signs its requests to auth under.
pub const AUTH_DOMAIN: &str = "one/auth";
/// How long a pairing code works (ms): what an app runtime tells the relay it lasts, which the relay holds it to.
pub const PAIRING_LIFE: u64 = 3_600_000;
/// What a session's endpoint signs to open its relay socket.
pub const SOCKET: &str = "one/socket";
/// How old a session's proof for its relay socket may be, either way: a socket is opened as soon as it's made.
pub const SOCKET_FRESH: u64 = 10_000;

/// The most objects the URL signer signs URLs for at once, and a step range it names may be.
pub const MOST_URLS: usize = 256;

/// What a device signs its requests to the notifications Worker for.
pub const NOTIFICATIONS: &str = "one/notifications";

/// What a device signs a note for.
pub const SIGNED_NOTE: &str = "one/note";

/// How long a call takes before it keeps a full span. ARCHITECTURE.md's targets state none for a whole call, and its
/// nearest is a write seen elsewhere within 1 s.
pub const SLOW_MS: u64 = 1_000;

/// One trace in this many that went well is kept whole, to see what normal looks like. A client's (`shared/protocol.js`'s
/// `KEPT_SPANS`) the same.
pub const SAMPLED_ONE_IN: u64 = 20;

/// What this build reads of who's who: the scope database's tables, types, roles and rules of this number. A device
/// that reads less is told to update, by app runtimes (`protocol::header::READS`), the relay, auth and the URL signer (`reads` in what it
/// asks them, `shared/protocol.js`'s `READS`, which `protocol::javascript` writes from this). A release that changes
/// `scope::SCHEMA`, `permissions::SCHEMA` or `permissions::DEFAULTS`, or takes away a route older devices use, or writes a
/// sealed format older devices can't open (`keys::FORMAT`), reads one more.
pub const READS: u32 = 16;

/// The headers of requests between machines, which every language that sends or reads one names by these.
pub mod header {
    /// A line an app's `_http` wants git to show whoever asked.
    pub const APP_SAYS: &str = "x-app-says";
    /// The database keys a caller hands an app runtime with a request, sealed to it (`keys::Handoff`).
    pub const KEYS: &str = "x-one-keys";
    /// A session's endpoint, certified by its device, so the app runtime takes it as that device.
    pub const ENDPOINT: &str = "x-one-endpoint";
    /// A client that picks its app runtime itself: the app runtime it asks says where else to ask, if it won't run the call.
    pub const ELSEWHERE: &str = "x-one-elsewhere";
    /// A request that asks for a trace of what it did, answered in the same header.
    pub const TRACE: &str = "x-one-trace";
    /// W3C's: the trace and span a request is part of.
    pub const TRACEPARENT: &str = "traceparent";
    /// Whose request it is, which an app runtime checks against the device's key.
    pub const USER: &str = "x-one-user";
    /// A signed request that has no body to carry it (a GET's or DELETE's).
    pub const REQUEST: &str = "x-one-request";
    /// How much of who's who a client says it reads (`database::READS`).
    pub const READS: &str = "x-one-reads";
    /// The agent a client's tool call is for, base64url of its name (names aren't all ASCII): credit, which proves nothing.
    pub const AGENT: &str = "x-one-agent";
    /// What a machine's own programs send an app runtime of theirs to say they're its own.
    pub const LOCAL_TOKEN: &str = "x-one-local-token";
    /// The code (`code`) of an error the app runtime answers a running call's ask with, which the app's error then carries.
    pub const CODE: &str = "x-one-code";
}

/// The kinds of message devices send each other over iroh, as `{ kind, body }`: an app runtime's with another, and what a
/// client or oned on Cloudflare asks of an app runtime.
pub mod message {
    /// A call to run (`{ call }`), or a user's request to join a network (`{ join }`).
    pub const ASK: &str = "ask";
    /// What a device runs (`{}`), asked when a screen shows it or a call needs its services: what its services describe
    /// of themselves, and its oned's parts and their state.
    pub const STATE: &str = "state";
    /// An app's data changed, which the other devices pass on to their screens.
    pub const CHANGED: &str = "changed";
    /// An ask of a service of the device's, sealed to it, which its runtime carries to it unread (`services::carried`).
    pub const SERVICE: &str = "service";
}

/// What a database keeps in the bucket beside its steps (`<prefix>/<n>`): the newest step's hint, where its steps
/// were deleted past (`tidied`), and the copy of its steps up to one, in a folder of its own.
pub const NEWEST: &str = "newest";
/// The bucket's folder of what `one-dev update` publishes, and the path it's read at from the network's address (oned/update/src/latest.rs).
pub const UPDATES: &str = "updates";
pub const TIDIED: &str = "tidied";
pub const COPY: &str = "copy";
/// The digits of a step's position in its name, so names sort in order.
pub const STEP_DIGITS: usize = 12;

/// Where iroh's relay protocol is reached at the network's address: the Workers' gateway sends these, and everything under
/// `/relay`, to the relay.
pub const IROH_PATHS: [(&str, &str); 3] = [("relay", "/relay"), ("ping", "/ping"), ("portal", "/generate_204")];

/// What this build says it is, in `POST /relay/computes` (`version`, frozen with it in `tests/contracts.rs`), so every
/// client can show each device's version: `<commit> <committed> <target>`, the words a published build goes by in the
/// signed record.
pub fn version() -> String {
    format!("{} {} {}", crate::config::BUILD, crate::config::COMMITTED, crate::config::TARGET)
}

/// A device's `version` read back: its commit, when that was made, and its target.
pub fn told(version: &str) -> Option<(&str, u64, &str)> {
    let mut words = version.split(' ');
    Some((words.next()?, words.next()?.parse().ok()?, words.next()?))
}

/// What a request from a device that reads less of who's who than this build (`database::READS`) is refused with, status 426:
/// `{yours}` is the network's name as users say it.
pub const UPDATE_REFUSAL: &str = "Update this device: {yours}'s other devices have moved on to what this one doesn't read yet.";

/// The refusal as a sentence, for `yours`.
pub fn update_refusal(yours: &str) -> String {
    UPDATE_REFUSAL.replace("{yours}", yours)
}

/// What a notification whose note the phone can't open shows, as Apple's service shows it for the app that can't
/// (and the browser's, which can't open it either).
pub const NOTE_FALLBACK: &str = "New notification";

/// How long a widget may take to draw beside the rest of its page. One that takes longer comes as `late`, showing
/// `LATE_WIDGET`, so the others aren't held back; asked for alone (`kind=<app>.<widget>`), it takes what it takes.
pub const WIDGET_LIMIT_MS: u64 = 1_000;
pub const LATE_WIDGET: &str = "Still drawing.";

/// The codes the app runtime's errors carry beside their text (`{ error, code }`), which callers act on, never on the text.
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
        /// A request to the app runtime that didn't show the token in its home, as the machine's own programs do: the caller
        /// knows which home it read (`api::token_refused`), the app runtime doesn't.
        NOT_THIS_MACHINE = "not_this_machine",
        /// What was asked needs an account or a sign-in that isn't there.
        NOT_SIGNED_IN = "not_signed_in",
        /// No device runs what was asked now: its app runtime or service is starting, asking again in a moment answers.
        RESTARTING = "restarting",
        /// A service's program stopped while it ran the call, which may have done some of it: look at what it did before
        /// asking again.
        STOPPED = "stopped",
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
        /// No key in the Vault pays for that AI model's company (ai-gateway/).
        NO_KEY = "no_key",
        /// An AI model's company said too many: wait and ask again.
        LIMIT = "limit",
        /// An AI call's retryKey used before for other input.
        RETRY_CHANGED = "retry_changed",
        /// An AI call's retryKey whose first call was sent and never answered: it may have run.
        UNCERTAIN = "uncertain",
        /// An AI model on the network's devices that none of those on has.
        UNAVAILABLE = "unavailable",
    }

    /// `not_let_in` as `notLetIn`, as `shared/protocol.js` names it.
    pub fn camel(code: &str) -> String {
        code.split('_').enumerate().map(|(n, w)| if n == 0 { w.to_string() } else { w[..1].to_uppercase() + &w[1..] }).collect()
    }
}

/// An error that says what kind it is, as `{ error, code }` on the wire: the app runtime's answers, and what a device passes
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
        Coded::new(crate::protocol::code::REFUSED, message)
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
#[macro_export]
macro_rules! allowed {
    ($allowed:expr, $($said:tt)+) => {
        if !$allowed {
            return Err($crate::protocol::Coded::refused(format!($($said)+)).into());
        }
    };
}

impl std::fmt::Display for Coded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Coded {}

/// `T`'s JSON Schema as the OpenAPI document (app-runtime/src/api/openapi.rs) says it, its description at the top: a whole object, with
/// no `$defs` to resolve.
pub fn schema_of<T: schemars::JsonSchema>() -> serde_json::Value {
    let mut settings = schemars::generate::SchemaSettings::draft2020_12();
    settings.inline_subschemas = true;
    let mut schema = settings.into_generator().into_root_schema_for::<T>();
    schema.remove("$schema");
    schema.remove("title");
    schema.to_value()
}

