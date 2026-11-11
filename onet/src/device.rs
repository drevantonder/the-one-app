//! A native app, or a computer's onet (`crate::computer`), as a device of its user, the same on each: its own key, which
//! an app keeps in the Keychain, the keystore or the Credential Locker and hands it at start, and a computer in its
//! home; asking to be let in, with the code it shows; and once a device of theirs approved it, asking its nodes with the
//! keys it opens (`crate::keyring`), keeping what outlives it in its folder. A browser runs the same keyring in its worker.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD as B64U;
use ed25519_dalek::SigningKey;
use ::custom_build::config::YOURS;
use ::{key::{Endpoint, hex}, api::handed::Waited};
use serde_json::{Value, json};

use crate::answers::{Answers, Files};
use crate::join;
use crate::bucket::Reached;
pub(crate) use crate::keyring::now;
use crate::keyring::{Address, HANDED_FOR, Keep, Keyring, error, refused};
use crate::{Body, Client, Error, Request, Response, Sent, unsent};

/// What a sign-in for the device whose key is `secret` names it by (`join`), before it's made: its key ‖ seal key, hex.
pub fn join_of(secret: [u8; 32]) -> String {
    let me = Endpoint::of(&SigningKey::from_bytes(&secret));
    format!("{}{}", hex(&me.key), hex(&me.seal))
}

/// A note pushed to the device whose key is `secret`, as its push carried it (base64url): → the note, and the key of
/// the device that sent it (hex), or none for one from the network's Workers. Opened here alone: nothing connects or starts.
pub fn open_note(secret: [u8; 32], pushed: &str) -> Result<(one::push::Note, Option<String>), Error> {
    let sealed = B64U.decode(pushed.trim().trim_end_matches('=').as_bytes()).map_err(|_| failed("That push isn't a note."))?;
    one::push::open_note(&::key::seal_secret(&SigningKey::from_bytes(&secret)), &sealed).map_err(|_| failed("That note isn't this device's."))
}

pub fn new_secret() -> [u8; 32] {
    rand::random()
}

fn failed(message: impl std::fmt::Display) -> Error {
    unsent(message)
}

/// Where a device is with its network, from its sign-in to its sign-out, as a browser goes the same way (cloudflare/web:
/// browser.js `join` and `leave`, web.js `start`, request.js `takenOut`). Whether this key was ever let in here is kept on disk (`joined`), as it
/// outlives the program: `removed` asks it.
enum Stage {
    /// Not reaching its nodes: not let in yet, having asked auth with `asked` if it has since it started (`ask`, then
    /// `wait`); or let in, and its connections closed until it's asked again (`disconnect`: its app in the background,
    /// or a program ending).
    Off { asked: Option<Value> },
    In(Client),
    /// Taken out, as the relay said, or signed out: nothing more is asked as it. Taken out, its key stays until its
    /// user signs in again and the relay still refuses it, as the relay's word alone takes nothing from it.
    Out,
}

/// What this client is built for, as it says in its listing.
pub const PLATFORM: &str = if cfg!(target_os = "ios") {
    "iphone"
} else if cfg!(target_os = "android") {
    "android"
} else if cfg!(target_os = "macos") {
    "mac"
} else if cfg!(windows) {
    "windows"
} else {
    "linux"
};

/// A device's way in: its keys, the network it's in, and what it has opened.
pub struct Device {
    key: SigningKey,
    me: Endpoint,
    address: String,
    /// Where it keeps what outlives it (`Keep`): whether it was let in (`joined`), and the keyring's.
    dir: PathBuf,
    bucket: Reached,
    /// Where its own app runtime answers on this machine, and what it's asked with: a computer's (`on_computer`).
    #[cfg(feature = "computer")]
    runtime: Option<crate::computer::Runtime>,
    /// Held while it connects, so it connects once.
    stage: tokio::sync::Mutex<Stage>,
    keyring: Keyring,
    /// The order things run in, as the node last said (`/api/state`): only ever used to sort nodes it knows.
    order: Mutex<Vec<[u8; 32]>>,
    /// The device it's on, as its app says (`on`): signing in there again replaces this sign-in.
    on: Mutex<Option<String>>,
    /// The last answer of each call that only reads, for its app's screens (`kept`): a phone's and a computer's onet
    /// keep them (`keeping`).
    answers: Option<Answers<Files>>,
    /// Where it keeps its log (`device_logs`), which a node asking for it is answered from.
    logs: Option<PathBuf>,
}

