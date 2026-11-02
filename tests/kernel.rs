//! The kernel services (`kernel/`) running in the pinned workerd, as nodes and the auth service use them.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::BASE64URL_NOPAD as B64U;
use one::kernel::{self, Served};
use serde_json::{Value, json};

/// The kernel's files as they are in this checkout, tests and all, with the screens' design kit where build.rs puts it.
fn files() -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push((path.strip_prefix(root).unwrap().to_string_lossy().into(), std::fs::read(&path).unwrap()));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("kernel");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    let screen = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/screen");
    let mut kit = Vec::new();
    walk(&screen, &screen, &mut kit);
    out.extend(kit.into_iter().map(|(path, bytes)| (format!("web/screen/{path}"), bytes)));
    out
}

fn module(entry: &str) -> Vec<u8> {
    kernel::module_from(files(), entry).unwrap()
}

/// Sockets listening on ports `port` gave out, until `run` gives them to the workerd serving there.
static LISTENING: std::sync::Mutex<Vec<std::net::TcpListener>> = std::sync::Mutex::new(Vec::new());

/// A port of this machine's that stays ours: nothing else, another test's workerd included, can take it.
fn port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    LISTENING.lock().unwrap().push(listener);
    port
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// A workerd running kernel services, stopped when dropped.
struct Running {
    child: std::process::Child,
    dir: std::path::PathBuf,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run(services: Vec<Served>) -> Running {
    let dir = crate::temp("workerd");
    let listening = {
        let mut held = LISTENING.lock().unwrap();
        services
            .iter()
            .map(|s| {
                let at = held.iter().position(|l| l.local_addr().unwrap().to_string() == s.listen).expect("a service listens on a port from port()");
                (s.name.clone(), held.swap_remove(at))
            })
            .collect()
    };
    let config = kernel::write_config(&dir, &services).unwrap();
    let child = kernel::start(&config, listening).unwrap();
    Running { child, dir }
}

fn post(url: &str, headers: &[(&str, &str)], body: Value) -> (u16, Value) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let mut req = agent.post(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut resp = req.send_json(body).unwrap();
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

#[test]
fn the_kernels_own_tests_pass() {
    let dir = crate::temp("kernel-tests");
    std::fs::write(dir.join("tests.js"), module("test/tests.js")).unwrap();
    std::fs::write(
        dir.join("config.capnp"),
        format!(
            "using Workerd = import \"/workerd/workerd.capnp\";\nconst config :Workerd.Config = (services = [(name = \"tests\", worker = (modules = [(name = \"tests.js\", esModule = embed \"tests.js\")], compatibilityDate = \"{}\"))]);\n",
            one::apps::COMPATIBILITY_DATE
        ),
    )
    .unwrap();
    let out = std::process::Command::new(one::apps::workerd().unwrap()).arg("test").arg(dir.join("config.capnp")).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "the kernel's tests failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

/// A vault, and a stand-in auth service that signs any claims.
struct Vault {
    _running: Running,
    url: String,
    /// The stand-in bucket, at `<sites>/install/<name>`.
    sites: String,
    auth: String,
    issuer: String,
    secret: String,
}

impl Vault {
    fn start() -> Vault {
        Vault::start_with(vec![])
    }

    /// A vault with `more` bindings besides its own.
    fn start_with(more: Vec<(String, String)>) -> Vault {
        let (vault, auth, sites) = (port(), port(), port());
        let (_, recovery_public) = kernel::twin(&rand::random());
        let secret = B64U.encode(&rand::random::<[u8; 32]>());
        // Nodes fetch its keys from the address their sign-ins are for, as from the auth service at an install's.
        let issuer = format!("http://127.0.0.1:{auth}/api/auth");
        let bucket = json!({ "endpoint": "https://s3.test", "bucket": "install", "region": "auto", "key_id": "id", "secret": "bucket secret" }).to_string();
        let running = run(vec![
            Served { name: "auth".into(), module: module("test/auth.js"), class: None, listen: format!("127.0.0.1:{auth}"), bindings: vec![], assets: None, outbound: None },
            Served { name: "sites".into(), module: module("test/sites.js"), class: None, listen: format!("127.0.0.1:{sites}"), bindings: vec![], assets: None, outbound: None },
            Served {
                name: "vault".into(),
                module: module("vault/worker.js"),
                class: Some("Vault".into()),
                listen: format!("127.0.0.1:{vault}"),
                bindings: vec![
                    ("AUTH".into(), "service:auth".into()),
                    ("ISSUER".into(), issuer.clone()),
                    ("KERNEL_SECRET".into(), secret.clone()),
                    ("RECOVERY_KEY".into(), B64U.encode(&recovery_public)),
                    ("BUCKET".into(), bucket),
                ]
                .into_iter()
                .chain(more)
                .collect(),
                assets: None,
                outbound: Some("sites".into()),
            },
        ]);
        Vault {
            _running: running,
            url: format!("http://127.0.0.1:{vault}"),
            sites: format!("http://127.0.0.1:{sites}"),
            auth: format!("http://127.0.0.1:{auth}"),
            issuer,
            secret,
        }
    }

    /// Asks as the auth service does.
    fn internal(&self, path: &str, body: Value) -> (u16, Value) {
        post(&format!("{}/internal/{path}", self.url), &[("x-kernel-secret", &self.secret)], body)
    }

    /// An access token for `user` from `client`, good for an hour, for the install at `auth`'s address.
    fn token(&self, user: &str, client: &str) -> (String, u64) {
        let exp = now() / 1000 + 3600;
        let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
        let token = agent.post(format!("{}/sign", self.auth)).send_json(json!({ "iss": self.issuer, "sub": user, "azp": client, "client_id": client, "aud": self.auth, "scope": "openid", "exp": exp, "jti": B64U.encode(&rand::random::<[u8; 16]>()) })).unwrap().body_mut().read_to_string().unwrap();
        (token, exp * 1000)
    }

    /// A client of `user`'s, added with their passkey: → its access token.
    fn client(&self, user: &str, passkey: &str, client: &str) -> String {
        let code = format!("{client}'s authorization code for {user}");
        assert_eq!(self.internal("grant", json!({ "user": user, "from": passkey, "to": code, "client": client })).0, 200);
        let (token, until) = self.token(user, client);
        assert_eq!(self.internal("rekey", json!({ "from": code, "to": format!("{client}'s refresh token for {user}"), "token": token, "until": until })).1["moved"], true);
        token
    }

    fn ask(&self, token: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
        self.ask_at(&self.url, token, method, path, body)
    }

    fn ask_at(&self, server: &str, token: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
        ask_at(server, token, method, path, body)
    }
}

/// Asks a kernel service with a bearer token.
fn ask_at(server: &str, token: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let url = format!("{server}{path}");
    let mut resp = match method {
        "GET" => agent.get(url).header("authorization", format!("Bearer {token}")).call().unwrap(),
        "DELETE" => agent.delete(url).header("authorization", format!("Bearer {token}")).call().unwrap(),
        _ => agent.post(url).header("authorization", format!("Bearer {token}")).send_json(body).unwrap(),
    };
    (resp.status().as_u16(), resp.body_mut().read_json().unwrap())
}

const ANDRE: &str = "00000000000000a1";
const SAM: &str = "00000000000000b2";

#[test]
fn nodes_get_create_only_urls_for_objects_in_scopes_they_may_use() {
    let v = Vault::start();
    let vault = kernel::Vault { url: v.url.clone() };
    v.internal("passkey", json!({ "user": ANDRE, "secret": "andre's passkey prf output" }));
    v.internal("passkey", json!({ "user": SAM, "secret": "sam's own passkey prf output" }));
    let mine = one::bucket::folder(&format!("user:{ANDRE}"));
    v.internal("grant", json!({ "user": ANDRE, "from": "andre's passkey prf output", "to": "andre's code here", "client": "iphone" }));
    let (token, until) = v.token(ANDRE, "iphone");
    v.internal("rekey", json!({ "from": "andre's code here", "to": "andre's refresh token", "token": token, "until": until }));

    let name = format!("{mine}/board/main/changes/00000000000000000001");
    let newest = format!("{mine}/board/main/newest");
    let urls = vault.urls(&token, &[&format!("{mine}/board/main/copy")], &[&name], &[(&newest, "\"etag-1\"")]).unwrap();
    assert_eq!(url_query(&urls.replace[&newest])["X-Amz-SignedHeaders"], "host;if-match");
    let create = url_query(&urls.create[&name]);
    assert!(urls.create[&name].starts_with(&format!("https://s3.test/install/{mine}/board/main/changes/")));
    assert_eq!(create["X-Amz-SignedHeaders"], "host;if-none-match");
    assert!(urls.until > now());
    assert_eq!(url_query(&urls.read[&format!("{mine}/board/main/copy")])["X-Amz-SignedHeaders"], "host");

    // Sam's objects, and names that climb out of an app, aren't Andre's to reach; Everyone's are (he's its first).
    let sams = format!("{}/board/main/copy", one::bucket::folder(&format!("user:{SAM}")));
    assert!(vault.urls(&token, &[&sams], &[], &[]).is_err());
    assert!(vault.urls(&token, &[], &[&format!("{mine}/board/../../x")], &[]).is_err());
    assert!(vault.urls(&token, &[], &[&format!("{mine}/board")], &[]).is_err());
    assert!(vault.urls(&token, &[], &[], &[(&name, "\"etag-1\"")]).is_err(), "only a newest is replaced");
    assert!(vault.urls(&token, &["log/00aa/000000000001"], &["log/00aa/000000000002"], &[]).is_ok());
    assert!(vault.urls(&token, &["global/board/main/newest"], &[], &[]).is_ok());

    // Once an admin's machine says, from the install's log, that Sam may open Everyone, Sam's clients may reach it too.
    let global = B64U.encode(&rand::random::<[u8; 32]>());
    assert_eq!(v.internal("scope-key", json!({ "scope": "global", "key": global, "people": [ANDRE, SAM] })).0, 200);
    v.internal("grant", json!({ "user": SAM, "from": "sam's own passkey prf output", "to": "sam's authorization code", "client": "iphone" }));
    let (sam, until) = v.token(SAM, "iphone");
    v.internal("rekey", json!({ "from": "sam's authorization code", "to": "sam's refresh token", "token": sam, "until": until }));
    let andre = vault.keys_until(&token, &[("global", "board", None)]).map(|k| k.0).unwrap();
    assert_eq!(vault.keys_until(&sam, &[("global", "board", None)]).map(|k| k.0).unwrap(), andre);
    assert!(vault.keys_until(&sam, &[(&format!("user:{ANDRE}"), "board", None)]).is_err());
    // Nobody opens a scope to anyone else on their own.
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let shared = agent.post(format!("{}/share", v.url)).header("authorization", format!("Bearer {token}")).send_json(json!({ "scope": format!("user:{ANDRE}"), "user": SAM })).unwrap();
    assert_eq!(shared.status(), 404);

    // Once the log says Sam may not, it's taken away: his sessions get no key and no URL under it, while Andre's do.
    assert_eq!(v.internal("scope-key", json!({ "scope": "global", "key": global, "people": [ANDRE] })).0, 200);
    assert!(vault.keys_until(&sam, &[("global", "board", None)]).is_err());
    assert!(vault.urls(&sam, &["global/board/main/newest"], &[], &[]).is_err());
    assert_eq!(vault.keys_until(&token, &[("global", "board", None)]).map(|k| k.0).unwrap(), andre);
    // Saying so never replaces the key it was settled with.
    assert_eq!(v.internal("scope-key", json!({ "scope": "global", "key": B64U.encode(&rand::random::<[u8; 32]>()), "people": [ANDRE, SAM] })).0, 409);
    assert!(vault.keys_until(&sam, &[("global", "board", None)]).is_err());
}

#[test]
fn where_people_sign_in_with_google_the_vault_opens_their_keys_itself_and_only_there() {
    let passkeys = Vault::start();
    assert_eq!(passkeys.internal("own", json!({ "user": ANDRE, "to": "a code for andre", "client": "web" })).0, 403, "a passkey's install opens nobody's keys for them");

    let v = Vault::start_with(vec![("SIGN_IN".into(), "google".into()), ("VAULT_KEY".into(), B64U.encode(&rand::random::<[u8; 32]>()))]);
    let vault = kernel::Vault { url: v.url.clone() };
    assert_eq!(v.internal("passkey", json!({ "user": ANDRE, "secret": "andre's passkey prf output" })).0, 403, "nor does a passkey open them here");
    // Each time Google signs someone in, the vault gives their new client the same keys.
    let signs_in = |user: &str, client: &str| {
        let code = format!("{client}'s authorization code for {user}");
        assert_eq!(v.internal("own", json!({ "user": user, "to": code, "client": client })).0, 200);
        let (token, until) = v.token(user, client);
        assert_eq!(v.internal("rekey", json!({ "from": code, "to": format!("{client}'s refresh token for {user}"), "token": token, "until": until })).1["moved"], true);
        token
    };
    let mine = format!("user:{ANDRE}");
    let (web, phone) = (signs_in(ANDRE, "web"), signs_in(ANDRE, "phone"));
    let key = vault.keys_until(&web, &[(&mine, "board", None)]).unwrap().0;
    assert_eq!(vault.keys_until(&phone, &[(&mine, "board", None)]).unwrap().0, key);
    // Someone who joins after him opens Everyone's, and not his.
    let sam = signs_in(SAM, "web");
    assert_eq!(vault.keys_until(&sam, &[("global", "board", None)]).unwrap().0, vault.keys_until(&web, &[("global", "board", None)]).unwrap().0);
    assert!(vault.keys_until(&sam, &[(&mine, "board", None)]).is_err());
    // Only the auth service asks, once Google has said yes.
    let (status, _) = post(&format!("{}/internal/own", v.url), &[("x-kernel-secret", "a guess")], json!({ "user": SAM, "to": "mallory's code", "client": "web" }));
    assert_eq!(status, 403);
}

#[test]
fn who_opens_a_scope_changes_at_once_and_only_as_its_admins_say() {
    use one::entry::Scope;
    use one::members::Role::{Admin, Member};
    const LEE: &str = "00000000000000d4";
    let group = Scope::parse("group:00000000000000c3").unwrap();
    let (everyone, lees) = (Scope::Global, Scope::User(LEE.into()));
    let one = |s: &Scope| vec![s.clone()];
    // On an install whose people sign in with a passkey, and on one whose people Google signs in.
    for google in [false, true] {
        let v = if google { Vault::start_with(vec![("SIGN_IN".into(), "google".into()), ("VAULT_KEY".into(), B64U.encode(&rand::random::<[u8; 32]>()))]) } else { Vault::start() };
        let vault = kernel::Vault { url: v.url.clone() };
        let signs_in = |user: &str, client: &str| {
            if !google {
                v.internal("passkey", json!({ "user": user, "secret": format!("{user}'s passkey prf output") }));
                return v.client(user, &format!("{user}'s passkey prf output"), client);
            }
            let code = format!("{client}'s authorization code for {user}");
            assert_eq!(v.internal("own", json!({ "user": user, "to": code, "client": client })).0, 200);
            let (token, until) = v.token(user, client);
            v.internal("rekey", json!({ "from": code, "to": format!("{client}'s refresh token for {user}"), "token": token, "until": until }));
            token
        };
        let (andre, sam, lee) = (signs_in(ANDRE, "web"), signs_in(SAM, "web"), signs_in(LEE, "web"));
        // As the install's log says: Andre is its admin and all three are in it; Sam is the group's only one.
        let key = || B64U.encode(&rand::random::<[u8; 32]>());
        assert_eq!(v.internal("scope-key", json!({ "scope": "global", "key": key(), "people": [ANDRE, SAM, LEE], "admins": [ANDRE] })).0, 200);
        assert_eq!(v.internal("scope-key", json!({ "scope": group.to_string(), "key": key(), "people": [SAM], "admins": [SAM] })).0, 200);
        let opens = |token: &str, scope: &Scope| vault.keys_until(token, &[(&scope.to_string(), "board", None)]).map(|k| k.0[0].key).ok();
        assert!(opens(&lee, &group).is_none());

        // Lee, a member, lets nobody in, himself included, and takes nobody out; Andre, who doesn't open the group,
        // can't let anyone in to it, though he's the install's admin.
        assert!(vault.role(&lee, LEE, Some(Member), false, &one(&group)).is_err(), "nobody invited him");
        assert!(vault.role(&lee, LEE, Some(Member), true, &one(&group)).is_err());
        assert!(vault.role(&lee, SAM, None, false, &one(&group)).is_err());
        assert!(vault.role(&lee, ANDRE, None, false, &one(&everyone)).is_err());
        assert!(vault.role(&sam, LEE, None, false, &one(&everyone)).is_err(), "Sam isn't the install's admin");
        assert!(vault.role(&andre, LEE, Some(Member), true, &one(&group)).is_err());
        assert!(vault.role(&lee, LEE, Some(Admin), false, &one(&everyone)).is_err(), "nobody makes themselves an admin");
        assert!(opens(&lee, &group).is_none() && opens(&sam, &group).is_some());

        // Sam invites Lee as a member: Lee opens it once he accepts, as that and nothing more, with Sam's key.
        vault.role(&sam, LEE, Some(Member), true, &one(&group)).unwrap();
        assert!(opens(&lee, &group).is_none(), "an invite opens nothing yet");
        assert!(vault.role(&lee, LEE, Some(Admin), false, &one(&group)).is_err(), "he was invited as a member");
        vault.role(&lee, LEE, Some(Member), false, &one(&group)).unwrap();
        assert_eq!(opens(&lee, &group), opens(&sam, &group));
        assert!(vault.role(&lee, SAM, None, false, &one(&group)).is_err(), "a member takes nobody out");

        // Made an admin, Lee takes Sam out of it; Sam, out, neither opens it nor lets himself back in.
        vault.role(&sam, LEE, Some(Admin), false, &one(&group)).unwrap();
        vault.role(&lee, SAM, None, false, &one(&group)).unwrap();
        assert!(opens(&sam, &group).is_none() && opens(&lee, &group).is_some() && opens(&sam, &everyone).is_some());
        assert!(vault.role(&sam, SAM, Some(Member), false, &one(&group)).is_err());
        assert!(vault.urls(&sam, &[&format!("{}/board/main/newest", one::bucket::folder(&group.to_string()))], &[], &[]).is_err());

        // An admin who stops being one says so of himself, and from then on says nobody's role.
        vault.role(&lee, LEE, Some(Member), false, &one(&group)).unwrap();
        assert!(vault.role(&lee, SAM, Some(Member), true, &one(&group)).is_err());
        assert!(opens(&lee, &group).is_some(), "he's still in it");

        // Andre takes Lee out of the install: Lee opens nothing, and says nobody's role any more.
        vault.role(&andre, LEE, None, false, &[everyone.clone(), group.clone(), lees.clone()]).unwrap();
        assert!(opens(&lee, &everyone).is_none() && opens(&lee, &group).is_none() && opens(&lee, &lees).is_none());
        assert!(vault.urls(&lee, &["global/board/main/newest"], &[], &[]).is_err());
        assert!(vault.role(&lee, SAM, Some(Member), true, &one(&group)).is_err());
        if google {
            // Google signing him in again opens Everyone's to him no more.
            assert!(opens(&signs_in(LEE, "phone"), &everyone).is_none());
        }
        // A group the vault has no key for yet changes nothing here.
        vault.role(&andre, SAM, Some(Member), true, &one(&Scope::parse("group:00000000000000e5").unwrap())).unwrap();
        assert!(opens(&andre, &everyone).is_some() && opens(&sam, &everyone).is_some());
    }
}

#[test]
fn the_vault_given_the_installs_keys_opens_what_its_nodes_wrote() {
    let v = Vault::start();
    let vault = kernel::Vault { url: v.url.clone() };
    v.internal("passkey", json!({ "user": ANDRE, "secret": "andre's passkey prf output" }));
    let token = v.client(ANDRE, "andre's passkey prf output", "web");

    // Before the install had a kernel, its nodes sealed the Board's key under Everyone's.
    let (global, board): ([u8; 32], [u8; 32]) = (rand::random(), rand::random());
    let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
    agent.put(format!("{}/install/global/board/_key", v.sites)).send(&kernel::seal_app_key(&global, "board", &board)[..]).unwrap();
    agent.put(format!("{}/install/global/_key", v.sites)).send(one::backups::key_id(&global).as_bytes()).unwrap();

    // Until an admin gives it, the vault gives out no key under its own, and seals no item under it.
    assert_eq!(v.ask(&token, "POST", "/keys", json!({ "keys": [{ "scope": "global", "app": "board" }] })).0, 409);
    let item = json!({ "id": "0123456789abcdef", "title": "A key", "hosts": ["api.example.com"], "apps": ["board"], "fields": { "key": { "generate": true } } });
    assert_eq!(v.ask(&token, "POST", "/items", json!({ "scope": "global", "item": item })).0, 409);

    // An admin's machine gives the vault Everyone's key, for Andre and for Sam, who has no keys here yet.
    let (status, out) = v.internal("scope-key", json!({ "scope": "global", "key": B64U.encode(&global), "people": [ANDRE, SAM] }));
    assert_eq!((status, out["granted"].clone()), (200, json!([ANDRE])), "{out}");
    let given = vault.keys_until(&token, &[("global", "board", Some("main"))]).unwrap().0;
    assert_eq!(given[0].key, kernel::database_key(&board, "main"), "the vault opens the Board the nodes wrote");

    // An app whose data was written before app keys were random has no key in the bucket: the vault keys it as the
    // nodes do, with the key its data was sealed with, never a new one that would open nothing.
    agent.put(format!("{}/install/global/notes/main/000000000001", v.sites)).send(&b"a step"[..]).unwrap();
    let given = vault.keys_until(&token, &[("global", "notes", Some("main"))]).unwrap().0;
    assert_eq!(given[0].key, kernel::database_key(&kernel::derived_app_key(&global, "notes"), "main"), "the vault opens the Notes the nodes wrote");

    // Once given, its key stays; and so does that of a scope nodes made none for, once an item is sealed under it.
    assert_eq!(v.internal("scope-key", json!({ "scope": "global", "key": B64U.encode(&rand::random::<[u8; 32]>()), "people": [ANDRE] })).0, 409);
    let mine = format!("user:{ANDRE}");
    assert_eq!(v.ask(&token, "POST", "/items", json!({ "scope": mine, "item": item })).0, 200);
    assert_eq!(v.internal("scope-key", json!({ "scope": mine, "key": B64U.encode(&rand::random::<[u8; 32]>()), "people": [ANDRE] })).0, 409);
}

fn url_query(url: &str) -> std::collections::BTreeMap<String, String> {
    let query = url.split_once('?').unwrap().1;
    query.split('&').map(|p| p.split_once('=').unwrap()).map(|(k, v)| (k.to_string(), v.replace("%3B", ";"))).collect()
}

#[test]
fn the_vault_fills_an_apps_requests_with_secrets_it_never_shows() {
    let v = Vault::start();
    v.internal("passkey", json!({ "user": ANDRE, "secret": "andre's passkey prf output" }));
    let token = v.client(ANDRE, "andre's passkey prf output", "mac");
    let me = format!("user:{ANDRE}");
    let id = "0123456789abcdef";
    let item = json!({ "id": id, "title": "Google sign-in", "type": "Google account", "hosts": ["oauth2.googleapis.com"], "apps": ["gmail"],
        "fields": { "client_id": { "value": "the client id" }, "refresh_token": { "value": "the refresh token itself", "secret": true } } });
    assert_eq!(v.ask(&token, "POST", "/items", json!({ "scope": me, "item": item })).0, 200);

    // Listed, a secret field says only that it's secret.
    let (_, list) = v.ask(&token, "GET", "/items", json!({}));
    assert!(!list.to_string().contains("the refresh token itself"));
    assert_eq!(list["items"][0]["fields"], json!({ "client_id": { "value": "the client id" }, "refresh_token": { "secret": true } }));

    // A field taken from a secret one stays secret, whatever it's asked to be.
    let copied = json!({ "id": id, "fields": { "copied": { "from": "refresh_token", "secret": false } } });
    assert_eq!(v.ask(&token, "POST", "/items", json!({ "scope": me, "item": copied })).0, 200);
    let (_, list) = v.ask(&token, "GET", "/items", json!({}));
    assert!(!list.to_string().contains("the refresh token itself"));
    assert_eq!(list["items"][0]["fields"]["copied"], json!({ "secret": true }));

    // Standing work, let use only some apps, changes no item.
    let (_, standing) = v.internal("standing", json!({ "token": token, "scopes": [me], "apps": ["calendar"] }));
    let (_, run) = v.internal("run", json!({ "secret": standing["secret"], "until": now() + 600_000 }));
    let run = run["token"].as_str().unwrap();
    let moved = json!({ "id": id, "hosts": ["evil.example.com"], "apps": ["calendar"] });
    assert_eq!(v.ask(run, "POST", "/items", json!({ "scope": me, "item": moved })).0, 400);

    // Gmail's refresh goes out with it filled in, to its host, through the vault.
    let fetch = |app: &str, url: &str, body: &str| v.ask(&token, "POST", "/fetch", json!({ "app": app, "url": url, "method": "POST", "headers": { "content-type": "text/plain" }, "body": body }));
    let (status, sent) = fetch("gmail", "https://oauth2.googleapis.com/token", &format!("grant_type=refresh_token&refresh_token={{{{{id}:refresh_token}}}}&client_id={{{{client_id}}}}"));
    assert_eq!(status, 200, "{sent}");
    let echoed: Value = serde_json::from_slice(&B64U.decode(sent["body64"].as_str().unwrap().as_bytes()).unwrap()).unwrap();
    assert_eq!(echoed["body"], "grant_type=refresh_token&refresh_token=the refresh token itself&client_id=the client id");
    assert_eq!(echoed["url"], "https://oauth2.googleapis.com/token");

    // Never to another host, never for an app it isn't switched on for, never over plain http.
    let (status, refused) = fetch("gmail", "https://evil.example.com/", &format!("{{{{{id}:refresh_token}}}}"));
    assert_eq!(status, 400);
    assert!(refused["error"].as_str().unwrap().contains("sent only to oauth2.googleapis.com"));
    let (_, other) = fetch("calendar", "https://oauth2.googleapis.com/token", &format!("{{{{{id}:refresh_token}}}}"));
    let echoed: Value = serde_json::from_slice(&B64U.decode(other["body64"].as_str().unwrap().as_bytes()).unwrap()).unwrap();
    assert_eq!(echoed["body"], format!("{{{{{id}:refresh_token}}}}"));
    assert_eq!(fetch("gmail", "http://oauth2.googleapis.com/token", "x").0, 400);

    // Nor where a plain field filled in before the path would send it.
    let moved = json!({ "id": id, "fields": { "client_id": { "value": "evil.example.com/?" } } });
    assert_eq!(v.ask(&token, "POST", "/items", json!({ "scope": me, "item": moved })).0, 200);
    let (status, refused) = fetch("gmail", &format!("https://{{{{client_id}}}}@oauth2.googleapis.com/?k={{{{{id}:refresh_token}}}}"), "");
    assert_eq!(status, 400, "{refused}");

    // A sidecar named among an item's hosts gets its field filled into what it sends on.
    let key = json!({ "id": "00000000000000ff", "title": "Work browser", "hosts": ["agent-devices"], "apps": ["agent-devices"], "fields": { "key": { "generate": true } } });
    assert_eq!(v.ask(&token, "POST", "/items", json!({ "scope": me, "item": key })).0, 200);
    let (_, filled) = v.ask(&token, "POST", "/fill", json!({ "app": "agent-devices", "host": "agent-devices", "text": "{{00000000000000ff:key}}" }));
    assert_eq!(filled["text"].as_str().unwrap().len(), 43, "a generated key: {filled}");
    assert_eq!(v.ask(&token, "POST", "/fill", json!({ "app": "agent-devices", "host": "shell", "text": "{{00000000000000ff:key}}" })).0, 400);

    // Deleted, it fills nothing.
    assert_eq!(v.ask(&token, "POST", "/items", json!({ "scope": me, "item": { "id": id, "deleted": true } })).0, 200);
    let (_, gone) = fetch("gmail", "https://oauth2.googleapis.com/token", &format!("{{{{{id}:refresh_token}}}}"));
    let echoed: Value = serde_json::from_slice(&B64U.decode(gone["body64"].as_str().unwrap().as_bytes()).unwrap()).unwrap();
    assert_eq!(echoed["body"], format!("{{{{{id}:refresh_token}}}}"));
}

/// A browser's cookies and Origin, for asking the auth service as its screens do.
struct Browser {
    origin: String,
    cookies: std::collections::BTreeMap<String, String>,
}

impl Browser {
    fn send(&mut self, method: &str, path: &str, body: Option<Value>, form: Option<&[(&str, &str)]>) -> (u16, Value, Option<String>) {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).build().into();
        let url = if path.starts_with("http") { path.to_string() } else { format!("{}{path}", self.origin) };
        let cookie = self.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ");
        let mut resp = match (method, body, form) {
            ("GET", ..) => agent.get(&url).header("origin", &self.origin).header("cookie", &cookie).call().unwrap(),
            (_, _, Some(form)) => agent.post(&url).header("origin", &self.origin).send_form(form.iter().copied()).unwrap(),
            (_, body, _) => agent.post(&url).header("origin", &self.origin).header("cookie", &cookie).send_json(body.unwrap_or(json!({}))).unwrap(),
        };
        for set in resp.headers().get_all("set-cookie") {
            let pair = set.to_str().unwrap().split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.insert(name.trim().into(), value.into());
        }
        let location = resp.headers().get("location").map(|l| l.to_str().unwrap().to_string());
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::String(text)), location)
    }

