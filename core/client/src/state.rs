//! Where a client is with its network (`State`): whether its device is in it, and whether it answers, and how the
//! client moves from one to the next, as it's answered and as it tries again (`reconnecting`).

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Serialize;
use serde_json::json;
use tokio::sync::watch;
use tokio::time::sleep;

use crate::{Client, auth::{DeviceAuthorization, NotApproved}};

/// Where a client is with its network, from when it opens: whether its device is in its network, and whether its
/// network answers. It's what every client shows, the app's window a page for each and a banner over its apps, and the
/// CLI and MCP a line. It's one state, so what can't be together never is: a device showing a code has been answered,
/// and only one that's in asks servers, which may say it's behind them. It moves only as `moves_to` says, and only by
/// its client's own steps (`Client::moved`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum State {
    /// Asking its network's address for its token, or for a code to be let in by, which it asks again while its
    /// network doesn't answer (`reconnecting`). Its connection is never `Online` or `Behind`: once the address
    /// answers, it's in or it shows a code.
    Opening { connection: Connection },
    /// In its network: it asks its servers, as whoever it signed in as.
    SignedIn { connection: Connection },
    /// Not in its network: it never was, or its network's address says it was taken out since, which a server's
    /// word alone never decides. It shows `code`, and is in once its person approves it where they sign in. What it
    /// kept stays until someone else signs in on it (`NetworkConfig::store_token`). `why` says why it's out, if
    /// it says: its last code was denied or expired, or it was removed. Its network gave it the code, so it answers.
    SignIn { code: Box<DeviceAuthorization>, why: Option<NotApproved> },
}

impl State {
    /// Whether a client in this state moves to `next`: the only moves there are.
    fn moves_to(&self, next: &State) -> bool {
        use Connection::*;
        use State::*;
        match (self, next) {
            // It's given its token, or a code to be let in by, or carries on with the token it has where its network's
            // address doesn't answer.
            (Opening { .. }, SignedIn { connection: Online | Offline } | SignIn { .. }) => true,
            // Its network's address didn't answer, or it asks again.
            (Opening { connection: was }, Opening { connection: now }) => {
                !matches!(now, Online | Behind(_)) && was.moves_to(now)
            }
            // Its network's address says it's nobody's now, or its person signed out on it.
            (SignedIn { .. }, SignIn { .. } | Opening { connection: Connecting }) => true,
            (SignedIn { connection: was }, SignedIn { connection: now }) => was.moves_to(now),
            // Its person approved it, or its code was denied or expired, which it asks about again.
            (SignIn { .. }, Opening { connection: Connecting }) => true,
            _ => false,
        }
    }

    /// Whether its network answers: one showing a code was answered.
    pub fn connection(&self) -> Connection {
        match self {
            Self::Opening { connection } | Self::SignedIn { connection } => connection.clone(),
            Self::SignIn { .. } => Connection::Online,
        }
    }

    /// The same state with its network's answer `connection`, where it has one.
    pub(crate) fn with(&self, connection: Connection) -> Option<State> {
        match self {
            Self::Opening { .. } => Some(Self::Opening { connection }),
            Self::SignedIn { .. } => Some(Self::SignedIn { connection }),
            Self::SignIn { .. } => None,
        }
    }

    /// Whether it's where `other` is with its network, its device in, out with the same code, or asking which,
    /// whatever its network answers.
    pub fn same_session(&self, other: &State) -> bool {
        use State::*;
        match (self, other) {
            (Opening { .. }, Opening { .. }) | (SignedIn { .. }, SignedIn { .. }) => true,
            (SignIn { .. }, SignIn { .. }) => self == other,
            _ => false,
        }
    }
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Opening { .. } => f.write_str("Opening…"),
            Self::SignedIn { .. } => f.write_str("Signed in."),
            Self::SignIn { code, why: Some(NotApproved::Removed) } => {
                write!(f, "This device was removed from your network. Open this where you sign in, and check it shows {}:\n{}", code.code, code.link)
            }
            Self::SignIn { code, why } => {
                let why = why.map(|why| format!("{why} ")).unwrap_or_default();
                write!(f, "{why}This device isn't in your network yet. Open this where you sign in, and check it shows {}:\n{}", code.code, code.link)
            }
        }
    }
}