impl Device {
    pub fn new(secret: [u8; 32], address: &str, dir: &str) -> Device {
        let address = address.trim_end_matches('/').to_string();
        Device::with(secret, address.clone(), dir, Reached::signed(address, secret))
    }

    /// A computer's device, its onet's (`crate::computer`): its bucket the place its home's network file names, from
    /// which it reads who's who to find the network's address, if it has one yet (`address` is empty till then); and
    /// its own app runtime at `runtime` (where it answers, and its token), which it asks before any other node.
    #[cfg(feature = "computer")]
    pub async fn on_computer(secret: [u8; 32], dir: &str, place: bucket::Place, address: Option<String>, runtime: Option<crate::computer::Runtime>) -> Result<Device, Error> {
        let mut device = Device::with(secret, String::new(), dir, Reached::place(place, secret, address)).keeping();
        device.runtime = runtime;
        device.keyring.read(&device).await?;
        let address = device.keyring.who(|db| db.setting("address")).and_then(Result::ok).flatten();
        device.address = address.unwrap_or_default().trim_end_matches('/').to_string();
        Ok(device)
    }

    /// Its keyring, as it last read who's who: what a computer's onet signs and opens with for its app runtime.
    #[cfg(feature = "computer")]
    pub(crate) fn keyring(&self) -> &Keyring {
        &self.keyring
    }

    /// Reads who's who again, and asks its own app runtime first as who's who lists it now, and the rest in its order.
    #[cfg(feature = "computer")]
    pub(crate) async fn refresh(&self) -> Result<(), Error> {
        self.keyring.read(self).await?;
        if let Some(client) = self.client().await {
            client.set_local(self.local());
            client.set_ranks(&self.keyring.ranks())?;
            client.set_wakes(self.keyring.cloudflare_node())?;
            client.set_nodes(&self.ordered())?;
        }
        Ok(())
    }

    /// Tells the relay what this device is on, as each device does, and → what it left for this device (`asked`), if
    /// anything: an ask to update, for a computer's onet to hear (`one::node::heard`).
    #[cfg(feature = "computer")]
    pub(crate) async fn told(&self) -> Result<Value, Error> {
        let asked = json!({ "computes": true, "reads": ::custom_build::protocol::READS, "version": ::custom_build::protocol::version() });
        let (_, out) = self.relay_computes(asked).await?;
        Ok(out["asked"].clone())
    }

    fn with(secret: [u8; 32], address: String, dir: &str, bucket: Reached) -> Device {
        let key = SigningKey::from_bytes(&secret);
        let me = Endpoint::of(&key);
        Device {
            bucket,
            #[cfg(feature = "computer")]
            runtime: None,
            keyring: Keyring::new(me.key, ::key::seal_secret(&key)),
            me,
            key,
            address,
            dir: dir.into(),
            stage: tokio::sync::Mutex::new(Stage::Off { asked: None }),
            order: Mutex::default(),
            on: Mutex::default(),
            answers: None,
            logs: None,
        }
    }

    /// Answers a node that asks for its log, from the log kept in `dir` (`device_logs::keep_in`).
    pub fn logging(mut self, dir: &std::path::Path) -> Device {
        self.logs = Some(dir.to_path_buf());
        self
    }

    /// Says which device it's on: that device's oned key (hex), on a machine with one, or an id of the device's own,
    /// hashed, that outlives signing out. Its request to join names it, and once it's in, its listing does.
    pub fn on(&self, device: &str) {
        if ::key::unhex(device).is_ok() {
            *self.on.lock().unwrap() = Some(device.to_string());
        }
    }

    pub fn keeping(mut self) -> Device {
        self.answers = Some(Answers::new(Files::new(&self.dir, &self.key)));
        self
    }

    pub fn key(&self) -> String {
        hex(&self.me.key)
    }

    /// The code it shows to be approved: "ABCD EFGH IJKL MNOP".
    pub fn code(&self) -> String {
        ::key::spaced_code(&::key::join_code(&self.me))
    }