    fn get(&mut self, path: &str) -> (u16, Value, Option<String>) {
        self.send("GET", path, None, None)
    }

    fn post(&mut self, path: &str, body: Value) -> (u16, Value) {
        let (status, out, _) = self.send("POST", path, Some(body), None);
        (status, out)
    }
}

/// A passkey kept in software, as an authenticator keeps one: an Ed25519 key, with attestation "none".
struct Passkey {
    key: ed25519_dalek::SigningKey,
    id: Vec<u8>,
    rp: String,
    origin: String,
}

fn cbor(v: &ciborium::Value) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(v, &mut out).unwrap();
    out
}

impl Passkey {
    fn client_data(&self, kind: &str, challenge: &str) -> Vec<u8> {
        json!({ "type": kind, "challenge": challenge, "origin": self.origin, "crossOrigin": false }).to_string().into_bytes()
    }

    fn rp_hash(&self) -> Vec<u8> {
        use sha2::Digest;
        sha2::Sha256::digest(self.rp.as_bytes()).to_vec()
    }

    /// Its PRF output, as a browser gets it at the install's address.
    fn prf(&self) -> String {
        B64U.encode(&self.id.repeat(2))
    }

    /// What `navigator.credentials.create` gives, as JSON, for registration options: only whether it has PRF, as the
    /// pages send it.
    fn create(&self, options: &Value) -> Value {
        self.create_with(options, true)
    }

