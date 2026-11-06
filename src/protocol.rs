//! What the Workers' JavaScript and the Rust say to each other and must say the same: the names requests are signed
//! under, how long they count, the roles a device has, the names objects have in the bucket and the answers refused
//! requests get. Rust is where each is written; `javascript` is the module the Workers and the web client import
//! (`shared/protocol.js`), made from it whenever they're built, so nothing is copied by hand.

use serde_json::json;

/// What a device signs its requests to the URL signer under.
pub const URLS: &str = "one/urls";
/// What a device signs its requests to the relay under.
pub const RELAY: &str = "one/relay";
pub use crate::push::DOMAIN as PUSH;
/// What a device signs its requests to auth under.
pub const AUTH_DOMAIN: &str = "one/auth";
/// How many days an invite link works.
pub const INVITE_DAYS: u64 = 3;
/// How long a pairing code works (ms): what a node tells the relay it lasts, which the relay holds it to.
pub const PAIRING_LIFE: u64 = 7 * 86_400_000;
/// What a session's endpoint signs to open its relay socket.
pub const SOCKET: &str = "one/socket";
/// How old a session's proof for its relay socket may be, either way: a socket is opened as soon as it's made.
pub const SOCKET_FRESH: u64 = 10_000;

/// The most objects the URL signer signs URLs for at once, and a step range it names may be.
pub const MOST_URLS: usize = 256;

/// What a database keeps in the bucket beside its steps (`<prefix>/<n>`): the newest step's hint, where its steps
/// were deleted past (`tidied`), and the copy of its steps up to one, in a folder of its own.
pub const NEWEST: &str = "newest";
pub const TIDIED: &str = "tidied";
pub const COPY: &str = "copy";
/// The digits of a step's position in its name, so names sort in order.
pub const STEP_DIGITS: usize = 12;

/// Where iroh's relay protocol is reached at the network's address: the Workers' gateway sends these, and everything under
/// `/relay`, to the relay.
pub const IROH_PATHS: [(&str, &str); 3] = [("relay", "/relay"), ("ping", "/ping"), ("portal", "/generate_204")];

/// What a request from a device that reads less of who's who than this build (`database::READS`) is refused with, status 426:
/// `{yours}` is the network's name as people say it.
pub const UPDATE_REFUSAL: &str = "Update this device: {yours}'s other devices have moved on to what this one doesn't read yet.";

/// The refusal as a sentence, for `yours`.
pub fn update_refusal(yours: &str) -> String {
    UPDATE_REFUSAL.replace("{yours}", yours)
}

/// What a push of a note whose text the phone can't open shows, as Apple's service shows it for the app that can't
/// (and the browser's, which can't open it either).
pub const NOTE_FALLBACK: &str = "You have a new notification.";