    /// Asks auth to let it in, as the user signed in with `token`, calling it `name` (and with someone new's invite
    /// secret, its proof).
    pub async fn ask(&self, token: &str, name: &str, invite: Option<&str>) -> Result<bool, Error> {
        let on = self.on.lock().unwrap().clone();
        let asked = join::asking(&self.me, name, on.as_deref(), Some(PLATFORM), invite);
        if let Stage::Off { asked: kept } = &mut *self.stage.lock().await {
            *kept = Some(asked.clone());
        }
        join::ask(self, token, &asked).await
    }

    /// Whether it's been let in: it reaches a node as itself. While it waits, what auth says of its request
    /// (`keys::waited`) is acted on as every client does: once approved, the network the device that approved it said it's
    /// in (its root) is the only one it takes, kept before it reads any, so none it's shown first can be another; one
    /// gone from auth is asked again unless it's in; a sign-in that's no more fails, to sign in again.
    pub async fn wait(&self, token: &str) -> Result<bool, Error> {
        let gone = match join::waited(self, token, &self.me.key).await? {
            Waited::SignedOut => return Err(failed("Sign in again.")),
            Waited::Waiting => return Ok(false),
            Waited::Approved { .. } => false,
            Waited::Gone => true,
        };
        if self.connect().await? {
            return Ok(true);
        }
        let asked = match &*self.stage.lock().await {
            Stage::Off { asked } => asked.clone(),
            _ => None,
        };
        if gone && let Some(asked) = asked {
            let _ = join::ask(self, token, &asked).await;
        }
        Ok(false)
    }

    /// Reaches its network's nodes, if it's been let in: false if not yet. Then, each time, as each start of a browser
    /// does, it settles its user's own things (`settle_own`), if they aren't: what fails is tried again at the next.
    pub async fn connect(&self) -> Result<bool, Error> {
        if !self.reach().await? {
            return Ok(false);
        }
        if let Some(client) = self.client().await {
            let on = self.on.lock().unwrap().clone();
            if let Err(err) = self.keyring.settle_own(&client, self, on.as_deref(), Some(PLATFORM)).await {
                eprintln!("settling this user's own things: {err}");
            }
        }
        Ok(true)
    }

    /// Reaches its network's nodes, if it's let in: false if it isn't (yet, or any more).
    async fn reach(&self) -> Result<bool, Error> {
        let mut stage = self.stage.lock().await;
        match &*stage {
            Stage::In(_) => return Ok(true),
            Stage::Out => return Ok(false),
            Stage::Off { .. } => {}
        }
        // The relay says who's on its network: until it lets this device in, nothing else will.
        let Some((relayed, online, writes)) = self.computes().await? else { return Ok(false) };
        match self.keyring.read_if_moved(self, writes).await {
            Ok(()) => {}
            // Not sealed global's keys yet: its approval is still being written.
            Err(_) if relayed.is_empty() => return Ok(false),
            Err(err) => return Err(err),
        }
        *self.order.lock().unwrap() = relayed;
        let nodes = self.ordered();
        if nodes.is_empty() {
            return Err(failed("None of your devices runs your things yet."));
        }
        let client = match self.local() {
            #[cfg(feature = "computer")]
            Some(local) => Client::beside(self.key.to_bytes(), &self.address, &nodes, local).await?,
            _ => Client::start(self.key.to_bytes(), None, &self.address, &nodes).await?,
        };
        if let Some(dir) = self.logs.clone() {
            client.answer_with(std::sync::Arc::new(move |ask| device_logs::answer(&dir, &ask)));
        }
        client.set_ranks(&self.keyring.ranks())?;
        client.set_wakes(self.keyring.cloudflare_node())?;
        client.set_online(&online)?;
        *stage = Stage::In(client);
        // Kept, with its key, so this key is known as one that was in, to its user's sign-in once the relay takes it
        // out, not to approve; a new key in the same folder (signed in again) is one to approve.
        if !self.was_in() {
            self.put("joined", self.key().as_bytes()).await.map_err(failed)?;
        }
        Ok(true)
    }

    /// Its own app runtime on this machine, as who's who lists it and where it answers, if it has both.
    #[cfg(feature = "computer")]
    pub(crate) fn local(&self) -> Option<crate::Local> {
        let key = self.keyring.own_runtime()?;
        Some(self.runtime.as_ref()?.local(key))
    }