    fn create_with(&self, options: &Value, prf: bool) -> Value {
        use ciborium::Value as C;
        let cose = C::Map(vec![(C::Integer(1.into()), C::Integer(1.into())), (C::Integer(3.into()), C::Integer((-8).into())), (C::Integer((-1).into()), C::Integer(6.into())), (C::Integer((-2).into()), C::Bytes(self.key.verifying_key().to_bytes().to_vec()))]);
        let mut auth = self.rp_hash();
        auth.push(0x45 | 0x18); // user present, verified, backed up, with a credential
        auth.extend(0u32.to_be_bytes());
        auth.extend([0; 16]);
        auth.extend((self.id.len() as u16).to_be_bytes());
        auth.extend(&self.id);
        auth.extend(cbor(&cose));
        let attestation = C::Map(vec![(C::Text("fmt".into()), C::Text("none".into())), (C::Text("attStmt".into()), C::Map(vec![])), (C::Text("authData".into()), C::Bytes(auth))]);
        let id = B64U.encode(&self.id);
        let client = self.client_data("webauthn.create", options["challenge"].as_str().unwrap());
        json!({ "id": id, "rawId": id, "type": "public-key", "clientExtensionResults": if prf { json!({ "prf": { "enabled": true } }) } else { json!({}) },
            "response": { "clientDataJSON": B64U.encode(&client), "attestationObject": B64U.encode(&cbor(&attestation)), "transports": ["internal"] } })
    }
}