/// Whether a client's network answers, as its state has it (`State`). Its network answers where what the client asks
/// of it is answered: its address, while the device asks to be let in, and its servers from then on. Whether it lost
/// its network is which one it is: asking again after it did is `Reconnecting`, never `Connecting`. It's said to a
/// page as its name (`online`, `behind`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "&'static str")]
pub enum Connection {
    /// Asking, and not answered yet, as the client opens, or as it asks to be let in again.
    Connecting,
    /// Its network answered the last thing asked.
    Online,
    /// Its network didn't answer. It tries again by itself, sooner at first and then less often, and at once when
    /// its person tries again (`retry`).
    Offline,
    /// Asking again after its network didn't answer, or its servers said it's behind them, and not answered yet.
    Reconnecting,
    /// Its servers answer, but refused it for being on a version before theirs that doesn't talk with them
    /// (`OneCore.Mcp`), saying why: from then on it asks only what brings it up to date (`updates`), as an app that
    /// requires an update does nothing but offer it, rather than being refused again for everything it asks. It's
    /// behind until a server talks with it again, which it asks afresh as it reconnects and as its person tries again
    /// (`retry`): a reason kept from before is never taken for a server's answer now.
    Behind(String),
}

impl Connection {
    /// Whether a client whose network answers this way moves to `next`: the only moves there are.
    fn moves_to(&self, next: &Connection) -> bool {
        use Connection::*;
        matches!(
            (self, next),
            // It was answered, or wasn't.
            (Connecting | Reconnecting | Offline, Online | Behind(_))
            | (Connecting | Reconnecting, Offline)
            // It tries again.
            | (Offline, Reconnecting)
            // It lost its network, or a server says it's behind it.
            | (Online, Offline | Behind(_))
            // It lost its network, or its person tries again.
            | (Behind(_), Offline | Reconnecting)
        )
    }

    /// Whether its network answered the last thing asked.
    pub fn answers(&self) -> bool {
        matches!(self, Self::Online | Self::Behind(_))
    }
}

impl From<Connection> for &'static str {
    fn from(connection: Connection) -> Self {
        match connection {
            Connection::Connecting => "connecting",
            Connection::Online => "online",
            Connection::Behind(_) => "behind",
            Connection::Offline => "offline",
            Connection::Reconnecting => "reconnecting",
        }
    }
}

impl std::fmt::Display for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Connecting => "Connecting to your network…",
            Self::Reconnecting => "Reconnecting to your network…",
            Self::Online => "Online.",
            Self::Offline => "Can't reach your network. Check this device is online, then try again.",
            Self::Behind(why) => why,
        })
    }
}

/// How long its network not answering goes unshown: the window's page shows nothing for a moment's gap.
const UNNOTICED: Duration = Duration::from_secs(3);