/// The codes the node's errors carry beside their text (`{ error, code }`), which callers act on, never on the text.
/// An app's error may carry one too (an error with a `code`), the Workers and clients of other languages read the
/// same ones from `shared/protocol.js`.
pub mod code {
    /// A device that reads less of who's who than this build.
    pub const UPDATE: &str = "update";
    /// A device the network doesn't list.
    pub const NOT_LET_IN: &str = "not_let_in";
    /// What was asked isn't the caller's to do, or needs `--confirm`.
    pub const REFUSED: &str = "refused";
    /// What was asked needs an account or a sign-in that isn't there.
    pub const NOT_SIGNED_IN: &str = "not_signed_in";
    /// No device runs what was asked now: its node or service is starting, asking again in a moment answers.
    pub const RESTARTING: &str = "restarting";
    /// An app that isn't added, or isn't on the person's pages.
    pub const NOT_ADDED: &str = "not_added";
    /// A step signed for a position of who's who that another device's step has taken: sign it again for the next.
    pub const TAKEN: &str = "taken";
    /// A signed request the relay already took once (it takes each `(by, at)` once): sign it again with a new `at`.
    pub const REPLAYED: &str = "replayed";
    /// What was asked is there already: a key the relay lists, a key sealed to a device.
    pub const ALREADY: &str = "already";
    /// A device or key to take out that the network doesn't list.
    pub const NOT_LISTED: &str = "not_listed";
    /// A pairing code that was used already or has expired.
    pub const PAIRING_USED: &str = "pairing_used";
    /// Not a pairing code: copied in part, or not one.
    pub const BAD_PAIRING_CODE: &str = "bad_pairing_code";
    /// A device that already belongs to a network, asked to join another.
    pub const ALREADY_IN_NETWORK: &str = "already_in_network";
    /// The device a pairing code is for couldn't be reached.
    pub const UNREACHABLE: &str = "unreachable";
    /// An app's sign-in needs the Vault's `client` first (Google's OAuth client), which an admin sets once.
    pub const NO_CLIENT: &str = "no_client";
    /// A sign-in to finish that nothing is waiting for: it finished already, or was never started.
    pub const NOT_WAITING: &str = "not_waiting";
    /// A limit of what the person's plan gives (AI's, by the minute or the day), which asking again later gets past.
    pub const LIMIT: &str = "limit";
    /// A browser profile an app asked an agent device for that isn't there.
    pub const NO_PROFILE: &str = "no_profile";
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

impl std::fmt::Display for Coded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Coded {}

/// What `<cli> cloudflare deploy` and the shell `update` runs it in exit with, which `update` acts on.
pub mod exit {
    /// Refused: a command that wasn't allowed, or needs `--confirm` (`tools::exit_code`); a deploy from a device that isn't
    /// the Workers' home.
    pub const REFUSED: i32 = 3;
    /// This device has no Cloudflare settings, so it has nothing to deploy.
    pub const NO_SETTINGS: i32 = 4;
    /// The device's node isn't the build being sent out yet: ask again once it is (the exit code `sh` programs use for
    /// "try again", EX_TEMPFAIL).
    pub const NOT_YET: i32 = 75;
}

/// `shared/protocol.js`: the constants above, and what the Workers need to say names of the bucket in them.
pub fn javascript() -> String {
    format!(
        "// What the Workers and the web client say that the Rust does too, written by src/protocol.rs when they're built:\n\
         // never edit it, and it isn't kept in git.\n\
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
         export const PAIRING_LIFE = {pairing_life};\n\
         export const MOST_URLS = {most_urls};\n\
         export const MOST_NEEDED = {most_needed};\n\
         export const BEHIND = {behind};\n\
         export const IROH_PATHS = {iroh};\n\
         export const ROLES = {roles};\n\
         export const NEWEST = {newest};\n\
         export const TIDIED = {tidied};\n\
         export const COPY = {copy};\n\
         export const STEP_DIGITS = {digits};\n\
         export const NUMBER_MAX = {number_max};\n\
         export const NODE_APP = {node_app};\n\
         export const MAIN = {main};\n\
         export const VAULT = {vault};\n\
         export const LOG = {log};\n\
         export const BLOBS = {blobs};\n\
         export const NOTE_FALLBACK = {fallback};\n\
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
",
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
        pairing_life = PAIRING_LIFE,
        most_urls = MOST_URLS,
        most_needed = crate::keys::MOST_NEEDED,
        behind = json!(crate::keys::BEHIND),
        iroh = json!(IROH_PATHS.iter().copied().collect::<std::collections::BTreeMap<_, _>>()),
        roles = json!(crate::database::scopes::Kind::ALL.map(crate::database::scopes::Kind::as_str)),
        newest = json!(NEWEST),
        tidied = json!(TIDIED),
        copy = json!(COPY),
        digits = STEP_DIGITS,
        number_max = crate::database::steps::NUMBER_MAX,
        node_app = json!(crate::database::scopes::APP),
        main = json!(crate::database::scopes::APP_DATABASE),
        vault = json!(crate::vault::DATABASE),
        log = json!(crate::database::steps::KEY_NAME),
        blobs = json!(crate::bucket::BLOBS),
        fallback = json!(NOTE_FALLBACK),
        yours = json!(crate::config::YOURS),
        codes = json!({ "update": code::UPDATE, "notLetIn": code::NOT_LET_IN, "refused": code::REFUSED, "notSignedIn": code::NOT_SIGNED_IN, "restarting": code::RESTARTING, "notAdded": code::NOT_ADDED, "taken": code::TAKEN, "replayed": code::REPLAYED, "already": code::ALREADY, "notListed": code::NOT_LISTED, "pairingUsed": code::PAIRING_USED, "badPairingCode": code::BAD_PAIRING_CODE, "alreadyInNetwork": code::ALREADY_IN_NETWORK, "unreachable": code::UNREACHABLE, "noClient": code::NO_CLIENT, "notWaiting": code::NOT_WAITING, "limit": code::LIMIT, "noProfile": code::NO_PROFILE }),
        update = json!(UPDATE_REFUSAL),
    )
}