/// The real kernel services, as an install runs them: people reach them at its address, the gateway's.
pub(crate) struct Kernel {
    _running: Running,
    pub(crate) address: String,
    vault: String,
    secret: String,
    /// The recovery key, as its holder keeps it.
    recovery_key: String,
    settings: kernel::Settings,
}

const RESOURCE: &str = "https://node.test/mcp";

impl Kernel {
    /// Runs them as `one kernel serve` does, from new settings, with the stand-in sites as the bucket, the gateway
    /// sending `/v1/` to `node_url` and serving a web client of two pages.
    pub(crate) fn start(node_url: Option<String>) -> Kernel {
        Kernel::start_as(node_url, false)
    }

    /// As `start`, for an install whose people sign in with Google at test.example, the stand-in sites as Google.
    fn google() -> Kernel {
        Kernel::start_as(None, true)
    }

    fn start_as(node_url: Option<String>, google: bool) -> Kernel {
        let gateway = port();
        let (mut settings, recovery) = kernel::Settings::new("Test", &format!("http://localhost:{gateway}"));
        settings.ports = [port(), port(), port(), gateway];
        settings.auth_url = format!("http://127.0.0.1:{}", settings.ports[0]);
        (settings.vault_url, settings.schedules_url) = (format!("http://127.0.0.1:{}", settings.ports[1]), format!("http://127.0.0.1:{}", settings.ports[2]));
        settings.resources = vec![RESOURCE.into()];
        settings.node_url = node_url;
        settings.bucket = Some(json!({ "endpoint": "https://s3.test", "bucket": "install", "region": "auto", "key_id": "id", "secret": "bucket secret" }));
        let modules = kernel::SERVICES.iter().map(|(_, entry, _)| module(entry)).collect();
        let sites = port();
        let web = crate::temp("web");
        std::fs::create_dir_all(web.join("screen")).unwrap();
        std::fs::write(web.join("index.html"), "<!doctype html><title>The web client</title>").unwrap();
        std::fs::write(web.join("frame.html"), "<!doctype html><title>A screen's frame</title>").unwrap();
        std::fs::write(web.join("screen/kit.css"), include_str!("../src/screen/kit.css")).unwrap();
        let mut served = kernel::served(&settings, modules, &web);
        served[1].outbound = Some("sites".into());
        if google {
            let vault_key = B64U.encode(&rand::random::<[u8; 32]>());
            served[0].outbound = Some("sites".into());
            served[0].bindings.extend([("SIGN_IN", "google"), ("JOIN_DOMAIN", "test.example"), ("GOOGLE_CLIENT_ID", "a client id"), ("GOOGLE_CLIENT_SECRET", "a client secret")].map(|(n, v)| (n.into(), v.into())));
            served[1].bindings.extend([("SIGN_IN".into(), "google".into()), ("VAULT_KEY".into(), vault_key)]);
        }
        served.push(Served { name: "sites".into(), module: module("test/sites.js"), class: None, listen: format!("127.0.0.1:{sites}"), bindings: vec![], assets: None, outbound: None });
        let running = run(served);
        Kernel {
            _running: running,
            address: settings.address.clone(),
            vault: settings.vault_url.clone(),
            secret: settings.kernel_secret.clone(),
            recovery_key: recovery,
            settings,
        }
    }

