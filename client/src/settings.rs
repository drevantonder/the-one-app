//! A client device as the device that signs Settings' changes (`one::settings`): its keyring, asked as a node's routes
//! of the same paths (`Keyring::request`), answers them itself, reading who's who as it read it and writing through its
//! nodes; and what only a client does, leaving as it signs out and registering for pushes.

use one::database::content::Content;
use one::database::scopes::{Scope, ScopeDb};
use one::database::steps::Welcome;
use one::keys::{self, Opens, Signs, hex};
use one::protocol::{Coded, code};
use serde_json::{Value, json};

use crate::keyring::{Address, Keep, Keyring, error, now, query, refused};
use crate::{Client, Error, not_let_in, unsent};

/// How often who's who is read again to renew app schedules, so it renews what its user's other devices set too.
const HOUR: u64 = 3_600_000;

/// A keyring with what it asks through, as the device `one::settings` signs as.
struct Mine<'a, O, D> {
    keyring: &'a Keyring<O>,
    client: &'a Client,
    on: &'a D,
}

/// A client's error, as `one::settings` reads it: its code kept, and not being let in as `code::NOT_LET_IN`.
fn to_one(err: Error) -> anyhow::Error {
    match (err.not_let_in, err.code.clone()) {
        (true, _) => Coded::error(code::NOT_LET_IN, err),
        (_, Some(c)) => Coded::error(&c, err),
        _ => anyhow::anyhow!("{err}"),
    }
}

/// `one::settings`' error, as a client's.
fn of_one(err: anyhow::Error) -> Error {
    match Coded::of(&err) {
        Some(c) if c == code::NOT_LET_IN => not_let_in(format!("{err:#}")),
        c => Error { code: c, ..unsent(format!("{err:#}")) },
    }
}

impl<O: Opens, D: Address + Keep + Signs> Signs for Mine<'_, O, D> {
    fn public(&self) -> [u8; 32] {
        self.on.public()
    }

    async fn sign_bytes(&self, bytes: &[u8]) -> anyhow::Result<[u8; 64]> {
        self.on.sign_bytes(bytes).await
    }
}

impl<O: Opens, D: Address + Keep + Signs> one::settings::Me for Mine<'_, O, D> {
    fn who<R>(&self, f: impl FnOnce(&ScopeDb) -> anyhow::Result<R>) -> anyhow::Result<R> {
        self.keyring.who(f).ok_or_else(|| to_one(refused()))?
    }

    fn user(&self) -> anyhow::Result<String> {
        self.keyring.user().ok_or_else(|| to_one(refused()))
    }

    fn seal(&self) -> [u8; 32] {
        self.keyring.opens().public()
    }

    fn welcome(&self) -> Option<Welcome> {
        self.keyring.welcome()
    }

    async fn read(&self) -> anyhow::Result<()> {
        self.keyring.read(self.on).await.map_err(to_one)
    }

    async fn write(&self, content: Content) -> anyhow::Result<()> {
        self.keyring.write(self.client, self.on, content).await.map_err(to_one)
    }

    /// Each sealed to it, as a node keeps them, opened and checked against the hash who's who names.
    async fn held(&self, scope: &Scope) -> anyhow::Result<Vec<(u32, [u8; 32])>> {
        let mine = self.keyring.asked(self.client, self.on, "GET", &format!("/v1/keys?scope={}&to={}", query(&scope.to_string()), hex(&self.on.public())), None).await.map_err(to_one)?;
        let mut out = Vec::new();
        for (n, sealed) in mine["sealed"].as_object().into_iter().flatten() {
            let n: u32 = n.parse()?;
            let key = keys::open_sealed_by(self.keyring.opens(), &data_encoding::BASE64URL_NOPAD.decode(sealed.as_str().unwrap_or("").as_bytes())?).await?;
            let key: [u8; 32] = key.try_into().map_err(|_| anyhow::anyhow!("A key for this didn't check out, so it wasn't used."))?;
            anyhow::ensure!(self.who(|who| who.epoch_hash(scope, n))? == Some(keys::epoch_hash(&key)), "A key for this didn't check out, so it wasn't used.");
            out.push((n, key));
        }
        Ok(out)
    }

    async fn keep_key(&self, sealed: Value) -> anyhow::Result<()> {
        self.keyring.asked(self.client, self.on, "POST", "/v1/keys", Some(sealed)).await.map(|_| ()).map_err(to_one)
    }

    async fn app_key(&self, scope: &Scope, app: &str, n: u32) -> anyhow::Result<[u8; 32]> {
        self.keyring.app_key(self.client, self.on, &scope.to_string(), app, n).await.map_err(to_one)
    }

    fn made_epoch(&self, scope: &Scope, n: u32, key: [u8; 32]) {
        self.keyring.made_epoch(scope, n, key);
    }

    fn address(&self) -> Option<String> {
        Some(self.on.address().to_string())
    }

    async fn ask(&self, method: &str, url: &str, header: Option<(&str, String)>, body: Option<Value>) -> anyhow::Result<(u16, Value)> {
        self.on.ask(method, url, header, body).await.map_err(to_one)
    }

    async fn issuer_keys(&self, url: &str) -> Option<Value> {
        self.on.ask("GET", url, None, None).await.ok().filter(|(s, _)| *s == 200).map(|(_, keys)| keys)
    }

    async fn kept(&self, name: &str) -> Option<Vec<u8>> {
        self.on.get(name).await
    }

    async fn keep(&self, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
        self.on.put(name, bytes).await
    }

    fn now(&self) -> u64 {
        now()
    }

    async fn wait(&self, ms: u64) {
        n0_future::time::sleep(std::time::Duration::from_millis(ms)).await;
    }

    async fn leave(&self) -> anyhow::Result<()> {
        self.keyring.leave(self.client, self.on).await.map_err(to_one)
    }

    async fn stored(&self, name: &str) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(one::bucket::Bucket::get(self.on.bucket(), name).await?.map(|o| o.bytes))
    }

    async fn open(&self, sealed: &[u8]) -> anyhow::Result<Vec<u8>> {
        keys::open_sealed_by(self.keyring.opens(), sealed).await
    }
}