    #[cfg(not(feature = "computer"))]
    pub(crate) fn local(&self) -> Option<crate::Local> {
        None
    }

    /// Its keyring, and its client, once it's reached its network: what signs as it (`Keyring::mine`).
    #[cfg(feature = "computer")]
    pub(crate) async fn reached(&self) -> Result<(&Keyring, Client), Error> {
        if !self.reach().await? {
            return Err(refused());
        }
        Ok((&self.keyring, self.client().await.ok_or_else(refused)?))
    }

    async fn client(&self) -> Option<Client> {
        match &*self.stage.lock().await {
            Stage::In(client) => Some(client.clone()),
            _ => None,
        }
    }

    /// Whether the relay lets this device in: → the compute endpoints on its network, which it says are there, not
    /// what they may do; those on it now; and its count of writes to `global`, if it said.
    async fn computes(&self) -> Result<Option<(Vec<[u8; 32]>, Vec<[u8; 32]>, Option<u64>)>, Error> {
        let (status, out) = self.relay_computes(json!({ "computes": true, "reads": ::custom_build::protocol::READS, "version": ::custom_build::protocol::version() })).await?;
        // The relay answers 403 only for a key it doesn't let in.
        if status == 403 {
            return Ok(None);
        }
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("{YOURS} answered {status}."))));
        }
        let keys = |v: &Value| v.as_array().into_iter().flatten().filter_map(|c| ::key::unhex(c.as_str().unwrap_or("")).ok()).collect::<Vec<_>>();
        Ok(Some((keys(&out["computes"]), keys(&out["online"]), out["moved"]["global"].as_u64())))
    }

    /// `asked` of the relay's `/relay/computes`, signed by this device: → its status and answer.
    async fn relay_computes(&self, asked: Value) -> Result<(u16, Value), Error> {
        let url = format!("{}/relay/computes", self.address);
        // Signed again once if another process of this device signed in the same millisecond, which the relay takes once.
        let mut at = ::key::next_at(now());
        let (mut status, mut out) = http("POST", &url, None, Some(::key::signed_request(&self.key, ::custom_build::protocol::RELAY, asked.clone(), at))).await?;
        if out["code"] == ::custom_build::protocol::code::REPLAYED {
            tokio::time::sleep(::key::again_in(at)).await;
            at = ::key::next_at(now());
            (status, out) = http("POST", &url, None, Some(::key::signed_request(&self.key, ::custom_build::protocol::RELAY, asked, at))).await?;
        }
        Ok((status, out))
    }

    fn ordered(&self) -> Vec<[u8; 32]> {
        let order = self.order.lock().unwrap().clone();
        self.keyring.ordered(&order)
    }

    pub async fn reconnect(&self) {
        if let Some(client) = self.client().await {
            client.reconnect().await;
        }
    }

    /// Closes its connections, until it's asked again, still knowing it was in: when its app goes to the background or
    /// a program using it ends, so a node isn't left holding a connection it counts against this device.
    pub async fn disconnect(&self) {
        let mut stage = self.stage.lock().await;
        if matches!(*stage, Stage::In(_))
            && let Stage::In(client) = std::mem::replace(&mut *stage, Stage::Off { asked: None })
        {
            client.close().await;
        }
    }

    /// Nothing more is asked as this device, and its connections close. What it kept stays, for if the relay lets it in again.
    async fn stop(&self) {
        if let Stage::In(client) = std::mem::replace(&mut *self.stage.lock().await, Stage::Out) {
            client.close().await;
        }
    }

    /// Out: signed out, or starting over. It stops, and what it kept as this device goes,
    /// as a browser's does (browser.js `forgetDevice` and `leave`): what each kind of request needed, its calls' answers
    /// and the network it was let into, which the next approval names again.
    async fn out(&self) {
        self.stop().await;
        self.keyring.forget(self).await;
        if let Some(answers) = &self.answers {
            answers.forget().await;
        }
    }

    /// Signs out: this device takes itself out of its user's devices first (`leave`), reaching the network first if
    /// its connections were closed, so signing in again makes a new one that replaces it, never a second; then it's
    /// out, and forgets it was in. A network that doesn't answer in a few seconds is left: approving the new device
    /// replaces it. The app forgets this device's key either way, as a removed key can't be let in again.
    pub async fn sign_out(&self) {
        if self.was_in() {
            let leave = async {
                if self.reach().await?
                    && let Some(client) = self.client().await
                {
                    self.keyring.leave(&client, self).await?;
                }
                Ok::<_, Error>(())
            };
            if let Ok(Err(err)) = n0_future::time::timeout(Duration::from_secs(15), leave).await {
                eprintln!("signing out: {err}");
            }
        }
        self.delete("joined").await;
        self.out().await;
    }

    /// Registers where a push service reaches it (`one::push::Address`) with the network's push Worker
    /// (`<address>/push/register`), signed with its key and naming its seal key, which each note pushed to it is sealed
    /// to, and its user as who's who lists it, so auth pushes it when another device of theirs asks to join. Once
    /// it's out of the network, the Worker forgets it.
    pub async fn register_push(&self, address: &one::push::Address) -> Result<(), Error> {
        self.keyring.register_push(self, address).await
    }

    /// Whether the network took this device out: this key was in, and the relay, which alone says who's on the
    /// network, lets it in no more. Never a node's word, which may only not have read its approval yet, nor a
    /// relay that didn't answer. It stops, keeping its key and what it kept: the app says so, and only its user
    /// signing in again, with the relay still refusing it, starts over (`start_over`).
    pub async fn removed(&self) -> bool {
        let removed = self.was_in() && matches!(self.computes().await, Ok(None));
        if removed {
            self.stop().await;
        }
        removed
    }

    /// Taken out and signed in again: what it kept as this device goes, and that this key was in, so the new key the app
    /// then makes starts clean.
    pub async fn start_over(&self) {
        self.delete("joined").await;
        self.out().await;
    }

    fn was_in(&self) -> bool {
        std::fs::read_to_string(self.dir.join("joined")).is_ok_and(|key| key.trim() == self.key())
    }

    /// A request to its network's nodes as its user (`Keyring::request`), given up on once the relay says this device
    /// is out. `/api/state` is read for the order things run in, and a screen's calls that only read are kept.
    pub async fn fetch(&self, method: &str, path: &str, headers: Vec<(String, String)>, body: impl Into<Sent>) -> Result<Response, Error> {
        let body = body.into();
        let client = match self.client().await {
            Some(client) => client,
            // Asked before it joined, or after its connections closed: it's in only if the relay lets it, else its own
            // screen says so.
            None if self.reach().await? => self.client().await.ok_or_else(refused)?,
            None => return Err(refused()),
        };
        let answered = n0_future::future::or(self.request(&client, method, path, headers, body), self.watch()).await;
        let err = match answered {
            Ok(response) => return Ok(response),
            Err(err) => err,
        };
        // A request that failed may be one a device taken out can't make: the relay says who's on its network, so it's
        // asked, whatever a node said. Out, this device starts over at joining, where nothing but its approval or sign-in
        // is shown.
        if !self.let_in().await {
            self.stop().await;
            return Err(refused());
        }
        Err(err)
    }

    /// Never answers while the relay lets this device in; once it doesn't, it's `refused`. A request that's slow may be
    /// one a device taken out can't be answered, which would otherwise wait out every node's time: asked once it's slow,
    /// then only now and then, as an agent's tool call can run for minutes and each ask is a request to the network's
    /// address.
    async fn watch(&self) -> Result<Response, Error> {
        let mut wait = 3;
        loop {
            n0_future::time::sleep(Duration::from_secs(wait)).await;
            if !self.let_in().await {
                return Err(refused());
            }
            wait = 60;
        }
    }

    async fn request(&self, client: &Client, method: &str, path: &str, headers: Vec<(String, String)>, body: Sent) -> Result<Response, Error> {
        let request = Request { method: method.into(), path: path.into(), headers, body: body.clone() };
        let response = self.keyring.request(client, self, request, now() + HANDED_FOR).await?;
        if response.status == 200 && path.split('?').next() == Some("/api/state") {
            let bytes = response.body.bytes().await?;
            self.learn_order(&bytes).await;
            return Ok(Response { body: Body::whole(bytes), ..response });
        }
        // What a screen's call answered, kept if it only reads, as the whole list of apps says which do.
        match &self.answers {
            Some(answers) => answers.passing(method, path, body.bytes(), response).await,
            None => Ok(response),
        }
    }

    /// The order things run in, from the node's `/api/state`; a node it lists that this device didn't know of has it
    /// read `global` again (now and then), and only nodes its own reading lists are asked.
    async fn learn_order(&self, bytes: &[u8]) {
        let Ok(state) = serde_json::from_slice::<scope::state::State>(bytes) else { return };
        let mut said: Vec<(usize, [u8; 32])> = state.network.iter().flat_map(|n| &n.devices).filter(|d| d.serves_me).filter_map(|d| Some((d.order, ::key::unhex(&d.key).ok()?))).collect();
        said.sort();
        let said: Vec<[u8; 32]> = said.into_iter().map(|s| s.1).collect();
        *self.order.lock().unwrap() = said.clone();
        self.keyring.reread_for(self, &said).await;
        if let Some(client) = self.client().await {
            let _ = client.set_ranks(&self.keyring.ranks());
            let _ = client.set_wakes(self.keyring.cloudflare_node());
            let _ = client.set_nodes(&self.ordered());
        }
    }

    /// Renews its user's app schedules of `app` as they use it (`Keyring::renew_app_schedules`).
    pub async fn renew_app_schedules(&self, app: &str) {
        if let Some(client) = self.client().await {
            self.keyring.renew_app_schedules(&client, self, app).await;
        }
    }

    /// What `app`'s `tool` answered last for `input` (JSON), if it only reads: its answer's JSON, and when (ms). For a
    /// screen to show at once while it asks again (bridge.js's `kept`).
    pub async fn kept(&self, app: &str, tool: &str, input: &[u8]) -> Option<(Vec<u8>, u64)> {
        self.answers.as_ref()?.kept(app, tool, input).await
    }
}