    fn browser(&self) -> Browser {
        Browser { origin: self.address.clone(), cookies: Default::default() }
    }

    /// A passkey for the install's address, as passkeys belong to it.
    fn passkey(&self) -> Passkey {
        let rp = self.address.split("://").nth(1).unwrap().split([':', '/']).next().unwrap().to_string();
        Passkey { key: ed25519_dalek::SigningKey::from_bytes(&rand::random()), id: rand::random::<[u8; 16]>().to_vec(), rp, origin: self.address.clone() }
    }
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Someone joins with an invite link for `name` (as `user`, if given): → their passkey and user id.
fn joins(k: &Kernel, browser: &mut Browser, name: &str, user: Option<&str>) -> (Passkey, String) {
    let invite = kernel::invite(&k.settings, name, user, || None).unwrap();
    joins_with(k, browser, invite.link.split_once('#').unwrap().1)
}

/// Opening an invite link (its secret, `token`): they make a passkey with PRF, which signs them in, and its PRF output
/// makes their keys, or opens those the link carries. → their passkey and user id.
fn joins_with(k: &Kernel, browser: &mut Browser, token: &str) -> (Passkey, String) {
    let (status, about) = k.browser().post("/invite/about", json!({ "token": token }));
    assert_eq!(status, 200, "{about}");
    let passkey = k.passkey();
    let (status, options, _) = browser.get(&format!("/api/auth/passkey/generate-register-options?context={}", enc(token)));
    assert_eq!(status, 200, "{options}");
    let (status, made) = browser.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options), "createSession": true }));
    assert_eq!(status, 200, "{made}");
    assert_eq!(made["user"]["name"], about["person"]);
    let user = made["user"]["id"].as_str().unwrap().to_string();
    let (status, keys) = browser.post("/invite/keys", json!({ "token": token, "prf": passkey.prf(), "credential": B64U.encode(&passkey.id) }));
    assert_eq!(status, 200, "{keys}");
    (passkey, user)
}

#[test]
fn someone_who_lost_every_passkey_joins_again_with_a_link_that_carries_their_keys() {
    let k = Kernel::start(None);
    let (lost, andre) = joins(&k, &mut k.browser(), "Andre", None);
    assert_eq!(opens(&k, &andre, &lost.prf()), 200);

    // He has keys, so his new link must carry them: only the recovery key opens them.
    let err = kernel::invite(&k.settings, "Andre", Some(&andre), || None).err().unwrap().to_string();
    assert!(err.contains("pipe the recovery key"), "{err}");
    let other = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>());
    assert!(kernel::invite(&k.settings, "Andre", Some(&andre), || Some(other)).is_err(), "another recovery key opens nothing");
    let invite = kernel::invite(&k.settings, "Andre", Some(&andre), || Some(k.recovery_key.clone())).unwrap();

    // Opening it on a new device, his new passkey opens what his lost one did.
    let mut browser = k.browser();
    let (passkey, user) = joins_with(&k, &mut browser, invite.link.split_once('#').unwrap().1);
    assert_eq!(user, andre);
    assert_eq!(opens(&k, &andre, &passkey.prf()), 200);

    // Someone new's link carries nothing, and needs no recovery key; their passkey opens only their own keys.
    let (sam, sam_id) = joins(&k, &mut k.browser(), "Sam", None);
    assert_ne!(sam_id, andre);
    assert_eq!(opens(&k, &sam_id, &sam.prf()), 200);
    assert_eq!(opens(&k, &andre, &sam.prf()), 403);
}

/// Signs in to a new client of theirs with their passkey, in a browser signed in at the address, as joining leaves it:
/// → the client's access token, whose session at the vault opens their keys.
fn signs_in(browser: &mut Browser, passkey: &Passkey) -> String {
    use sha2::Digest;
    let back = "http://localhost:9/signed-in";
    let (status, client) = browser.post("/api/auth/oauth2/register", json!({ "client_name": "Tester", "redirect_uris": [back], "token_endpoint_auth_method": "none" }));
    assert_eq!(status, 201, "{client}");
    let client = client["client_id"].as_str().unwrap().to_string();
    let verifier = B64U.encode(&rand::random::<[u8; 32]>());
    let challenge = B64U.encode(&sha2::Sha256::digest(verifier.as_bytes()));
    let (_, _, sign_in) = browser.get(&format!("/api/auth/oauth2/authorize?response_type=code&client_id={client}&redirect_uri={}&scope=openid&state=s&code_challenge={challenge}&code_challenge_method=S256", enc(back)));
    let query = sign_in.expect("authorizing goes to the sign-in page").split_once('?').unwrap().1.to_string();
    let (status, accepted) = browser.post("/sign-in/accept", json!({ "oauth_query": query, "prf": passkey.prf(), "credential": B64U.encode(&passkey.id) }));
    assert_eq!(status, 200, "{accepted}");
    let code = url_query(accepted["url"].as_str().unwrap())["code"].clone();
    let form = [("grant_type", "authorization_code"), ("code", code.as_str()), ("redirect_uri", back), ("client_id", client.as_str()), ("code_verifier", verifier.as_str())];
    let (status, tokens, _) = browser.send("POST", "/api/auth/oauth2/token", None, Some(&form));
    assert_eq!(status, 200, "{tokens}");
    tokens["access_token"].as_str().unwrap().to_string()
}

#[test]
fn someone_in_the_log_before_their_first_passkey_joins_after_kernel_keys() {
    const KIM: &str = "00000000000000e6";
    let k = Kernel::start(None);
    joins(&k, &mut k.browser(), "Andre", None);
    // An admin's kernel keys gives Kim's own scope, with the key the install's nodes made, before she has a passkey.
    let key = B64U.encode(&rand::random::<[u8; 32]>());
    let give = |people: Value| post(&format!("{}/internal/scope-key", k.vault), &[("x-kernel-secret", &k.secret)], json!({ "scope": format!("user:{KIM}"), "key": key, "people": people }));
    assert_eq!(give(json!([KIM])).0, 200);
    // She joins all the same; the next kernel keys opens her own to her, with that key.
    let mut kims = k.browser();
    let (passkey, kim) = joins(&k, &mut kims, "Kim", Some(KIM));
    assert_eq!(kim, KIM);
    let token = signs_in(&mut kims, &passkey);
    let vault = kernel::Vault { url: k.vault.clone() };
    let mine = format!("user:{KIM}");
    assert!(vault.keys_until(&token, &[(&mine, "board", None)]).is_err());
    assert_eq!(give(json!([KIM])).0, 200);
    assert!(vault.keys_until(&token, &[(&mine, "board", None)]).is_ok());
}

#[test]
fn an_admin_signed_in_makes_an_invite_link_that_works_once() {
    let k = Kernel::start(None);
    let (mut andres, mut sams) = (k.browser(), k.browser());
    let ((andre_key, andre), (sam_key, sam)) = (joins(&k, &mut andres, "Andre", None), joins(&k, &mut sams, "Sam", None));
    // The install's log says Andre is its admin and Sam a member, as an admin's kernel keys tells the vault.
    let everyone = json!({ "scope": "global", "key": B64U.encode(&rand::random::<[u8; 32]>()), "people": [andre, sam], "admins": [andre] });
    let (status, out) = post(&format!("{}/internal/scope-key", k.vault), &[("x-kernel-secret", &k.secret)], everyone);
    assert_eq!(status, 200, "{out}");
    let vault = kernel::Vault { url: k.vault.clone() };
    let (andres, sams) = (signs_in(&mut andres, &andre_key), signs_in(&mut sams, &sam_key));

    let err = vault.invite(&sams, "Kim").err().expect("a member makes no invite link").to_string();
    assert!(err.contains("only an admin"), "{err}");
    let (secret, until) = vault.invite(&andres, "Kim").unwrap();
    assert!(until > now() + 2 * 86_400_000, "it works for days");
    let mut kims = k.browser();
    let (kim_key, kim) = joins_with(&k, &mut kims, &secret);
    assert!(kim != andre && kim != sam, "it's for someone new");
    // She opens Everyone's as soon as she's joined, with no kernel keys after: Andre's key, sealed into the link.
    let kims = signs_in(&mut kims, &kim_key);
    let everyone = |token: &str| vault.keys_until(token, &[("global", "board", None)]).map(|k| k.0[0].key).unwrap();
    assert_eq!(everyone(&kims), everyone(&andres));
    let (status, used) = k.browser().post("/invite/about", json!({ "token": secret }));
    assert_eq!(status, 404, "a used link lets nobody else in: {used}");
}