/// While a client's network doesn't answer, it tries again by itself, after 1 s, then 2 s, then every 3 s at most,
/// each wait a random time from half of it to all of it, and at once when its person tries again (`retry`) or the
/// device is back online or in front (`reconnect`), as the device or its network may be back any time: what's outside a
/// device comes and goes, and no person should have to ask for what was going to come back anyway. A device that's
/// opening asks its network's address to let it in, and one that's in and watching asks a server. Once its network
/// answers again, it says again that it's watching, to every server, since one that was starting may not have heard.
pub(crate) async fn reconnecting(client: std::sync::Weak<Client>) {
    const FIRST: Duration = Duration::from_secs(1);
    const MOST: Duration = Duration::from_secs(3);
    let Some((mut state, tried)) = client.upgrade().map(|client| (client.state(), client.again.clone())) else {
        return;
    };
    // Where it was when it last looked: answered after `Offline` or `Reconnecting`, it's back.
    let (mut again, mut was) = (FIRST, state.borrow().connection());
    loop {
        let now = state.borrow_and_update().clone();
        let before = std::mem::replace(&mut was, now.connection());
        if now.connection() == Connection::Offline {
            tokio::select! {
                _ = sleep(again.mul_f64(0.5 + fastrand::f64() / 2.0)) => {}
                _ = tried.notified() => {}
                changed = state.changed() => if changed.is_err() { return } else { continue },
            }
            again = (again * 2).min(MOST);
            let Some(client) = client.upgrade() else { return };
            let now = client.state.borrow().clone();
            match now {
                State::Opening { .. } => {
                    client.connection_moved(Connection::Reconnecting);
                    client.admit(&now, None).await;
                }
                State::SignedIn { .. } if client.watching.load(Ordering::Relaxed) => {
                    client.connection_moved(Connection::Reconnecting);
                    let _ = client.request("ping", json!({})).await;
                }
                _ => {}
            }
            continue;
        }
        if now.connection().answers() && matches!(before, Connection::Offline | Connection::Reconnecting) {
            again = FIRST;
            let Some(client) = client.upgrade() else { return };
            if client.watching.load(Ordering::Relaxed) {
                client.watch(true).await;
            }
        }
        if state.changed().await.is_err() {
            return;
        }
    }
}

impl Client {
    /// Where this client is with its network, as that changes (`State`). Whether its network answers is found out by
    /// asking: nothing is said over a connection to learn it.
    pub fn state(&self) -> watch::Receiver<State> {
        self.state.subscribe()
    }

    /// Moves this client to where `next` says from where it is, if it says and that's a move there is
    /// (`State::moves_to`), and once its network answers again after it didn't for longer than a person is shown
    /// nothing of it, reports how long it didn't. Gives whether it moved.
    pub(crate) fn moved(&self, next: impl FnOnce(&State) -> Option<State>) -> bool {
        let mut answers = None;
        let moved = self.state.send_if_modified(|now| {
            let Some(next) = next(now).filter(|next| now.moves_to(next)) else { return false };
            answers = Some(next.connection());
            *now = next;
            true
        });
        let mut since = self.unanswered.lock().expect("no ask panics");
        match answers {
            Some(Connection::Offline) => drop(since.get_or_insert_with(|| (Instant::now(), self.offline()))),
            Some(connection) if connection.answers() => {
                if let Some((since, offline)) = since.take().filter(|(since, _)| since.elapsed() > UNNOTICED) {
                    let took = since.elapsed().as_secs_f64() * 1000.0;
                    self.report(match offline {
                        true => json!({"name": "one.client.unanswered", "took": took, "attributes": {"one.outcome": "offline"}}),
                        false => json!({"name": "one.client.unanswered", "took": took, "error": "No server is answering"}),
                    })
                }
            }
            _ => {}
        }
        moved
    }

    /// Moves whether its network answers to `next`, where its state has that.
    fn connection_moved(&self, next: Connection) -> bool {
        self.moved(|now| now.with(next))
    }

    /// Has a client whose network didn't answer ask it again now, as the device says it's back online or its app is
    /// in front again, rather than when it next would (`reconnecting`). One that's answered asks nothing.
    pub fn reconnect(&self) {
        if self.state.borrow().connection() == Connection::Offline {
            self.again.notify_one();
        }
    }

    /// Asks its network again now, as its person does by trying again: where it didn't answer, and where a server
    /// refused it for being behind (`Connection::Behind`), where it's reconnecting until a server answers, which may
    /// talk with it now, updated or rolled back since. What it asks is the list of its files, as itself, which a
    /// server refuses while it's behind: it answers once it has, failing with why where it wasn't answered.
    pub async fn retry(&self) -> Result<()> {
        self.reconnect();
        self.moved(|now| matches!(now.connection(), Connection::Behind(_)).then(|| now.with(Connection::Reconnecting)).flatten());
        self.fresh("resources/list", &json!({})).await.map(drop)
    }
}