/// What it keeps, as files in its folder.
impl Keep for Device {
    async fn get(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join(name)).ok()
    }

    async fn put(&self, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        Ok(std::fs::write(self.dir.join(name), bytes)?)
    }

    async fn delete(&self, name: &str) {
        let _ = std::fs::remove_file(self.dir.join(name));
    }
}

impl ::key::Signs for Device {
    fn public(&self) -> [u8; 32] {
        self.me.key
    }

    async fn sign_bytes(&self, bytes: &[u8]) -> anyhow::Result<[u8; 64]> {
        self.key.sign_bytes(bytes).await
    }
}

impl Address for Device {
    type Bucket = Reached;

    fn address(&self) -> &str {
        &self.address
    }

    async fn ask(&self, method: &str, url: &str, header: Option<(&str, String)>, body: Option<Value>) -> Result<(u16, Value), Error> {
        http(method, url, header, body).await
    }

    fn bucket(&self) -> &Reached {
        &self.bucket
    }

    /// Who the relay says is on its network now is raced first from then on.
    async fn let_in(&self) -> bool {
        match self.computes().await {
            Ok(None) => false,
            Ok(Some((_, online, _))) => {
                if let Some(client) = self.client().await {
                    let _ = client.set_online(&online);
                }
                true
            }
            _ => true,
        }
    }
}

/// A request to the network's address (auth, the relay), JSON in and out: → its status and JSON.
async fn http(method: &str, url: &str, header: Option<(&str, String)>, body: Option<Value>) -> Result<(u16, Value), Error> {
    let (method, url, header) = (method.to_string(), url.to_string(), header.map(|(n, v)| (n.to_string(), v)));
    tokio::task::spawn_blocking(move || {
        let mut request = ureq::http::Request::builder().method(method.as_str()).uri(&url);
        if let Some((name, value)) = header {
            request = request.header(name, value);
        }
        let request = match body {
            Some(body) => request.header("content-type", "application/json").body(::key::canonical(&body)).map_err(failed)?,
            None => request.body(Vec::new()).map_err(failed)?,
        };
        let twice = if method == "GET" { platform::hedge::Twice::Safe } else { platform::hedge::Twice::Never };
        let (mut response, _) = platform::hedge::run(&crate::bucket::AGENT, "address", request, twice).map_err(|e| failed(format!("Couldn't reach {YOURS}: {e}")))?;
        let status = response.status().as_u16();
        Ok((status, response.body_mut().read_json().unwrap_or(Value::Null)))
    })
    .await
    .map_err(failed)?
}