/// Whether a passkey's PRF output opens `user`'s keys at the vault, as signing in to a client asks: → its status.
fn opens(k: &Kernel, user: &str, prf: &str) -> u16 {
    post(&format!("{}/internal/grant", k.vault), &[("x-kernel-secret", &k.secret)], json!({ "user": user, "from": prf, "to": "a code that no client is given" })).0
}

/// Asks `url` as a browser's script would: → (status, headers, body).
fn ask(method: &str, url: &str, headers: &[(&str, &str)], body: Option<Value>) -> (u16, std::collections::BTreeMap<String, String>, String) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).build().into();
    let mut resp = match body {
        None => {
            let mut req = agent.request_builder_get(method, url);
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            req.call().unwrap()
        }
        Some(body) => {
            let mut req = agent.post(url);
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            req.send_json(body).unwrap()
        }
    };
    let got = resp.headers().iter().map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())).collect();
    (resp.status().as_u16(), got, resp.body_mut().read_to_string().unwrap_or_default())
}

trait GetWith {
    fn request_builder_get(&self, method: &str, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody>;
}

impl GetWith for ureq::Agent {
    fn request_builder_get(&self, method: &str, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        match method {
            "HEAD" => self.head(url),
            _ => self.get(url),
        }
    }
}

/// A stand-in node that answers every request with what it was sent, as a page that would run script, sets a cookie
/// and may be framed: → its address.
fn hostile_node() -> String {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            // Each connection on its own, and one that closes or resets before it's answered is let go: workerd may
            // drop one it opened.
            std::thread::spawn(move || -> std::io::Result<()> {
                let mut reader = BufReader::new(stream.try_clone()?);
                let mut line = String::new();
                reader.read_line(&mut line)?;
                let mut headers = serde_json::Map::new();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.trim().split_once(':') {
                        headers.insert(k.trim().to_lowercase(), json!(v.trim()));
                    }
                }
                let body = json!({ "request": line.trim(), "headers": headers }).to_string();
                write!(
                    &stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\nset-cookie: better-auth.session_token=planted; Path=/\r\naccess-control-allow-origin: *\r\nservice-worker-allowed: /\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
            });
        }
    });
    addr
}

#[test]
fn the_gateway_sends_each_path_where_it_belongs_and_nothing_a_node_says_runs_at_its_origin() {
    let node = hostile_node();
    let k = Kernel::start(Some(node));
    let at = |path: &str| format!("{}{path}", k.address);

    // The web client's files: its page at / and wherever it routes itself, the screens' kit, the frame screens load.
    for path in ["/", "/callback", "/vault?id=1&scope=user"] {
        let (status, headers, body) = ask("GET", &at(path), &[], None);
        assert_eq!((status, headers["content-type"].as_str()), (200, "text/html; charset=utf-8"), "{path}");
        assert!(body.contains("The web client"), "{path}: {body}");
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["x-frame-options"], "SAMEORIGIN", "no other site frames the web client");
    }
    let (status, headers, body) = ask("GET", &at("/frame.html"), &[], None);
    assert!(status == 200 && body.contains("A screen's frame"));
    assert!(!headers.contains_key("content-security-policy"), "a screen's frame carries its own policy: {headers:?}");
    assert_eq!(headers["x-frame-options"], "SAMEORIGIN", "only the web client's own page frames it");
    let (status, headers, _) = ask("GET", &at("/screen/kit.css"), &[], None);
    assert_eq!((status, headers["content-type"].as_str()), (200, "text/css; charset=utf-8"));
    assert_eq!(ask("GET", &at("/nothing.js"), &[], None).0, 404, "a missing file isn't the page");

    // Sign-in goes to the auth service, as the install's address; its own routes don't.
    let (status, _, meta) = ask("GET", &at("/.well-known/oauth-authorization-server/api/auth"), &[], None);
    assert_eq!(status, 200, "{meta}");
    assert_eq!(serde_json::from_str::<Value>(&meta).unwrap()["issuer"], at("/api/auth"));
    for (path, says) in [("/sign-in", "Sign In with a Passkey"), ("/invite", "Make a Passkey"), ("/passkeys", "Add a Passkey")] {
        let (status, _, body) = ask("GET", &at(path), &[], None);
        assert!(status == 200 && body.contains(says), "{path}: {status}");
    }
    let (status, _, _) = ask("POST", &at("/internal/invite"), &[], Some(json!({ "name": "Mallory" })));
    assert_eq!(status, 405, "the kernel's own routes aren't reachable at the address");

    // A node gets the token, never the browser's cookies, and what it says is stamped so it can't run here.
    let (status, headers, body) = ask("GET", &at("/v1/screens/board/main?scope=user"), &[("authorization", "Bearer a token"), ("cookie", "better-auth.session_token=mine")], None);
    assert_eq!(status, 200);
    let seen: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(seen["request"], "GET /v1/screens/board/main?scope=user HTTP/1.1");
    assert_eq!(seen["headers"]["authorization"], "Bearer a token");
    assert!(seen["headers"].get("cookie").is_none(), "{seen}");
    assert!(seen["headers"]["forwarded"].as_str().is_some_and(|f| f.starts_with("host=")), "a node knows it came from elsewhere: {seen}");
    assert_eq!(headers["content-security-policy"], "sandbox allow-scripts allow-forms allow-popups allow-downloads");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    for header in ["set-cookie", "access-control-allow-origin", "service-worker-allowed"] {
        assert!(!headers.contains_key(header), "a node's {header} reached the browser: {headers:?}");
    }
    assert_eq!(ask("POST", &at("/v1/call"), &[], Some(json!({}))).0, 200);
    assert_eq!(ask("GET", &at("/mcp"), &[], None).0, 200, "/mcp is a node's");
    assert_eq!(ask("GET", &at("/v1/state"), &[("service-worker", "script")], None).0, 403, "no node's answer becomes a service worker");
}