impl<O: Opens> Keyring<O> {
    fn mine<'a, D>(&'a self, client: &'a Client, on: &'a D) -> Mine<'a, O, D> {
        Mine { keyring: self, client, on }
    }

    /// The answer to a request this device answers itself (`one::settings::answer`): none for any other, which goes to
    /// a node.
    pub(crate) async fn answer(&self, client: &Client, on: &(impl Address + Keep + Signs), method: &str, path: &str, body: &[u8]) -> Option<Result<Value, Error>> {
        Some(one::settings::answer(&self.mine(client, on), method, path, body).await?.map_err(of_one))
    }

    /// Signs a step of `content` in `global`'s scope database for the next position (`one::database::steps`), and
    /// writes it, signing it again for the one after while another device's goes first.
    pub(crate) async fn write(&self, client: &Client, on: &(impl Address + Keep + Signs), content: Content) -> Result<(), Error> {
        let user = self.user().ok_or_else(refused)?;
        let newest = format!("/v1/steps/global?from={}", one::database::steps::NUMBER_MAX);
        let mut steps = self.asked(client, on, "GET", &newest, None).await?;
        for _ in 0..5 {
            let (at, epoch) = (steps["at"].as_u64().unwrap_or(0), steps["epoch"].as_u64().unwrap_or(0) as u32);
            let step = one::database::steps::sign_by(on, &Scope::Global, at + 1, &user, epoch, now(), content.clone()).await.map_err(unsent)?;
            match self.asked(client, on, "POST", "/v1/steps", Some(json!([step.to_json()]))).await {
                Ok(_) => return Ok(()),
                Err(err) if err.code.as_deref() == Some(code::TAKEN) => steps = self.asked(client, on, "GET", &newest, None).await?,
                Err(err) => return Err(err),
            }
        }
        Err(unsent("Who's who changed too often to write to: try again."))
    }

    /// As each start of a client does (`one::settings::settle_own`).
    pub async fn settle_own(&self, client: &Client, on: &(impl Address + Keep + Signs), on_device: Option<&str>, platform: Option<&str>) -> Result<(), Error> {
        one::settings::settle_own(&self.mine(client, on), on_device, platform).await.map_err(of_one)
    }

    /// Takes this device out of its user's devices, as it signs out, so signing in again makes a new one that
    /// replaces it, never a second. Nothing it writes after counts, so it goes in this order: its removal, then off the
    /// network. Its scopes keep their keys, as it forgets its own (ARCHITECTURE.md).
    pub async fn leave(&self, client: &Client, on: &(impl Address + Keep + Signs)) -> Result<(), Error> {
        self.read(on).await?;
        // A node closes a removed device's connection as it applies its removal, so its answer may never come: off
        // the network all the same.
        let wrote = self.write(client, on, one::database::content::remove_device(&on.public())).await;
        one::settings::take_out(&self.mine(client, on), &on.public()).await.map_err(of_one)?;
        // One sent whose answer was lost is that removal applied: only one never sent failed.
        match wrote {
            Err(err) if !err.sent => Err(err),
            _ => Ok(()),
        }
    }

    /// Renews this user's app schedules of `app` as they use it (`one::settings::renew_app_schedules`), reading
    /// who's who again first at most once an hour.
    pub async fn renew_app_schedules(&self, client: &Client, on: &(impl Address + Keep + Signs), app: &str) {
        if self.renew_checked(HOUR) {
            let _ = self.read(on).await;
        }
        one::settings::renew_app_schedules(&self.mine(client, on), app).await;
    }

    /// Registers where a push service reaches this device (`one::push::Address`) with the network's push Worker, signed
    /// with its key and naming its seal key, which each note pushed to it is sealed to, and its user, so auth pushes
    /// it when another device of theirs asks to join. Once it's out of the network, the Worker forgets it.
    pub async fn register_push(&self, on: &(impl Address + Signs), address: &one::push::Address) -> Result<(), Error> {
        let fields = one::push::register_fields(&self.opens().public(), address, self.user().as_deref()).map_err(unsent)?;
        let body = keys::signed_request_by(on, one::push::DOMAIN, fields, now()).await.map_err(unsent)?;
        let (status, out) = on.ask("POST", &format!("{}/push/register", on.address()), None, Some(body)).await?;
        if status != 200 {
            return Err(unsent(error(&out).unwrap_or_else(|| format!("{}'s push service didn't take this device ({status}).", one::config::YOURS))));
        }
        Ok(())
    }
}