/// Settings' screens work for someone signed in, through the gateway, as they do on that person's own Mac: each route
/// acts as them and is checked as theirs, so a member can't do what only admins may, and nobody acts as anyone else.
#[test]
fn a_sign_in_uses_settings_as_its_person_and_only_with_their_rights() {
    let v = Vault::start();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = crate::temp("node");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": "node" }).to_string()).unwrap();
    let (node, api) = rt.block_on(async {
        let node = one::node::Node::start(&dir, one::sidecars::built_in()).await.unwrap();
        node.create("Test", "Andre").await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let app = one::api::router(node.clone());
        tokio::spawn(one::api::serve(listener, app));
        (node, api)
    });
    // Andre made the install; Sam is a member. The device is the install's, so both use it; it takes sign-ins made
    // at the install's address.
    let (andre, sam) = (node.user().unwrap(), one::entry::new_id());
    rt.block_on(async {
        node.member("admit", json!({ "key": one::entry::hex(&rand::random()), "user": sam, "as": "client", "name": "Sam's phone", "user_name": "Sam", "role": "member" })).await.unwrap();
        node.member("acl", json!({ "device": one::entry::hex(&node.id()), "owners": ["install"], "users": ["install"] })).await.unwrap();
        node.member("setting", json!({ "name": "address", "value": v.auth })).await.unwrap();
        node.member("setting", json!({ "name": "kernel_vault", "value": v.url })).await.unwrap();
    });
    v.internal("passkey", json!({ "user": andre, "secret": "andre's passkey's prf output" }));
    v.internal("passkey", json!({ "user": sam, "secret": "sam's passkey's prf output" }));
    let (andres, sams) = (v.client(&andre, "andre's passkey's prf output", "web"), v.client(&sam, "sam's passkey's prf output", "web"));
    let ask = |token: &str, method: &str, path: &str, body: Value| ask_at(&api, token, method, path, body);
    let state = |token: &str| ask(token, "GET", "/v1/state", Value::Null).1;
    let items = |token: &str| state(token)["settings_items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let refused = |(status, body): (u16, Value)| status == 400 && body["error"].as_str().is_some_and(|e| !e.is_empty());

    // Settings shows each their own: Sam first, as himself; only admins see Users, General and Backups.
    assert_eq!(state(&sams)["settings_items"][0], json!({ "name": "account", "title": "Sam", "icon": "user-circle", "color": "#8e8e93" }));
    assert!(!items(&sams).iter().any(|i| ["users", "general", "backups"].contains(&i.as_str())), "{:?}", items(&sams));
    assert!(["users", "general", "backups"].iter().all(|i| items(&andres).iter().any(|n| n == i)), "{:?}", items(&andres));
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let page = |path: &str| agent.get(format!("{api}{path}")).header("authorization", format!("Bearer {sams}")).call().unwrap().body_mut().read_to_string().unwrap();
    let vault = page("/v1/client-screens/vault");
    assert!(vault.contains(&format!("\"user\":\"{sam}\"")), "the page is Sam's: {vault}");
    assert!(page("/v1/client-screens/nothing").contains("one.notFound()"));

    // What only admins may do, Sam can't; Andre can, signed in as on his Mac.
    let setting = json!({ "name": "made_here", "value": "Ours" });
    assert!(refused(ask(&sams, "POST", "/v1/settings", setting.clone())), "a member changes no install setting");
    assert!(refused(ask(&sams, "POST", "/v1/role", json!({ "user": sam, "role": "admin" }))), "nor makes himself an admin");
    assert!(refused(ask(&sams, "POST", "/v1/remove", json!({ "user": andre }))), "nor removes anyone");
    assert!(refused(ask(&sams, "POST", "/v1/backups/place", json!({ "path": dir.join("backups"), "title": "Disk" }))), "nor adds backups");
    assert!(refused(ask(&sams, "POST", "/v1/pair", json!({ "user": andre }))), "nor pairs a machine as someone else");
    assert_eq!(ask(&andres, "POST", "/v1/settings", setting).0, 200);
    assert_eq!(state(&andres)["install"]["settings"]["made_here"], "Ours");

    // A group Sam makes is his, not this device's person's; its members change only as its admins and invitees say.
    let (status, made) = ask(&sams, "POST", "/v1/group", json!({ "name": "Sam's" }));
    assert_eq!(status, 200, "{made}");
    let group = made["group"].as_str().unwrap().to_string();
    let members = || state(&sams)["install"]["groups"].as_array().unwrap().iter().find(|g| g["id"] == group.as_str()).unwrap()["members"].clone();
    assert_eq!(members(), json!([{ "user": sam, "role": "admin" }]));
    assert!(refused(ask(&sams, "POST", "/v1/accept", json!({ "group": group }))), "nobody accepts an invite they weren't sent");
    assert_eq!(ask(&sams, "POST", "/v1/invite", json!({ "group": group, "user": andre })).0, 200);
    assert!(state(&andres)["settings_items"].as_array().unwrap().iter().any(|i| i["name"] == "requests" && i["badge"] == 1), "his invite waits in Requests");
    assert_eq!(ask(&andres, "POST", "/v1/accept", json!({ "group": group })).0, 200);
    assert_eq!(members().as_array().unwrap().len(), 2);

    // What only this machine's own programs do, no sign-in from elsewhere (through the gateway) does: renaming the
    // device, say.
    let renamed = agent.post(format!("{api}/v1/name")).header("authorization", format!("Bearer {andres}")).header("forwarded", "host=\"one.example\"").send_json(json!({ "name": "elsewhere" })).unwrap();
    assert_eq!(renamed.status(), 403);
    // A sign-in is no device: not the device it reached, so nothing offers it that device's name to change.
    let seen = state(&andres);
    assert_eq!(seen["me"]["key"], "", "{}", seen["me"]);
    assert!(seen["install"]["devices"].as_array().unwrap().iter().all(|s| s["me"] == false), "{}", seen["install"]["devices"]);
    assert!(seen.get("client").is_none());

    // A file is read only by whoever put it or may use a scope that names it: knowing its hash isn't enough. Only
    // this machine's own programs put files, a sign-in's MCP answers among them.
    let get = |token: &str, hash: &str| agent.get(format!("{api}/v1/blobs/{hash}")).header("authorization", format!("Bearer {token}")).header("forwarded", "host=\"one.example\"").call().unwrap().status().as_u16();
    let theirs = node.put_blob(b"an attachment of someone else's").unwrap();
    assert_eq!(get(&sams, &theirs), 400, "a sign-in reads a file nobody gave it");
    let put = |token: &str, forwarded: bool| {
        let req = agent.post(format!("{api}/v1/blobs")).header("authorization", format!("Bearer {token}"));
        let mut resp = if forwarded { req.header("forwarded", "host=\"one.example\"") } else { req.header(one::api::LOCAL_TOKEN_HEADER, node.local_token()) }.send(&b"Sam's file"[..]).unwrap();
        (resp.status().as_u16(), resp.body_mut().read_json::<Value>().ok().and_then(|b| b["blob"].as_str().map(str::to_string)))
    };
    assert_eq!(put(&sams, true).0, 403, "a sign-in from elsewhere puts no files here");
    let (status, sams_file) = put(&sams, false);
    assert_eq!(status, 200);
    let sams_file = sams_file.unwrap();
    assert_eq!(get(&sams, &sams_file), 200, "Sam reads back what he put");
    assert_eq!(get(&andres, &sams_file), 400, "and Andre doesn't, by its hash");

    // Removing someone takes their keys at the vault as soon as the node writes it, with no kernel keys after.
    let key = || B64U.encode(&rand::random::<[u8; 32]>());
    v.internal("scope-key", json!({ "scope": "global", "key": key(), "people": [andre, sam], "admins": [andre] }));
    v.internal("scope-key", json!({ "scope": format!("group:{group}"), "key": key(), "people": [andre, sam], "admins": [sam] }));
    let vault = kernel::Vault { url: v.url.clone() };
    let opens = |token: &str, scope: &str| vault.keys_until(token, &[(scope, "board", None)]).is_ok();
    assert!(opens(&andres, &format!("group:{group}")) && opens(&sams, "global"));
    assert_eq!(ask(&sams, "POST", "/v1/member", json!({ "group": group, "user": andre, "role": null })).0, 200);
    assert!(!opens(&andres, &format!("group:{group}")) && opens(&andres, "global"));
    // Letting someone in does too. Andre, the install's admin but no longer in it, can't: the invite is made, and
    // he's told why he won't open it, on the screen he asked from. Sam invites him back as an admin, and he opens it
    // once he accepts.
    let (status, said) = ask(&andres, "POST", "/v1/invite", json!({ "group": group, "user": andre }));
    assert_eq!(status, 400);
    assert!(said["error"].as_str().unwrap().starts_with("You are invited to Sam's, but won't open what's in it"), "{said}");
    assert_eq!(ask(&sams, "POST", "/v1/invite", json!({ "group": group, "user": andre, "role": "admin" })).0, 200);
    assert!(!opens(&andres, &format!("group:{group}")));
    assert_eq!(ask(&andres, "POST", "/v1/accept", json!({ "group": group })).0, 200);
    assert!(opens(&andres, &format!("group:{group}")));
    assert_eq!(ask(&andres, "POST", "/v1/remove", json!({ "user": sam })).0, 200);
    assert!(!opens(&sams, "global") && !opens(&sams, &format!("group:{group}")) && opens(&andres, "global"));
    // An admin who makes himself a member is one at the vault no more, at once.
    assert_eq!(ask(&andres, "POST", "/v1/role", json!({ "user": andre, "role": "member" })).0, 200);
    let err = vault.invite(&andres, "Kim").err().expect("he's no admin there").to_string();
    assert!(err.contains("only an admin"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A client names itself, so the sign-in page also says where it goes, and no client takes the install's name, nor its
/// apps' unless it comes back to the device as they do: one passkey tap gives a client the person's keys.
#[test]
fn a_client_is_shown_by_where_it_goes_and_none_is_named_as_the_install() {
    let k = Kernel::start(None);
    let register = |name: &str, back: &str| post(&format!("{}/api/auth/oauth2/register", k.address), &[], json!({ "client_name": name, "redirect_uris": [back], "token_endpoint_auth_method": "none", "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"] }));
    for (name, back) in [("Test", "https://elsewhere.example/back"), (" the TEST ", "http://localhost:4000/back"), ("Test on Andre's Mac", "https://elsewhere.example/back")] {
        let (status, out) = register(name, back);
        assert_eq!(status, 400, "{name} going to {back} was taken: {out}");
    }
    let goes = |name: &str, back: &str| {
        let (status, out) = register(name, back);
        assert_eq!(status, 201, "{out}");
        let (_, about, _) = k.browser().get(&format!("/sign-in/about?client_id={}&redirect_uri={}", out["client_id"].as_str().unwrap(), enc(back)));
        (about["client"].as_str().unwrap().to_string(), about["goes"].clone())
    };
    assert_eq!(goes("Claude", "https://claude.ai/api/mcp/auth_callback"), ("Claude".into(), json!("claude.ai")));
    assert_eq!(goes("Claude Code", "http://localhost:33418/callback").1, "an app on this device");
    assert_eq!(goes("Test on Andre's iPhone", "one://signed-in/").1, "an app on this device");
    assert_eq!(goes("Test on a device", &format!("{}/sign-in/code/done", k.address)).1, "where you got your code");
}

/// A client registers itself (anyone may) and its authorization goes to the sign-in page: → the page's query.
fn authorizes(browser: &mut Browser, name: &str, back: &str) -> String {
    let (status, client) = browser.post("/api/auth/oauth2/register", json!({ "client_name": name, "redirect_uris": [back], "token_endpoint_auth_method": "none" }));
    assert_eq!(status, 201, "{client}");
    let client = client["client_id"].as_str().unwrap();
    let (_, _, sign_in) = browser.get(&format!("/api/auth/oauth2/authorize?response_type=code&client_id={client}&redirect_uri={}&scope=openid&state=s&code_challenge=c&code_challenge_method=S256", enc(back)));
    sign_in.expect("authorizing goes to the sign-in page").split_once('?').unwrap().1.to_string()
}

/// Andre signs in with Google in `browser`, coming back to the sign-in page for `query`, as its button does, which
/// starts it there first unless `started` is false.
fn signs_in_with_google(browser: &mut Browser, query: &str, started: bool) {
    if started {
        let (status, out) = browser.post("/sign-in/google", json!({ "oauth_query": query }));
        assert_eq!(status, 200, "{out}");
    }
    let back = format!("/sign-in?{query}");
    let (status, out) = browser.post("/api/auth/sign-in/social", json!({ "provider": "google", "callbackURL": back, "errorCallbackURL": back }));
    assert_eq!(status, 200, "{out}");
    let state = url_query(out["url"].as_str().unwrap())["state"].clone();
    let andre = json!({ "iss": "https://accounts.google.com", "sub": "1234", "email": "andre@test.example", "email_verified": true, "hd": "test.example", "name": "Andre" });
    let (status, _, to) = browser.get(&format!("/api/auth/callback/google?code={}&state={state}", B64U.encode(andre.to_string().as_bytes())));
    assert_eq!((status, to.as_deref()), (302, Some(back.as_str())));
}

/// Signing in to a client with Google goes straight on, with no second tap, only for the authorization the sign-in
/// was started for, in that browser: a link to someone else's client, opened just after signing in to another, waits
/// for the person.
#[test]
fn a_google_sign_in_goes_on_only_to_the_client_it_was_started_for() {
    let k = Kernel::google();
    let mut andres = k.browser();
    let fresh = |b: &mut Browser, query: &str| b.get(&format!("/sign-in/about?{query}")).1["fresh"].clone();
    let accept = |b: &mut Browser, query: &str| b.post("/sign-in/accept", json!({ "oauth_query": query })).0;
    let ours = authorizes(&mut andres, "Andre's", "http://localhost:9/signed-in");
    signs_in_with_google(&mut andres, &ours, true);
    assert_eq!(fresh(&mut andres, &ours), true);
    assert_eq!(accept(&mut andres, &ours), 200);
    assert_eq!(accept(&mut andres, &ours), 403, "it goes on once");

    // Someone's client, whose link he opens a minute later: signed in with Google just now, all the same.
    let theirs = authorizes(&mut k.browser(), "Someone's", "https://elsewhere.example/back");
    assert_eq!(fresh(&mut andres, &theirs), false);
    assert_eq!(accept(&mut andres, &theirs), 403);
    // Nor once its sender starts a Google sign-in for it in their own browser, nor a page elsewhere in his.
    let mut theirs_started = k.browser();
    assert_eq!(theirs_started.post("/sign-in/google", json!({ "oauth_query": theirs })).0, 200);
    let mut elsewhere = Browser { origin: "https://elsewhere.example".into(), cookies: andres.cookies.clone() };
    let (status, out) = elsewhere.post(&format!("{}/sign-in/google", k.address), json!({ "oauth_query": theirs }));
    assert_eq!(status, 403, "{out}");
    assert_eq!(accept(&mut andres, &theirs), 403);
    // Nor signing in with Google again not started for it.
    signs_in_with_google(&mut andres, &theirs, false);
    assert_eq!(accept(&mut andres, &theirs), 403);
    // Tapping Sign In with Google on its page, he means it.
    signs_in_with_google(&mut andres, &theirs, true);
    assert_eq!(accept(&mut andres, &theirs), 200);
}

#[test]
fn only_a_try_installs_kernel_signs_its_person_in_with_nothing_to_show() {
    let k = Kernel::start(None);
    let (status, out) = k.browser().post("/api/auth/try/sign-in", json!({}));
    assert_eq!(status, 404, "a kernel people use signed someone in with no passkey: {out}");
    let (status, out) = k.browser().post("/api/auth/try/sign-in", json!({ "person": "Andre" }));
    assert_eq!(status, 404, "a kernel people use signed someone in by name: {out}");
    // Nor does an invite link join anyone there without a passkey.
    let invite = kernel::invite(&k.settings, "Sam", None, || None).unwrap();
    let (status, out) = k.browser().post("/api/auth/try/join", json!({ "token": invite.link.split_once('#').unwrap().1 }));
    assert_eq!(status, 404, "a kernel people use let someone join with no passkey: {out}");
    let (status, _) = post(&format!("{}/internal/own", k.vault), &[("x-kernel-secret", &k.secret)], json!({ "user": "0123456789abcdef", "to": "a code", "client": "web" }));
    assert_eq!(status, 403, "the vault of a kernel people use opened someone's keys itself");
    // Only `one try` makes such settings, and nothing puts them where anyone but this machine reaches them.
    let mut tried = k.settings.clone();
    tried.try_person = Some(("0123456789abcdef".into(), "Andre".into()));
    let cf = kernel::Cloudflare { api: "http://127.0.0.1:9".into(), token: "a token".into(), account: "an account".into() };
    for refused in [kernel::deploy(&cf, &tried, &[], &[]), one::serverless::deploy(&cf, &tried), kernel::serve(&tried)] {
        assert!(refused.unwrap_err().to_string().contains("try install"));
    }
}

#[test]
fn signing_out_ends_a_clients_sessions_at_the_vault() {
    let v = Vault::start();
    let vault = kernel::Vault { url: v.url.clone() };
    v.internal("passkey", json!({ "user": ANDRE, "secret": "andre's passkey prf output" }));
    let laptop = v.client(ANDRE, "andre's passkey prf output", "web");
    // Another browser of Andre's: the same client to auth, signed in on its own.
    let code = "the phone's authorization code";
    v.internal("grant", json!({ "user": ANDRE, "from": "andre's passkey prf output", "to": code, "client": "web" }));
    let (phone, until) = v.token(ANDRE, "web");
    v.internal("rekey", json!({ "from": code, "to": "the phone's refresh token", "token": phone, "until": until }));
    // The laptop refreshes: its old access token's session is still its own.
    let (fresh, until) = v.token(ANDRE, "web");
    assert_eq!(v.internal("rekey", json!({ "from": format!("web's refresh token for {ANDRE}"), "to": "the laptop's next refresh token", "token": fresh, "until": until })).1["moved"], true);
    let board = [("global", "board", None)];
    assert!(vault.keys_until(&laptop, &board).is_ok() && vault.keys_until(&fresh, &board).is_ok());

    // The laptop signs out: auth revokes its refresh token, and no access token it was given gets keys again.
    assert_eq!(v.internal("forget", json!({ "secret": "the laptop's next refresh token" })).0, 200);
    for token in [&laptop, &fresh] {
        assert!(vault.keys_until(token, &board).is_err(), "a signed-out client's session still gets keys");
        assert_eq!(v.ask(token, "GET", "/session", json!(null)).0, 403);
    }
    assert!(vault.keys_until(&phone, &board).is_ok(), "signing out one browser signs out another");

    // Removing the phone from Andre's clients ends its session too.
    let (_, copies) = v.ask(&phone, "GET", "/copies", json!(null));
    let id = copies["copies"].as_array().unwrap().iter().find(|c| c["kind"] == "client").unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(v.ask(&phone, "DELETE", &format!("/copies/{id}"), json!(null)).0, 200);
    assert!(vault.keys_until(&phone, &board).is_err(), "a removed client's session still gets keys");
}

#[test]
fn the_address_serves_only_scripts_that_parse() {
    let (settings, _) = kernel::Settings::new("Test", "https://one.example.com");
    kernel::parses(&settings.web()).unwrap();
    let broken = vec![("web.js".to_string(), b"const label = 1;\nconst label = 2;\n".to_vec())];
    assert!(kernel::parses(&broken).is_err(), "a script that doesn't parse is refused");
    let inline = vec![("index.html".to_string(), b"<p>hi</p><script>let a = ;</script>".to_vec())];
    assert!(kernel::parses(&inline).is_err(), "so is an HTML page's inline script");
}

/// The serverless node is the node's Rust as WebAssembly: what doesn't build for it, `one update` can't send out.
/// Checked in a target folder of its own, kept between runs, so this test's cargo never waits on the one running it.
#[test]
fn the_serverless_node_builds_for_webassembly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["check", "-q", "-p", "one-serverless", "--target", "wasm32-unknown-unknown", "--target-dir"])
        .arg(root.join("target/serverless-check"))
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success(), "the serverless node doesn't build (it needs rustup target add wasm32-unknown-unknown, and clang):\n{}", String::from_utf8_lossy(&out.stderr));
}
