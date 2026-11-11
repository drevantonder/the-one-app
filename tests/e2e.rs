//! Real nodes on this machine, reaching each other through the relay at a real network's gateway.

use std::time::Duration;

use one::node::Node;
use one::protocol::CallRequest;
use serde_json::{Value, json};

use crate::temp;

pub(crate) async fn start(name: &str) -> Node {
    let dir = temp(name);
    std::fs::write(dir.join("device.json"), json!({ "secret": one::keys::hex(&rand::random()), "name": name }).to_string()).unwrap();
    Node::start(&dir).await.unwrap()
}

/// A new network on `a`: its recovery key's secret.
pub(crate) async fn create(a: &Node, name: &str, user_name: &str) -> String {
    a.create_with(name, user_name, None, one::config::STARTERS).await.unwrap()
}

/// Workers whose relay the devices of `a`'s network reach each other through, taking only `a` and those it
/// admits: `a` says it's the network's.
async fn relay(a: &Node) -> crate::cloudflare::Workers {
    let k = tokio::task::block_in_place(|| crate::cloudflare::Workers::for_devices(vec![one::keys::hex(&a.id())]));
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    a.set_setting(&me, "address", Some(&k.address)).await.unwrap();
    k
}

/// Waits for what's expected of `node`, checking again at each event it announces: a limit only for what never happens.
pub(crate) async fn eventually(what: &str, node: &Node, mut check: impl AsyncFnMut() -> bool) {
    let mut events = node.subscribe();
    let waiting = async {
        while !check().await {
            let _ = events.recv().await;
        }
    };
    if tokio::time::timeout(Duration::from_secs(180), waiting).await.is_err() {
        panic!("timed out waiting for {what}");
    }
}

/// `content` written to `global`'s scope database as a step `caller`'s own device (`key`) signs, as a client does.
pub(crate) async fn signed_by(a: &Node, caller: &one::node::Caller, key: &ed25519_dalek::SigningKey, content: one::database::content::Content) {
    let global = one::database::scopes::Scope::Global;
    let next = a.global_steps(caller, one::database::steps::NUMBER_MAX, None).await.unwrap();
    let (at, epoch) = (next["at"].as_u64().unwrap(), next["epoch"].as_u64().unwrap() as u32);
    let step = one::database::steps::sign(key, &global, at + 1, &caller.user, epoch, one::node::now_ms(), content).unwrap();
    a.put_signed_steps(caller, vec![step]).await.unwrap();
}

/// A request to join from the device of `key`, as auth lists it, for `user`, with the proof `secret`'s invite makes.
fn asks(key: &ed25519_dalek::SigningKey, user: &str, secret: Option<&[u8]>) -> one::settings::Asked {
    let e = one::keys::Endpoint::of(key);
    let proof = secret.map(|s| one::keys::invite_proof(s, &e));
    one::settings::Asked::of(&json!({ "key": one::keys::hex(&e.key), "seal": one::keys::hex(&e.seal), "name": "Sam's laptop", "user": user, "proof": proof })).unwrap()
}

/// An invite for Sam, as a member, whose secret is `secret`.
fn invited(secret: &str) -> one::settings::Invited {
    serde_json::from_value(json!({ "secret": secret, "name": "Sam", "role": "member", "until": u64::MAX })).unwrap()
}

/// Sam, invited by `a`'s user as a member, let in with his first device, `key`, as `a` approves it: → his id.
pub(crate) async fn let_sam_in(a: &Node, key: &ed25519_dalek::SigningKey) -> String {
    let sam = one::database::scopes::new_id();
    one::settings::approve(a, &asks(key, &sam, Some(b"the-invites-secret")), &one::settings::Joining::Link(invited("the-invites-secret"))).await.unwrap();
    sam
}

fn notes(tool: &str, input: Value) -> CallRequest {
    CallRequest { app: "notes".into(), tool: tool.into(), input, ..Default::default() }
}

/// Deploys the tests' own app, `tests/notes/app.js` (lines of text in `global`'s database), for everyone, as an admin
/// would add an app, and waits for this device to run it.
async fn add_notes(a: &Node) {
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    let bundle = a.put_blob(&one::app::bundle::pack(&[("app.js".to_string(), include_bytes!("notes/app.js").to_vec())]).unwrap()).unwrap();
    a.deploy_making(&me, "notes", &bundle, Some("admins"), true).await.unwrap();
    a.add(&me, "notes", &one::database::scopes::Scope::Global, true, None).await.unwrap();
}

/// The command that opens a backup, on a machine with only the disk and the recovery key: nothing running or joined
/// here, and whatever it asked a node at would refuse the connection. The key comes from a file or stdin, and is never
/// printed.
fn the_command_checks_it_with_no_node(backup: &std::path::Path, recovery: &str) {
    use std::io::Write;
    let nothing = temp("no-node");
    let file = nothing.join("recovery.key");
    std::fs::write(&file, format!("{recovery}\n")).unwrap();
    for (key, stdin) in [(file.to_str().unwrap(), None), ("-", Some(recovery))] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_one"));
        command.args(["backups", "check", "--json", "--folder", backup.to_str().unwrap(), "--recovery-key", key]);
        command.env(one::config::env("HOME"), &nothing).env(one::config::env("ADDR"), "127.0.0.1:1").env("HOME", &nothing);
        let mut child = command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        let mut sent = child.stdin.take().unwrap();
        sent.write_all(stdin.unwrap_or("").as_bytes()).unwrap();
        drop(sent);
        let done = child.wait_with_output().unwrap();
        let (out, err) = (String::from_utf8_lossy(&done.stdout), String::from_utf8_lossy(&done.stderr));
        assert!(done.status.success(), "the check needs only the disk and the key: {err}");
        assert!(!out.contains(recovery) && !err.contains(recovery), "the key is never printed");
        let said: Value = serde_json::from_str(&out).unwrap();
        assert_eq!((&said[0]["network"], &said[0]["problems"]), (&json!("Home"), &json!([])), "{said}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn backups_go_to_a_folder_encrypted_and_restore_with_the_recovery_key_alone() {
    let a = start("backup-a").await;
    let b = start("backup-b").await;
    let recovery = create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    let _relay = relay(&a).await;
    b.join(&a.pair().unwrap()).await.unwrap();
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    eventually("notes to run", &a, async || a.call(notes("post", json!({ "text": "before the backup" }))).await.is_ok()).await;
    a.set_item(&me, &one::database::scopes::Scope::User(me.user.clone()), json!({ "title": "Example key", "fields": { "token": { "value": "s3cret-value", "secret": true } }, "hosts": ["api.example.com"] })).await.unwrap();

    let bad = a.add_place(&me, &json!({ "endpoint": "http://127.0.0.1:9", "bucket": "b", "key_id": "k", "secret": "s" })).await.unwrap();
    assert!(bad["id"].is_null() && bad["tried"][0]["error"].is_string(), "{bad}");
    assert!(a.backups().unwrap()["places"].as_array().unwrap().is_empty());

    let dir = temp("backup-disk").join("One Backups");
    let added = a.add_place(&me, &json!({ "path": dir, "title": "T7 Shield" })).await.unwrap();
    assert!(added["id"].is_string() && added["tried"].as_array().unwrap().iter().all(|s| s["error"].is_null()), "{added}");
    eventually("the first backup", &a, async || !a.backups().unwrap()["places"][0]["last"].is_null()).await;
    let shown = a.backups().unwrap();
    assert_eq!(shown["places"][0]["title"], "T7 Shield");
    assert_eq!(shown["places"][0]["last"]["device"], "backup-a", "the folder's own device backs it up: {shown}");
    let first = shown["places"][0]["last"]["time"].clone();
    eventually("b to see it", &b, async || !b.backups().unwrap()["places"][0]["last"].is_null()).await;

    // The folder's device restores it into a throwaway copy, against what it holds: who's who, databases and blobs.
    let report = a.check_backups().await.unwrap();
    assert!(report[0]["error"].is_null() && !report[0]["scopes"].as_array().unwrap().is_empty(), "{report}");
    assert!(a.backups().unwrap()["places"][0]["checked"]["error"].is_null(), "and says it passed");

    // From nothing but the disk and the recovery key.
    let place = one::bucket::Place::Folder(dir.clone());
    let network = one::backups::networks(&place).unwrap().remove(0);
    let keys = one::backups::open_keys(&place, &network, &one::keys::unhex(&recovery).unwrap()).unwrap();
    assert!(keys.contains_key("global") && keys.contains_key(&format!("user:{}", me.user)), "{:?}", keys.keys());
    let checked = one::backups::check(&one::backups::Check { place: &place, network: &network, keys: &keys, root: None }).unwrap().0;
    assert_eq!(checked["problems"], json!([]), "{checked}");
    assert_eq!(checked["network"], "Home");
    assert_eq!(checked["users"], json!(["Andre"]));
    assert!(checked["apps"].as_array().unwrap().contains(&json!("notes")), "{checked}");
    let mine = format!("user:{}", me.user);
    let items = backed_up(&dir, &network, &one::keys::unhex(&recovery).unwrap(), &mine, "node", "vault", "select item from items").await;
    assert!(items.iter().any(|i| i.contains("\"Example key\"")), "the Vault's item came back: {items:?}");
    the_command_checks_it_with_no_node(&dir, &recovery);
    let restored = one::backups::restore(&place, &network, "global", &keys["global"]).unwrap();
    assert_eq!(backed_up_posts(&dir, &network, &one::keys::unhex(&recovery).unwrap()).await, ["before the backup"]);
    let steps_then = restored.snapshot.databases["notes/main"].0;
    let files: Vec<Vec<u8>> = walk(&dir).iter().map(|f| std::fs::read(f).unwrap()).collect();
    assert!(!files.iter().any(|f| f.windows(12).any(|w| w == b"s3cret-value") || f.windows(17).any(|w| w == b"before the backup")), "nothing is readable on the disk");

    // A change is backed up on the next run; only what's new is sent.
    a.call(notes("post", json!({ "text": "after the backup" }))).await.unwrap();
    a.back_up_now();
    // A run is done once it says when it last backed up, not once its copy of the bucket has the post: it copies the
    // bucket before writing the snapshot that names the copy, and backs up every scope.
    eventually("the second backup", &a, async || a.backups().unwrap()["places"][0]["last"]["time"] != first).await;
    let again = one::backups::restore(&place, &network, "global", &keys["global"]).unwrap();
    assert!(again.snapshot.databases["notes/main"].0 > steps_then);
    assert_eq!(backed_up_posts(&dir, &network, &one::keys::unhex(&recovery).unwrap()).await, ["before the backup", "after the backup"]);
    let checked = a.check_backups().await.unwrap();
    assert_eq!(checked[0]["place"], "T7 Shield", "{checked}");
    assert!(checked[0]["error"].is_null(), "{checked}");
    let shown = a.backups().unwrap();
    let moment = &shown["snapshots"][0];
    assert!(moment["scopes"].as_array().unwrap().contains(&json!("Everyone")) && moment["places"] == json!(["T7 Shield"]), "one snapshot of the network: {shown}");
    assert!(a.state().unwrap().backups.is_none(), "no badge while backups work");

    // A disk that breaks: admins see what's wrong and its fix, on Settings, until a run works again.
    let copy = dir.join(&network).join("bucket");
    std::fs::rename(&copy, dir.join("bucket-aside")).unwrap();
    std::fs::write(&copy, b"not a folder").unwrap();
    // Its failure shows once it's done: nothing writes to the disk until the next run.
    a.call(notes("post", json!({ "text": "broken" }))).await.unwrap();
    a.back_up_now();
    eventually("a failed run", &a, async || a.backups().unwrap()["places"][0]["problem"]["text"].as_str().is_some_and(|t| t.contains("couldn't be used") || t.contains("won't let"))).await;
    let problem = a.state().unwrap().backups.unwrap();
    assert_eq!(problem["fix"], "Check the disk on backup-a is plugged in and has room.", "{problem}");
    assert!(problem["text"].as_str().unwrap().starts_with("T7 Shield "), "{problem}");
    std::fs::remove_file(&copy).unwrap();
    std::fs::rename(dir.join("bucket-aside"), &copy).unwrap();
    a.call(notes("post", json!({ "text": "fixed" }))).await.unwrap();
    a.back_up_now();
    eventually("the badge to clear", &a, async || a.state().unwrap().backups.is_none()).await;

    // A folder named only by a Vault item, which anyone may write, is never written to: a folder is its own device's
    // choice. Nor is an S3 place no admin's setting names.
    let global = one::database::scopes::Scope::Global;
    let planted = temp("backup-planted").join("home");
    let fields = json!({ "path": { "value": planted }, "device": { "value": one::keys::hex(&a.id()) } });
    a.set_item(&me, &global, json!({ "title": "Planted", "type": one::backups::KIND, "fields": fields, "apps": ["node"] })).await.unwrap();
    let s3 = json!({ "endpoint": { "value": "http://127.0.0.1:9" }, "bucket": { "value": "b" }, "key_id": { "value": "k" }, "secret": { "value": "s", "secret": true } });
    a.set_item(&me, &global, json!({ "title": "Unnamed bucket", "type": one::backups::KIND, "fields": s3, "hosts": ["127.0.0.1"], "apps": ["node"] })).await.unwrap();
    a.back_up_now();
    let place = |title: &str| a.backups().unwrap()["places"].as_array().unwrap().iter().find(|p| p["title"] == title).cloned();
    eventually("the planted folder's run to fail", &a, async || place("Planted").is_some_and(|p| !p["problem"].is_null())).await;
    assert!(!planted.exists(), "nothing was written there");
    assert!(place("Unnamed bucket").is_none(), "{}", a.backups().unwrap());

    // A place typed into the sheet on the user's device is a place too.
    let asked = a.request_place(&me, &json!({ "endpoint": "http://127.0.0.1:9", "title": "Sheet bucket" })).await.unwrap();
    let answer = json!({ "fields": { "endpoint": "http://127.0.0.1:9", "bucket": "b", "key_id": "k", "secret": "s" } });
    a.answer_request(&me, &global, asked["id"].as_str().unwrap(), &answer).await.unwrap();
    assert!(place("Sheet bucket").is_some_and(|p| p["kind"] == "s3"), "{}", a.backups().unwrap());
}

/// The notes' lines in `global` as a place's copy of the network's bucket holds them, opened with the recovery key
/// alone: each epoch's key sealed to it, each app's key in the epoch sealed under that.
async fn backed_up_posts(dir: &std::path::Path, install: &str, recovery: &[u8; 32]) -> Vec<String> {
    backed_up(dir, install, recovery, "global", "notes", "main", "select text from posts order by n").await
}

/// The first column of what `sql` reads from `app`'s `database` in `scope`, as the backup's copy of the bucket has it,
/// opened with the recovery key alone.
async fn backed_up(dir: &std::path::Path, install: &str, recovery: &[u8; 32], scope: &str, app: &str, database: &str, sql: &str) -> Vec<String> {
    let (copy, keys) = backed_up_keys(dir, install, recovery, scope, app, database);
    match one::database::stored::Db::open(&copy, &format!("{}/{app}/{database}", one::bucket::folder(scope)), keys).await {
        Ok(db) => db.query(sql, &[]).unwrap().into_iter().filter_map(|r| r.values().next().and_then(|v| v.as_str()).map(str::to_string)).collect(),
        Err(_) => Vec::new(),
    }
}

/// The backup's copy of the network's bucket, and the keys of `app`'s `database` in `scope` there, opened with the
/// recovery key alone.
fn backed_up_keys(dir: &std::path::Path, install: &str, recovery: &[u8; 32], scope: &str, app: &str, database: &str) -> (one::bucket::Place, one::database::stored::Keys) {
    let copy = one::bucket::Place::Folder(dir.join(install).join("bucket"));
    let folder = one::bucket::folder(scope);
    let mut keys = one::database::stored::Keys::new();
    for n in copy.list(&format!("{folder}/_epochs")).unwrap().iter().filter_map(|n| n.parse::<u32>().ok()) {
        let epoch = one::backups::unseal(recovery, &copy.get(&one::keys::epoch_name(scope, n, "recovery")).unwrap().unwrap()).unwrap();
        let Ok(Some(sealed)) = copy.get(&one::keys::app_key_name(scope, app, n)) else { continue };
        keys.insert(n, one::keys::open_database_key(&epoch, app, database, &sealed).unwrap());
    }
    (copy, keys)
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        match e.path().is_dir() {
            true => out.extend(walk(&e.path())),
            false => out.push(e.path()),
        }
    }
    out
}

/// A device updates, or deploys the Workers, only when asked by a device who's who says may.
#[tokio::test(flavor = "multi_thread")]
async fn only_someone_who_may_remove_a_device_can_ask_it_to_update() {
    let secret: [u8; 32] = rand::random();
    let dir = temp("update-asked");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::keys::hex(&secret), "name": "a" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    create(&a, "n", "Ann").await;
    let me = ed25519_dalek::SigningKey::from_bytes(&secret);
    let sam = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let_sam_in(&a, &sam).await;
    let stranger = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let ask = |by: &ed25519_dalek::SigningKey, of: [u8; 32]| one::keys::signed_request(by, one::protocol::RELAY, json!({ "update": one::keys::hex(&of) }), one::node::now_ms());
    assert_eq!(a.who_asked_to_update(&ask(&me, a.id())).unwrap().0, "a", "its own user may");
    assert!(a.who_asked_to_update(&ask(&sam, a.id())).is_err(), "someone else's device may not");
    assert!(a.who_asked_to_update(&ask(&stranger, a.id())).is_err(), "nor a device who's who doesn't list");
    assert!(a.who_asked_to_update(&ask(&me, sam.verifying_key().to_bytes())).is_err(), "an ask for another device isn't for this one");
    let forged = { let mut f = ask(&me, a.id()); f["update"] = json!(one::keys::hex(&rand::random::<[u8; 32]>())); f };
    assert!(a.who_asked_to_update(&forged).is_err(), "a change to the ask breaks its signature");
    // Deploying the Workers again, which the relay leaves for the device they're deployed from, is for an admin's device.
    let deploy = |by: &ed25519_dalek::SigningKey| one::keys::signed_request(by, one::protocol::RELAY, json!({ "cloudflare": true }), one::node::now_ms());
    assert_eq!(a.who_asked_to_update(&deploy(&me)).unwrap().0, "a", "an admin's");
    assert!(a.who_asked_to_update(&deploy(&sam)).is_err(), "not someone else's");
    assert!(a.who_asked_to_update(&deploy(&stranger)).is_err(), "nor a device who's who doesn't list");
}

/// A device is approved only by its user's own device, or a newcomer's by the admin whose invite their proof was made
/// with; a device removed opens no epoch made after.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_users_own_device_or_their_inviter_lets_a_device_in() {
    let dir = temp("approve");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::keys::hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    eventually("notes to run", &a, async || a.call(notes("post", json!({ "text": "hi" }))).await.is_ok()).await;
    let global = one::database::scopes::Scope::Global;
    let new_key = || ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let opens = async |key: &ed25519_dalek::SigningKey, scope: &one::database::scopes::Scope| -> Vec<u32> {
        let got = a.sealed_keys(&me, scope, Some(&one::keys::hex(&key.verifying_key().to_bytes())), None).await.unwrap();
        let sealed = got["sealed"].as_object().unwrap();
        sealed.iter().filter(|(_, v)| one::keys::open_sealed(&one::keys::seal_secret(key), &data_encoding::BASE64URL_NOPAD.decode(v.as_str().unwrap().as_bytes()).unwrap()).is_ok()).map(|(n, _)| n.parse().unwrap()).collect()
    };

    // Andre's new laptop, approved from his machine, opens global.
    let laptop = new_key();
    one::settings::approve(&a, &asks(&laptop, &me.user, None), &one::settings::Joining::Own { proved: true }).await.unwrap();
    assert_eq!(opens(&laptop, &global).await, vec![1]);

    // Sam is let in only with the proof of the secret Andre's invite made.
    let (sam, sams) = (one::database::scopes::new_id(), new_key());
    let invite = one::settings::Joining::Link(invited("the-invites-secret"));
    assert!(one::settings::approve(&a, &asks(&sams, &sam, Some(b"another-secret")), &invite).await.is_err());
    assert!(!a.is_user(&sam), "a request whose proof doesn't match lets no one in");
    one::settings::approve(&a, &asks(&sams, &sam, Some(b"the-invites-secret")), &invite).await.unwrap();
    assert!(a.is_user(&sam));
    assert_eq!(opens(&sams, &global).await, vec![1]);

    // Sam's next device is his to approve, not Andre's.
    let refused = one::settings::approve(&a, &asks(&new_key(), &sam, None), &one::settings::Joining::Own { proved: true }).await.unwrap_err();
    assert!(format!("{refused:#}").contains("only their own devices"), "{refused:#}");

    // The laptop, removed, opens no epoch made after; Andre's machine and Sam's device do, and the notes go on.
    let laptop_key = json!({ "device": one::keys::hex(&laptop.verifying_key().to_bytes()) });
    let removed = one::settings::answer(&a, "POST", "/api/devices/remove", laptop_key.to_string().as_bytes()).await.unwrap().unwrap();
    assert_eq!(removed["not_renewed"], json!([]), "{removed}");
    assert_eq!(opens(&laptop, &global).await, vec![1]);
    for f in walk(&dir).into_iter().filter(|p| p.to_string_lossy().contains("/_epochs/2/")) {
        assert!(one::keys::open_sealed(&one::keys::seal_secret(&laptop), &std::fs::read(&f).unwrap()).is_err(), "{} opens for the removed laptop", f.display());
    }
    assert_eq!(opens(&sams, &global).await, vec![1, 2]);
    a.call(notes("post", json!({ "text": "after" }))).await.unwrap();
}

/// Removing a device with an app runtime of its own takes the runtime out with it, named by either: a runtime is never
/// left without its device, nor a device without its runtime.
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_device_takes_its_app_runtime_and_naming_the_runtime_removes_the_device() {
    use one::database::scopes::Kind;
    let a = start("remove-runtime").await;
    create(&a, "Home", "Andre").await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    // A computer of Andre's, approved from his machine, listing its own app runtime, as its onet does.
    let computer = async || {
        let device = ed25519_dalek::SigningKey::from_bytes(&rand::random());
        one::settings::approve(&a, &asks(&device, &me.user, None), &one::settings::Joining::Own { proved: true }).await.unwrap();
        let runtime = one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()));
        let listed = one::database::content::runtime(&runtime, &me.user, "computer", &device.verifying_key().to_bytes());
        signed_by(&a, &me, &device, listed).await;
        (device.verifying_key().to_bytes(), runtime.key)
    };
    let listed = |k: [u8; 32]| one::settings::Me::who(&a, |w| w.machine(&k)).unwrap().is_some();
    let remove = async |k: [u8; 32]| one::settings::answer(&a, "POST", "/api/devices/remove", json!({ "device": one::keys::hex(&k) }).to_string().as_bytes()).await.unwrap();

    let (device, runtime) = computer().await;
    assert!(listed(device) && listed(runtime));
    remove(device).await.unwrap();
    assert!(!listed(device) && !listed(runtime), "removing the device takes its runtime");

    let (device, runtime) = computer().await;
    remove(runtime).await.unwrap();
    assert!(!listed(device) && !listed(runtime), "naming its runtime removes the device");
}

/// The recovery key is sealed only to a device of someone an admin named, not to a node, and only where nothing holds it
/// yet; naming a holder changes nothing a device already opens; and one taken off is sealed no more.
#[tokio::test(flavor = "multi_thread")]
async fn the_recovery_key_is_sealed_only_to_a_named_holders_device() {
    use data_encoding::BASE64URL_NOPAD as B64U;
    use one::keys::{Endpoint, hex, open_sealed, seal_secret, seal_to, unhex};
    let dir = temp("holders");
    std::fs::write(dir.join("device.json"), json!({ "secret": hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    let secret = unhex(&create(&a, "Home", "Andre").await).unwrap();
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    let global = one::database::scopes::Scope::Global;
    let new_device = async |platform: &str| {
        let key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
        let e = Endpoint::of(&key);
        let asked = one::settings::Asked::of(&json!({ "key": hex(&e.key), "seal": hex(&e.seal), "name": platform, "user": me.user, "platform": platform })).unwrap();
        one::settings::approve(&a, &asked, &one::settings::Joining::Own { proved: true }).await.unwrap();
        key
    };
    let recovery = ed25519_dalek::SigningKey::from_bytes(&secret);
    let sealed_for = |key: &ed25519_dalek::SigningKey| {
        let sealed = seal_to(&Endpoint::of(key).seal, &secret).unwrap();
        B64U.encode(&one::keys::held_copy(&recovery, &hex(&key.verifying_key().to_bytes()), &sealed))
    };
    let name = async |holds: bool| one::settings::answer(&a, "POST", "/api/devices/recovery-holder", json!({ "user": me.user, "holds": holds }).to_string().as_bytes()).await.unwrap().unwrap();
    let (phone, browser) = (new_device("iphone").await, new_device("browser").await);
    let opens = async |key: &ed25519_dalek::SigningKey| a.sealed_keys(&me, &global, Some(&hex(&key.verifying_key().to_bytes())), None).await.unwrap()["sealed"].clone();
    let before = opens(&phone).await;

    let to = hex(&phone.verifying_key().to_bytes());
    assert!(one::settings::recovery::seal(&a, &to, &sealed_for(&phone)).await.is_err(), "no one is named yet");
    name(true).await;
    assert_eq!(opens(&phone).await, before, "naming a holder rewrites nothing sealed");
    let sealed_to_node = seal_to(&one::keys::public_of(&[9; 32]), &secret).unwrap();
    assert!(one::settings::recovery::seal(&a, &hex(&a.id()), &B64U.encode(&one::keys::held_copy(&recovery, &hex(&a.id()), &sealed_to_node))).await.is_err(), "not a node");
    assert!(one::settings::recovery::seal(&a, &hex(&browser.verifying_key().to_bytes()), &sealed_for(&browser)).await.is_err(), "not a browser");

    let bad_key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let sealed = seal_to(&Endpoint::of(&phone).seal, &secret).unwrap();
    assert!(one::settings::recovery::seal(&a, &to, &B64U.encode(&one::keys::held_copy(&bad_key, &to, &sealed))).await.is_err(), "only the recovery key signs a copy");
    one::settings::recovery::seal(&a, &to, &sealed_for(&phone)).await.unwrap();
    // Anyone who can reach the bucket can create an object at that name: it neither counts nor stands in the way.
    let files: Vec<_> = walk(&dir).into_iter().filter(|p| p.to_string_lossy().contains("/_recovery/")).collect();
    assert!(!files.is_empty());
    for file in &files {
        std::fs::write(file, b"anyone can make this").unwrap();
    }
    assert_eq!(a.recovery_holders(&me).await.unwrap()["holders"][0]["devices"][0]["holds"], false, "a fake isn't counted");
    one::settings::recovery::seal(&a, &to, &sealed_for(&phone)).await.unwrap();
    let listed = a.recovery_holders(&me).await.unwrap();
    assert_eq!(listed["holders"][0]["devices"][0]["holds"], true, "{listed}");
    assert_eq!(listed["holders"][0]["devices"].as_array().unwrap().len(), 1, "only the phone: {listed}");
    assert!(one::settings::recovery::seal(&a, &to, &sealed_for(&phone)).await.is_err(), "the one there stays");
    let held: Vec<_> = walk(&dir).into_iter().filter(|p| p.to_string_lossy().contains("/_recovery/")).collect();
    assert_eq!(held.len(), 1, "{held:?}");
    let copy = std::fs::read(&held[0]).unwrap();
    let opened = one::keys::held_sealed(&recovery.verifying_key().to_bytes(), &to, &copy).expect("signed by the recovery key");
    assert_eq!(open_sealed(&seal_secret(&phone), opened).unwrap(), secret, "it opens to the key on that device alone");
    assert!(open_sealed(&seal_secret(&browser), opened).is_err());

    name(false).await;
    let laptop = new_device("linux").await;
    assert!(one::settings::recovery::seal(&a, &hex(&laptop.verifying_key().to_bytes()), &sealed_for(&laptop)).await.is_err(), "taken off, none of their devices is sealed it");
    assert!(a.recovery_holders(&me).await.unwrap()["holders"].as_array().unwrap().is_empty());
}

/// Someone who lost every device: a holder's own copy of the recovery key gives their new device their own scope's keys,
/// by the code on that device's screen, and someone not named as a holder can't. The holder's device here is a node
/// standing in for a client, its copy put where a client's would be, as a node is never sealed one; and being the same
/// user's, its node may also hand the device their keys, so what shows restoring did it is what restoring says it opened.
#[tokio::test(flavor = "multi_thread")]
async fn a_holder_restores_someone_and_no_one_else_can() {
    use data_encoding::BASE64URL_NOPAD as B64U;
    use one::database::scopes::Scope;
    use one::keys::{Endpoint, hex, join_code, seal_to, unhex};
    let dir = temp("restore");
    std::fs::write(dir.join("device.json"), json!({ "secret": hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    let secret = unhex(&create(&a, "Home", "Andre").await).unwrap();
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    let own = Scope::User(me.user.clone());
    eventually("their own scope's first keys", &a, async || !a.sealed_keys(&me, &own, Some(&hex(&a.id())), None).await.unwrap()["sealed"].as_object().unwrap().is_empty()).await;
    let recovery = ed25519_dalek::SigningKey::from_bytes(&secret);
    let phone = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let e = Endpoint::of(&phone);
    let asked = one::settings::Asked::of(&json!({ "key": hex(&e.key), "seal": hex(&e.seal), "name": "new phone", "user": me.user, "platform": "iphone" })).unwrap();
    one::settings::approve(&a, &asked, &one::settings::Joining::Lost).await.unwrap();
    let to = hex(&e.key);
    let code = one::keys::spaced_code(&join_code(&e));
    let opens = async || a.sealed_keys(&me, &own, Some(&to), None).await.unwrap()["sealed"].as_object().unwrap().len();
    let name = async |holds: bool| one::settings::answer(&a, "POST", "/api/devices/recovery-holder", json!({ "user": me.user, "holds": holds }).to_string().as_bytes()).await.unwrap().unwrap();
    let restore = async |confirm: bool| one::settings::answer(&a, "POST", "/api/devices/restore", json!({ "code": code, "confirm": confirm }).to_string().as_bytes()).await.unwrap();

    assert!(restore(true).await.is_err(), "no one is named yet");
    name(true).await;
    assert!(restore(true).await.unwrap_err().to_string().contains("doesn't have the recovery key"), "named, without its own copy");
    one::settings::recovery::seal(&a, &to, &B64U.encode(&one::keys::held_copy(&recovery, &to, &seal_to(&e.seal, &secret).unwrap()))).await.unwrap();
    let phone_copy = walk(&dir).into_iter().find(|p| p.to_string_lossy().contains("/_recovery/")).unwrap();
    let mine = phone_copy.with_file_name(hex(&a.id()));
    std::fs::write(&mine, one::keys::held_copy(&recovery, &hex(&a.id()), &seal_to(&one::settings::Me::seal(&a), &secret).unwrap())).unwrap();

    assert!(restore(false).await.unwrap()["said"].as_str().unwrap().contains("new phone"), "without confirm, whose it is");
    let done = restore(true).await.unwrap();
    assert!(done["given"].as_array().unwrap().iter().any(|s| s.as_str() == Some(&own.to_string())), "{done}");
    assert_eq!(opens().await, 1, "their own scope reaches them");
    let wrong = one::settings::answer(&a, "POST", "/api/devices/restore", json!({ "code": "AAAA AAAA AAAA AAAA", "confirm": true }).to_string().as_bytes()).await.unwrap();
    assert!(wrong.is_err(), "a code no listed device shows does nothing");
    name(false).await;
    assert!(restore(true).await.is_err(), "taken off the list, they restore no one");
}

/// Replacing the recovery key: old holders' copies stop counting, each scope gets a new epoch only the new key and its
/// users open, every old epoch still opens with both keys, and nothing already sealed is deleted. Run again, it changes
/// nothing more.
#[tokio::test(flavor = "multi_thread")]
async fn a_replaced_recovery_key_stops_what_the_old_one_opens_growing() {
    use one::keys::{Endpoint, epoch_name, hex, open_sealed, seal_secret, seal_to, twin_public, unhex};
    use data_encoding::BASE64URL_NOPAD as B64U;
    let dir = temp("replace");
    std::fs::write(dir.join("device.json"), json!({ "secret": hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    let old_secret = unhex(&create(&a, "Home", "Andre").await).unwrap();
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    let global = one::database::scopes::Scope::Global;
    let old = ed25519_dalek::SigningKey::from_bytes(&old_secret);
    let phone = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let e = Endpoint::of(&phone);
    let asked = one::settings::Asked::of(&json!({ "key": hex(&e.key), "seal": hex(&e.seal), "name": "phone", "user": me.user, "platform": "iphone" })).unwrap();
    one::settings::approve(&a, &asked, &one::settings::Joining::Own { proved: true }).await.unwrap();
    one::settings::answer(&a, "POST", "/api/devices/recovery-holder", json!({ "user": me.user, "holds": true }).to_string().as_bytes()).await.unwrap().unwrap();
    one::settings::recovery::seal(&a, &hex(&e.key), &B64U.encode(&one::keys::held_copy(&old, &hex(&e.key), &seal_to(&e.seal, &old_secret).unwrap()))).await.unwrap();
    assert_eq!(a.recovery_holders(&me).await.unwrap()["holders"][0]["devices"][0]["holds"], true);

    let own = one::database::scopes::Scope::User(me.user.clone());
    a.set_item(&me, &own, json!({ "title": "Example key", "fields": { "token": { "value": "s3cret-value", "secret": true } }, "hosts": ["api.example.com"] })).await.unwrap();
    let new = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let new_secret = seal_secret(&new);
    let file_at = |name: &str| walk(&dir).into_iter().find(|p| p.to_string_lossy().ends_with(name));
    let opened = |sealed: &[u8], secret: &[u8; 32]| open_sealed(secret, sealed).ok().and_then(|k| <[u8; 32]>::try_from(k).ok());
    let before = a.sealed_keys(&me, &global, Some("recovery"), None).await.unwrap();
    let n1 = before["epoch"].as_u64().unwrap() as u32;

    let said = one::settings::recovery::replace(&a, &old, &new, false).await.unwrap();
    assert!(said["said"].as_str().unwrap().contains("Would"), "{said}");
    assert_eq!(a.sealed_keys(&me, &global, Some("recovery"), None).await.unwrap()["epoch"], n1, "and changes nothing");
    assert!(one::settings::recovery::replace(&a, &new, &ed25519_dalek::SigningKey::from_bytes(&rand::random()), true).await.is_err(), "a file that isn't the recovery key changes nothing");

    let done = one::settings::recovery::replace(&a, &old, &new, true).await.unwrap();
    assert!(done["not_renewed"].as_array().unwrap().is_empty() && done["not_carried"].as_array().unwrap().is_empty(), "{done}");
    assert_eq!(a.state().unwrap().network.unwrap().recovery.unwrap(), hex(&new.verifying_key().to_bytes()));
    let n2 = a.sealed_keys(&me, &global, Some("recovery"), None).await.unwrap()["epoch"].as_u64().unwrap() as u32;
    assert!(n2 > n1, "global has a new epoch");

    let new_copy = std::fs::read(file_at(&epoch_name("global", n2, "recovery")).unwrap()).unwrap();
    let key = opened(&new_copy, &new_secret).expect("the new epoch opens with the new key");
    assert!(opened(&new_copy, &seal_secret(&old)).is_none(), "and never with the old");
    let to_phone = std::fs::read(file_at(&epoch_name("global", n2, &hex(&e.key))).unwrap()).unwrap();
    assert_eq!(opened(&to_phone, &seal_secret(&phone)), Some(key), "and its users' devices open it");
    let carried = std::fs::read(file_at(&epoch_name("global", n1, &one::keys::recovery_to(&twin_public(&new.verifying_key().to_bytes()).unwrap()))).unwrap()).unwrap();
    let was = std::fs::read(file_at(&epoch_name("global", n1, "recovery")).unwrap()).unwrap();
    assert_eq!(opened(&carried, &new_secret), opened(&was, &seal_secret(&old)), "the new key opens what the old did, and the old copy is still there");

    let vault = std::fs::read(file_at(&one::keys::vault_key_name(&own.to_string(), 2, "recovery")).expect("the Vault has new keys")).unwrap();
    assert!(opened(&vault, &new_secret).is_some() && opened(&vault, &seal_secret(&old)).is_none(), "under the new key only");

    assert_eq!(a.recovery_holders(&me).await.unwrap()["holders"][0]["devices"][0]["holds"], false, "old holders' copies stop counting");
    assert!(walk(&dir).into_iter().any(|p| p.to_string_lossy().contains(&format!("/_recovery/{}/", hex(&old.verifying_key().to_bytes())))), "and nothing sealed was deleted");

    let again = one::settings::recovery::replace(&a, &old, &new, true).await.unwrap();
    assert!(again["renewed"].as_array().unwrap().is_empty(), "run again, nothing's made twice: {again}");
    assert_eq!(a.sealed_keys(&me, &global, Some("recovery"), None).await.unwrap()["epoch"].as_u64().unwrap() as u32, n2);
}

/// A backup keeps the copies of the recovery key its holders' devices have, as they were signed, so a restored network
/// still knows who held it.
#[tokio::test(flavor = "multi_thread")]
async fn a_backup_keeps_a_holders_copy_of_the_recovery_key() {
    use data_encoding::BASE64URL_NOPAD as B64U;
    use one::keys::{Endpoint, hex, open_sealed, seal_secret, seal_to, unhex};
    let a = start("holder-backup").await;
    let secret = unhex(&create(&a, "Home", "Andre").await).unwrap();
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    let recovery = ed25519_dalek::SigningKey::from_bytes(&secret);
    let phone = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let e = Endpoint::of(&phone);
    let asked = one::settings::Asked::of(&json!({ "key": hex(&e.key), "seal": hex(&e.seal), "name": "phone", "user": me.user, "platform": "iphone" })).unwrap();
    one::settings::approve(&a, &asked, &one::settings::Joining::Own { proved: true }).await.unwrap();
    one::settings::answer(&a, "POST", "/api/devices/recovery-holder", json!({ "user": me.user, "holds": true }).to_string().as_bytes()).await.unwrap().unwrap();
    let to = hex(&e.key);
    let sealed = seal_to(&e.seal, &secret).unwrap();
    one::settings::recovery::seal(&a, &to, &B64U.encode(&one::keys::held_copy(&recovery, &to, &sealed))).await.unwrap();

    let dir = temp("holder-backup-disk").join("One Backups");
    a.add_place(&me, &json!({ "path": dir, "title": "Disk" })).await.unwrap();
    eventually("the first backup", &a, async || !a.backups().unwrap()["places"][0]["last"].is_null()).await;
    let place = one::bucket::Place::Folder(dir.clone());
    let network = one::backups::networks(&place).unwrap().remove(0);
    let copied = place.get(&format!("{network}/bucket/{}", one::keys::recovery_held_name(&hex(&recovery.verifying_key().to_bytes()), &to))).unwrap().expect("the backup keeps it");
    let opened = one::keys::held_sealed(&recovery.verifying_key().to_bytes(), &to, &copied).expect("still signed");
    assert_eq!(open_sealed(&seal_secret(&phone), opened).unwrap(), secret, "and it opens with the holder's device alone");
}

/// A file put from elsewhere (a client device's, a sign-in's) is read back by whoever put it alone, or by someone who
/// may use a scope that names it: knowing its hash isn't enough. A file put to answer one request (git's push, to
/// Repos' `_http`) goes once it's answered, unless something keeps it.
/// A Settings change whose body doesn't read as its route's is refused, never taken as another: a mistyped role
/// doesn't take someone out of a group. Invites this device can't read are an error, never written over.
#[tokio::test(flavor = "multi_thread")]
async fn a_settings_change_that_doesnt_read_is_refused() {
    let dir = temp("bodies");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::keys::hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    create(&a, "Home", "Andre").await;
    let me = a.user().unwrap();
    let ask = async |path: &str, body: Value| one::settings::answer(&a, "POST", path, body.to_string().as_bytes()).await.unwrap();
    let group = ask("/api/group", json!({ "name": "Team" })).await.unwrap()["group"].as_str().unwrap().to_string();
    let refused = ask("/api/member", json!({ "group": group, "user": me, "role": "admn" })).await.unwrap_err();
    assert!(format!("{refused:#}").contains("admn"), "{refused:#}");
    assert!(ask("/api/invite", json!({ "group": group, "user": me, "role": "owner" })).await.is_err());
    assert!(ask("/api/role", json!({ "user": me, "role": "admin", "extra": true })).await.is_err());
    let role = one::settings::Me::who(&a, |db| db.role_in(&me, &one::database::scopes::Scope::Group(group.clone()))).unwrap();
    assert_eq!(role, Some(one::database::scopes::Role::Admin), "still the group's admin");

    std::fs::write(dir.join("invites.json"), b"{ not json").unwrap();
    let refused = ask("/api/devices/invite", json!({ "name": "Sam" })).await.unwrap_err();
    assert!(format!("{refused:#}").contains("can't be read"), "{refused:#}");
    assert_eq!(std::fs::read(dir.join("invites.json")).unwrap(), b"{ not json");
}

/// A build that couldn't open a sealed format took a step in it for junk and kept a position past it. What it kept is
/// read again, so the device ends up with the step it passed over.
#[tokio::test(flavor = "multi_thread")]
async fn a_position_kept_by_a_build_that_passed_over_a_step_is_read_again() {
    let dir = temp("kept-before");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::keys::hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    create(&a, "Home", "Andre").await;
    let me = a.user().unwrap();
    let kept = std::path::Path::new("network/scope-kept").join(data_encoding::HEXLOWER.encode(b"global"));
    let read = |dir: &std::path::Path| serde_json::from_slice::<Value>(&std::fs::read(dir.join(&kept)).unwrap()).unwrap();
    let mut before = read(&dir);
    let answer = one::settings::answer(&a, "POST", "/api/group", json!({ "name": "Team" }).to_string().as_bytes()).await.unwrap().unwrap();
    let group = one::database::scopes::Scope::Group(answer["group"].as_str().unwrap().to_string());
    let after = read(&dir);
    assert!(after["at"].as_u64() > before["at"].as_u64(), "making a group is a step");

    let copy = temp("kept-before-copy");
    assert!(std::process::Command::new("cp").arg("-r").arg(dir.join(".")).arg(&copy).status().unwrap().success());
    before["at"] = after["at"].clone();
    before.as_object_mut().unwrap().remove("sealed").expect("what's kept says what it read");
    std::fs::write(copy.join(&kept), before.to_string()).unwrap();

    let b = Node::start(&copy).await.unwrap();
    let is_admin = async || one::settings::Me::who(&b, |db| db.role_in(&me, &group)).ok().flatten() == Some(one::database::scopes::Role::Admin);
    eventually("the step it passed over", &b, is_admin).await;
}

/// Erasing an app deletes its keys in the bucket whoever erases it: here Settings' own answer, as any client's.
#[tokio::test(flavor = "multi_thread")]
async fn erasing_an_app_deletes_its_keys_whoever_erases_it() {
    let dir = temp("erase");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::keys::hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    eventually("notes to run", &a, async || a.call(notes("post", json!({ "text": "hi" }))).await.is_ok()).await;
    let keys = || walk(&dir).into_iter().filter(|p| p.to_string_lossy().contains("/notes/_key/")).count();
    assert!(keys() > 0, "notes has keys");
    for how in ["deleted", "erased"] {
        one::settings::answer(&a, "POST", "/api/retire", json!({ "app": "notes", "how": how }).to_string().as_bytes()).await.unwrap().unwrap();
    }
    assert_eq!(keys(), 0, "notes' keys are gone");
}

/// Taking an app off a page keeps what it kept there, and adding it back brings it back.
#[tokio::test(flavor = "multi_thread")]
async fn an_app_taken_off_a_page_and_added_back_has_what_it_kept() {
    let a = start("remove-add").await;
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    eventually("notes to run", &a, async || a.call(notes("post", json!({ "text": "before it left" }))).await.is_ok()).await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    a.add(&me, "notes", &one::database::scopes::Scope::Global, false, None).await.unwrap();
    assert!(a.call(notes("read", json!({}))).await.is_err(), "an app off the page doesn't run there");
    a.add(&me, "notes", &one::database::scopes::Scope::Global, true, None).await.unwrap();
    let read = a.call(notes("read", json!({}))).await.unwrap();
    assert!(read.to_string().contains("before it left"), "{read}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_put_from_elsewhere_is_read_only_by_whoever_put_it() {
    let a = start("put-from").await;
    create(&a, "Home", "Andre").await;
    let caller = |user: &str| one::node::Caller { key: a.id(), user: user.into(), agent: None, credential: one::node::Credential::Client([user.len() as u8; 32]) };
    let (sam, andre) = (caller("sam"), caller(&a.user().unwrap()));
    let (hash, size) = a.put_blob_from(&sam, axum::body::Body::from("Sam's file")).await.unwrap();
    assert_eq!(size, 10);
    assert_eq!(a.blob_for(&sam, &hash).await.unwrap(), b"Sam's file", "Sam reads back what Sam put");
    assert!(a.blob_for(&andre, &hash).await.is_err(), "and Andre doesn't, by its hash");
    let theirs = a.put_blob(b"an attachment of someone else's").unwrap();
    assert!(a.blob_for(&sam, &theirs).await.is_err(), "nor Sam one nobody gave him");
    a.drop_put(&hash);
    assert!(a.blob_for(&sam, &hash).await.is_err(), "a request's file goes once it's answered");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trace_says_where_a_call_went_and_never_what_it_carried() {
    use one::node::trace;
    let a = start("trace-a").await;
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    eventually("notes to run", &a, async || a.call(notes("post", json!({ "text": "a private thought" }))).await.is_ok()).await;
    let t = trace::Trace::new();
    let read = trace::within(Some(t.clone()), a.call(notes("read", json!({ "format": "json" })))).await.unwrap();
    assert!(read.to_string().contains("a private thought"), "{read}");
    let said = t.done(&a.device_name()).to_string();
    assert!(said.contains(r#""step":"device""#) && said.contains(r#""tool":"read""#) && said.contains(r#""step":"run""#), "{said}");
    assert!(!said.contains("private thought") && !said.contains(&a.user().unwrap()), "a trace holds names and timings alone: {said}");
}

/// What insights keeps is names and numbers, and what a real failure said, cleaned: a call's arguments and what it was
/// refused with are the caller's, and go nowhere it can be read.
#[tokio::test(flavor = "multi_thread")]
async fn a_refusals_words_never_reach_what_insights_keeps_and_a_failures_are_cleaned() {
    let a = start("insights").await;
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    eventually("notes to run", &a, async || a.call(notes("post", json!({ "text": "a private thought" }))).await.is_ok()).await;
    let refused = a.call(notes("refuse", json!({ "text": "a private thought" }))).await.unwrap_err();
    assert!(format!("{refused:#}").contains("a private thought"), "the caller still reads it: {refused:#}");
    let failed = a.call(notes("fail", json!({ "text": "couldn't mail anna@example.org" }))).await.unwrap_err();
    assert!(format!("{failed:#}").contains("anna@example.org"), "the caller still reads it: {failed:#}");
    let batch = a.take_insights();
    let sent = batch.rows();
    let said = sent.to_string();
    let spans = sent["spans"].as_array().unwrap();
    let of = |name: &str| spans.iter().find(|s| s["name"] == name).unwrap_or_else(|| panic!("no {name}: {said}"));
    assert_eq!(of("refuse")["status"], "unset", "{said}");
    assert_eq!(of("fail")["status"], "error", "{said}");
    assert_eq!(of("fail")["message"], "couldn't mail <email>", "{said}");
    assert!(of("fail")["attrs"].as_str().unwrap().contains("one.fingerprint"), "{said}");
    assert!(of("refuse")["attrs"].as_str().unwrap().contains("one.fingerprint"), "and it's in the list of what was refused: {said}");
    assert_eq!(of("refuse")["message"], Value::Null, "a refusal's words go nowhere: {said}");
    assert!(of("post")["kind"] == "server", "every request is sent, for its rates: {said}");
    assert!(!said.contains("private thought") && !said.contains("anna"), "only names, numbers and a failure's cleaned words: {said}");
    let attrs: Value = serde_json::from_str(of("post")["attrs"].as_str().unwrap()).unwrap();
    assert_eq!(attrs["enduser.id"], a.user().unwrap(), "a call says who made it, by their id: {said}");
}

/// A device's log is read over the network by whoever may remove the device (its user, and the network's admins), never
/// by another member, and what's returned is capped.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_devices_user_or_an_admin_reads_its_logs() {
    let dir = temp("logs");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::keys::hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    create(&a, "Home", "Andre").await;
    let andre = one::node::Caller::own(a.id(), a.user().unwrap());
    let now = one::node::now_ms();
    let log: String = (0..3000).map(|i| format!("{} line {i}\n", now - 1000 + i)).collect();
    std::fs::create_dir_all(dir.join("logs")).unwrap();
    std::fs::write(dir.join("logs/node.log"), log).unwrap();
    let logs = |input: Value| CallRequest { app: "node".into(), tool: "logs".into(), device: Some("mac".into()), input, ..Default::default() };

    let read = a.call_as(logs(json!({ "grep": "line 29" })), andre.clone()).await.unwrap();
    assert_eq!(read["matched"], 111, "{read}");
    let all = a.call_as(logs(json!({ "lines": 100000 })), andre.clone()).await.unwrap();
    assert!(all["shown"].as_u64().unwrap() <= one::logs::MOST_LINES as u64, "a read is capped");

    // Sam, invited as a member, may not; once an admin, may.
    let sam = let_sam_in(&a, &ed25519_dalek::SigningKey::from_bytes(&rand::random())).await;
    let from_sam = one::node::Caller::own(a.id(), sam.clone());
    let refused = a.call_as(logs(json!({})), from_sam.clone()).await.unwrap_err();
    assert!(format!("{refused:#}").contains("reads its logs"), "{refused:#}");
    assert!(!a.may_read_logs(&sam).unwrap() && a.may_read_logs(&andre.user).unwrap());
    a.member_content(&andre, one::database::content::role(&sam, one::database::scopes::Role::Admin)).await.unwrap();
    assert!(a.may_read_logs(&sam).unwrap(), "an admin of the network reads any device's logs");
}

/// A node runs a build of an app's service only if its app's latest deploy names it: one whose bytes aren't that hash's
/// is refused, a deploy naming none for this platform runs none, and only someone who may deploy the app names one.
#[tokio::test(flavor = "multi_thread")]
async fn a_service_runs_only_the_build_its_apps_deploy_names() {
    use one::oned::Build;
    let dir = temp("service-build");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::keys::hex(&rand::random()), "name": "linux" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    create(&a, "Home", "Andre").await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    let build = a.put_blob(b"a program").unwrap();
    let bundle = |services: Value| {
        let files = vec![("app.js".to_string(), b"export default { tools: {} }".to_vec()), (one::app::bundle::SERVICES.to_string(), services.to_string().into_bytes())];
        a.put_blob(&one::app::bundle::pack(&files).unwrap()).unwrap()
    };
    let named = bundle(json!({ one::config::TARGET: build }));
    a.deploy_making(&me, "tiny", &named, None, false).await.unwrap();
    assert_eq!(a.service_builds().await.get("tiny"), Some(&Build::Deployed(build.clone())));
    assert_eq!(a.service_build(&build).await.unwrap(), b"a program");

    // One byte changed where it's kept, and it isn't the build its deploy names.
    let kept = one::release::blob_path(&dir, &build).unwrap();
    std::fs::write(&kept, b"a progran").unwrap();
    assert!(a.service_build(&build).await.is_err(), "a build whose bytes aren't its hash is refused");

    // A deploy naming builds only for other platforms: none runs here.
    a.deploy(&me, "tiny", &bundle(json!({ "elsewhere": build })), false).await.unwrap();
    assert_eq!(a.service_builds().await.get("tiny"), Some(&Build::None));

    // Sam may not deploy Andre's app, so names no build of its service.
    let sam = one::node::Caller { key: a.id(), user: "sam".into(), agent: None, credential: one::node::Credential::Client([8; 32]) };
    assert!(a.deploy(&sam, "tiny", &named, false).await.is_err());
    // Nor does Andre, from elsewhere, name a build he can't read: one Sam put, by its hash.
    let away = one::node::Caller { key: [7; 32], credential: one::node::Credential::Client([7; 32]), ..me.clone() };
    let (sams, _) = a.put_blob_from(&sam, axum::body::Body::from("Sam's program")).await.unwrap();
    let (his, _) = a.put_blob_from(&away, axum::body::Body::from(one::app::bundle::pack(&[("app.js".to_string(), b"export default {}".to_vec()), (one::app::bundle::SERVICES.to_string(), json!({ one::config::TARGET: sams }).to_string().into_bytes())]).unwrap())).await.unwrap();
    let refused = a.deploy(&away, "tiny", &his, false).await.unwrap_err();
    assert!(format!("{refused:#}").contains("put it first"), "{refused:#}");
}

/// An app's call of its service has the secret of an item switched on for that app filled in, when the item names the
/// service among its hosts, and blotted out of what comes back; another app's call, or one to a service the item
/// doesn't name, gets none of it.
#[tokio::test(flavor = "multi_thread")]
async fn an_apps_service_call_gets_its_switched_on_items_secret_and_no_other_does() {
    use one::database::scopes::Scope;
    let a = start("vault-fill").await;
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    let item = json!({ "title": "Work", "type": "Browser profile", "hosts": ["notes"], "apps": ["notes"], "fields": { "_key": { "value": "s3cret", "secret": true } } });
    let id = a.set_item(&me, &Scope::User(me.user.clone()), item).await.unwrap();
    let asked = || json!({ "profile": { "key": format!("{{{{{id}:_key}}}}") } });
    let mut input = asked();
    let used = a.fill_for_service("notes", &mut input, &me, Some("notes")).await.unwrap();
    assert_eq!(input["profile"]["key"], "s3cret");
    assert_eq!(used, vec!["s3cret".to_string()], "what it filled is blotted out of the answer");
    let mut input = asked();
    assert!(a.fill_for_service("notes", &mut input, &me, Some("other")).await.is_err() || input == asked(), "an app the item isn't switched on for gets none of it");
    let mut input = asked();
    assert!(a.fill_for_service("shell", &mut input, &me, Some("notes")).await.is_err() || !input.to_string().contains("s3cret"), "a service its item doesn't name gets none of it");
}

/// A secret is kept only under a Vault key some machine opens, so what's made while none does is refused, never sealed
/// where no grant afterwards opens it; what's made once one does opens, and still does after the next grant.
#[tokio::test(flavor = "multi_thread")]
async fn nothing_secret_is_kept_where_no_machine_opens_the_vault_and_what_is_kept_opens_after_a_grant() {
    use one::database::scopes::Scope;
    let a = start("vault-grant").await;
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    let own = Scope::User(me.user.clone());
    let grant = async |on: bool| one::settings::answer(&a, "POST", "/api/vault/grant", json!({ "device": a.device_name(), "on": on }).to_string().as_bytes()).await.unwrap().unwrap();
    let item = |title: &str| json!({ "title": title, "type": "Token", "hosts": ["notes"], "apps": ["notes"], "fields": { "_key": { "value": "s3cret", "secret": true } } });
    let opens = async |id: &str| {
        let mut input = json!({ "key": format!("{{{{{id}:_key}}}}") });
        a.fill_for_service("notes", &mut input, &me, Some("notes")).await.is_ok_and(|_| input["key"] == "s3cret")
    };
    let before = a.set_item(&me, &own, item("Before")).await.unwrap();
    grant(false).await;
    let refused = a.set_item(&me, &own, item("While none opens")).await.unwrap_err();
    assert!(format!("{refused:#}").contains("vault grant"), "it says to grant a machine: {refused:#}");
    assert!(a.set_item(&me, &own, json!({ "title": "Plain", "type": "Note", "fields": { "text": { "value": "hello" } } })).await.is_ok(), "an item with no secret needs no key");
    grant(true).await;
    let after = a.set_item(&me, &own, item("After")).await.unwrap();
    grant(true).await;
    assert!(opens(&before).await && opens(&after).await, "what was kept while a machine opened the Vault opens after each grant");
}

/// An app's schedules run on what a call of it is handed and no more, in their scope, only while they're set: its own
/// databases and the Vault's, so a scheduled run sees the items switched on for it, as a call does, and no other app's.
#[tokio::test(flavor = "multi_thread")]
async fn an_apps_schedules_see_its_vault_items_as_its_calls_do_while_theyre_set() {
    use one::database::scopes::Scope;
    let a = start("schedules-vault").await;
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    for (title, app) in [("For notes", "notes"), ("For another", "another")] {
        a.set_item(&me, &Scope::Global, json!({ "title": title, "type": "Token", "hosts": ["api.example.com"], "apps": [app], "fields": { "token": { "value": "s3cret", "secret": true } } })).await.unwrap();
    }
    let run = || a.app_schedules_call(&Scope::Global, "notes", &me.user, "vault", json!({}), "schedule");
    assert!(run().await.is_err(), "nothing runs notes' schedules yet");
    let body = json!({ "app": "notes", "device": one::keys::hex(&a.id()), "scope": "global" }).to_string();
    let set = async |path: &str| one::settings::answer(&a, "POST", path, body.as_bytes()).await.unwrap().unwrap();
    set("/api/devices/run_schedules").await;
    assert_eq!(run().await.unwrap(), json!(["For notes"]), "a scheduled run sees its own items, and no other app's");
    set("/api/devices/stop_schedules").await;
    assert!(run().await.is_err(), "stopped, they run nothing");
}

/// A client device's MCP request (the remote MCP server's, say) is answered as its user, never as the node's own: it
/// lists every tool any client does, its tools run as them, with what they may do and the keys they hand, and a tool
/// needing keys they didn't hand is answered 428 with what it needs, for them to hand and ask again.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_devices_mcp_runs_as_its_user() {
    use one::api::client::{InProcess, Via};
    let a = start("mcp").await;
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    eventually("notes to run", &a, async || a.call(notes("post", json!({ "text": "hi" }))).await.is_ok()).await;
    let device = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let_sam_in(&a, &device).await;
    // Asked as the node asks its routes for a request over iroh: as the device its connection proved.
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(one::protocol::header::READS, one::database::READS.into());
    let from = InProcess::new(a.clone(), Some(one::api::FromClient(device.verifying_key().to_bytes(), u64::MAX)), None, &headers);
    let from = std::sync::Arc::new(from);
    let mcp = async |body: Value| {
        let from = from.clone();
        tokio::task::spawn_blocking(move || {
            let answer = from.send("POST", "/mcp", &[("content-type", "application/json")], Some(body.to_string().into_bytes()), None).unwrap();
            (answer.status, serde_json::from_reader::<_, Value>(answer.body).unwrap())
        })
        .await
        .unwrap()
    };

    let (status, init) = mcp(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "clientInfo": { "name": "claude-ai" } } })).await;
    assert_eq!(status, 200, "{init}");
    let said = init["result"]["instructions"].as_str().unwrap();
    assert!(said.contains("You act for Sam") && !said.contains("You act for Andre"), "{said}");
    let (_, listed) = mcp(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).await;
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(["notes_post", "vault_list", "push_send", "logs", "devices_remove", "backups_now"].iter().all(|n| names.contains(n)), "every client lists the same tools: {names:?}");
    let (status, vault) = mcp(json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "vault_list", "arguments": {} } })).await;
    assert!(status == 200 && vault["result"]["isError"] != true, "Sam uses his own Vault: {vault}");
    let (_, logs) = mcp(json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "logs", "arguments": { "device": a.device_name() } } })).await;
    assert!(logs["result"]["isError"] == true && logs.to_string().contains("admin"), "Sam reads no log of Andre's device: {logs}");
    let (_, backups) = mcp(json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": "backups_status", "arguments": {} } })).await;
    assert!(backups["result"]["isError"] == true && backups.to_string().contains("Only admins"), "Backups are admins': {backups}");
    // A node signs who can do what only as its own user's device, for its own programs: a device answers that itself.
    let (_, devices) = mcp(json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": { "name": "devices_remove", "arguments": { "device": "mac", "confirm": true } } })).await;
    assert!(devices["result"]["isError"] == true, "{devices}");
    // The recovery key is used only by this machine's own programs: no device hands it to a node.
    let asked = from.clone();
    let recovered = tokio::task::spawn_blocking(move || asked.send("POST", "/api/devices/approve-with-recovery", &[("content-type", "application/json")], Some(json!({ "code": "ABCDEFGH", "recovery": "00".repeat(32) }).to_string().into_bytes()), None).unwrap().status).await.unwrap();
    assert_eq!(recovered, 403);
    let (status, needs) = mcp(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "notes_post", "arguments": { "text": "from Sam" } } })).await;
    assert_eq!(status, 428, "{needs}");
    assert!(needs["need"].as_array().is_some_and(|n| !n.is_empty()), "{needs}");
}

/// A tool that gives an `output` is held to it: an answer that keeps to it goes through, and one that doesn't (a wrong type,
/// a missing field, a field it doesn't list) fails, naming the tool, rather than drifting from what the API's document
/// says. A tool that says nothing of its answer is let through, and a deploy says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_tool_answers_what_its_output_says() {
    let a = start("answers").await;
    create(&a, "Home", "Andre").await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    let bundle = a.put_blob(&one::app::bundle::pack(&[("app.js".to_string(), include_bytes!("answers/app.js").to_vec())]).unwrap()).unwrap();
    let deployed = a.deploy_making(&me, "answers", &bundle, Some("admins"), true).await.unwrap();
    assert_eq!(deployed["lacking"], json!(["unsaid"]), "the deploy names the tool that says nothing of its answer: {deployed}");
    let schema = &one::openapi::document(&[], true)["paths"]["/api/deploy"]["post"]["responses"]["200"]["content"]["application/json"]["schema"];
    one::tools::check(schema, &deployed).unwrap_or_else(|err| panic!("a deploy answers what the document doesn't say: {err:#}\n{deployed}"));
    a.add(&me, "answers", &one::database::scopes::Scope::Global, true, None).await.unwrap();
    let call = |tool: &str| CallRequest { app: "answers".into(), tool: tool.into(), input: json!({}), ..Default::default() };
    eventually("answers to run", &a, async || a.call(call("keeps")).await.is_ok()).await;
    assert_eq!(a.call(call("keeps")).await.unwrap(), json!({ "n": 1 }));
    assert!(a.call(call("unsaid")).await.is_ok(), "a tool with no output is let through");
    for tool in ["wrong_type", "missing", "extra"] {
        let err = format!("{:#}", a.call(call(tool)).await.unwrap_err());
        assert!(err.contains(&format!("answers {tool} answered what its output schema doesn't say")), "{tool}: {err}");
    }
}

/// Every built-in app's tools say what they are and what they answer (a title and an `output`), the ones for its own
/// screens (`_`) aside, as the OpenAPI document and the agents that read them go by.
#[tokio::test(flavor = "multi_thread")]
async fn every_built_in_apps_tools_say_what_they_answer() {
    let a = start("built-in-outputs").await;
    create(&a, "Home", "Andre").await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    let mut lacking = Vec::new();
    let mut seen = 0;
    let all = a.catalog(&me, true).await.unwrap();
    for app in all.iter().filter(|app| app["built_in"].is_string()) {
        let name = app["name"].as_str().unwrap();
        let described = a.described(name).await.unwrap_or_else(|err| panic!("{name}: {err:#}"));
        seen += described["tools"].as_array().map_or(0, Vec::len);
        lacking.extend(one::app::deploy::lacking(&described).into_iter().map(|tool| format!("{name}_{tool}")));
    }
    assert!(seen > 150, "the built-in apps' tools were found: {seen}");
    assert!(lacking.is_empty(), "these tools give no title or no output: {}", lacking.join(", "));
}

/// What the OpenAPI document says a route answers is what it answers: each JSON answer is held to its schema, as a tool's
/// is to its output.
#[tokio::test(flavor = "multi_thread")]
async fn a_routes_answer_is_what_the_document_says() {
    use one::api::client::{InProcess, Via};
    let a = start("route-answers").await;
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    eventually("notes to run", &a, async || a.call(notes("post", json!({ "text": "hi" }))).await.is_ok()).await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    let document = one::openapi::document(&a.apps(&me).await.unwrap(), true);
    // As this machine's own programs ask: over its loopback, with the token in the node's home.
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(one::protocol::header::LOCAL_TOKEN, one::api::local_token(a.dir()).unwrap().parse().unwrap());
    let from = std::sync::Arc::new(InProcess::new(a.clone(), None, Some(axum::extract::ConnectInfo("127.0.0.1:1".parse().unwrap())), &headers));
    let mut asked = 0;
    for (method, path, body) in [
        ("GET", "/api/state", None),
        ("GET", "/api/apps", None),
        ("GET", "/api/labs", None),
        ("GET", "/api/widgets", None),
        ("GET", "/api/badges", None),
        ("GET", "/api/vault", None),
        ("GET", "/api/vault/requests", None),
        ("POST", "/api/call", Some(json!({ "app": "notes", "tool": "read", "input": {} }))),
    ] {
        let schema = document["paths"][path][method.to_lowercase()]["responses"]["200"]["content"]["application/json"]["schema"].clone();
        assert!(schema.is_object(), "{method} {path} says what it answers");
        let from = from.clone();
        let answer = tokio::task::spawn_blocking(move || {
            let body = body.map(|b: Value| b.to_string().into_bytes());
            let answer = from.send(method, path, &[("content-type", "application/json")], body, None).unwrap();
            (answer.status, serde_json::from_reader::<_, Value>(answer.body).unwrap())
        })
        .await
        .unwrap();
        assert_eq!(answer.0, 200, "{method} {path}: {}", answer.1);
        one::tools::check(&schema, &answer.1).unwrap_or_else(|err| panic!("{method} {path} answered what the document doesn't say: {err:#}\n{}", answer.1));
        asked += 1;
    }
    assert_eq!(asked, 8);
}

/// `oned apps` as a device's app runtime, a process of its own key, its own home's.
struct Runtime(std::process::Child);

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Asks the app runtime at `at` as one of its device's own programs would, with `token`. → status, answer.
async fn ask_runtime(at: &str, token: &str, body: Value) -> (u16, Value) {
    let (url, token) = (format!("http://{at}/api/call"), token.to_string());
    tokio::task::spawn_blocking(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        match agent.post(&url).header(one::protocol::header::LOCAL_TOKEN, &token).send_json(body) {
            Ok(mut answer) => (answer.status().as_u16(), answer.body_mut().read_json().unwrap_or(Value::Null)),
            Err(err) => (0, json!(err.to_string())),
        }
    })
    .await
    .unwrap()
}

/// A device's app runtime, an endpoint of its own key listed under it, runs what the device ran: its own programs'
/// calls on its loopback, with the token in its folder alone, opening the device's scopes with what the device's key
/// (held by the device, on its loopback) opens; a call it passes on to another of its user's devices; and the
/// device's app schedules, once the device leaves them to it.
#[tokio::test(flavor = "multi_thread")]
async fn a_devices_app_runtime_of_its_own_key_runs_its_calls_and_schedules() {
    use one::database::content::{self, Change};
    use one::database::scopes::Kind;
    let k = start("rt-device").await;
    create(&k, "Home", "Andre").await;
    add_notes(&k).await;
    let _relay = relay(&k).await;
    let b = start("rt-other").await;
    b.join(&k.pair().unwrap()).await.unwrap();
    let me = one::node::Caller::own(k.id(), k.user().unwrap());

    // The device's own key, held on its loopback, where its own programs ask with the token in its home.
    let holder = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let holder_at = holder.local_addr().unwrap().to_string();
    tokio::spawn(one::api::serve(holder, one::api::router(k.clone())));

    // Its runtime: a key of its own in its folder, in a home with the device's network and token, listed by the
    // device as a compute naming it, let in at the relay; then the device drops its own Compute.
    let home = crate::temp("rt-runtime");
    let key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let r = one::keys::Endpoint::of(&key);
    let folder = one::node::runtime_folder(&home);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("key"), one::keys::hex(&key.to_bytes())).unwrap();
    std::fs::create_dir_all(home.join("network")).unwrap();
    std::fs::copy(k.dir().join("network/network.json"), home.join("network/network.json")).unwrap();
    std::fs::copy(k.dir().join(one::api::LOCAL_TOKEN), home.join(one::api::LOCAL_TOKEN)).unwrap();
    // The device runs its user's app schedules of notes, for everyone.
    let ran = one::settings::answer(&k, "POST", "/api/devices/run_schedules", json!({ "app": "notes", "device": one::keys::hex(&k.id()), "scope": "global" }).to_string().as_bytes()).await.unwrap().unwrap();
    assert!(ran["until"].is_u64(), "{ran}");
    let listing = |key: &[u8; 32], seal: &[u8; 32], name: &str, roles: &[Kind], device: Option<&[u8; 32]>| {
        content::node(Change::Device, json!({ "key": one::keys::hex(key), "seal": one::keys::hex(seal), "user": me.user, "name": name, "roles": roles, "device": device.map(one::keys::hex) }))
    };
    k.member_content(&me, listing(&r.key, &r.seal, "rt-device's runtime", &[Kind::Compute], Some(&k.id()))).await.unwrap();
    one::settings::tell(&k, &k, json!({ "add": one::keys::hex(&r.key), "admin": false, "role": "compute" })).await.unwrap();
    k.member_content(&me, listing(&k.id(), &one::settings::Me::seal(&k), "rt-device", &[Kind::Client], None)).await.unwrap();

    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let at = format!("127.0.0.1:{port}");
    let log = std::fs::File::create(home.join("runtime.log")).unwrap();
    let mut oned = std::process::Command::new(env!("CARGO_BIN_EXE_one"));
    std::os::unix::process::CommandExt::arg0(&mut oned, one::oned::name());
    oned.args([one::database::scopes::APPS_PART, &format!("--listen={at}")]);
    oned.env(one::config::env("HOME"), &home).env(one::config::env("ADDR"), &holder_at).stdout(log.try_clone().unwrap()).stderr(log);
    let _runtime = Runtime(oned.spawn().unwrap());

    // Only the device's own programs ask it, with the token in its folder, never the one in the device's home.
    let log = home.join("runtime.log");
    let started = async || std::fs::exists(folder.join(one::api::LOCAL_TOKEN)).unwrap() && ask_runtime(&at, "", json!({})).await.0 == 403;
    polled("the runtime to start", &log, started).await;
    let token = std::fs::read_to_string(folder.join(one::api::LOCAL_TOKEN)).unwrap();
    let home_token = std::fs::read_to_string(k.dir().join(one::api::LOCAL_TOKEN)).unwrap();
    assert_eq!(ask_runtime(&at, &home_token, json!({ "app": "notes", "tool": "read", "input": {} })).await.0, 403);
    let post = json!({ "app": "notes", "tool": "post", "input": { "text": "from the runtime" } });
    polled("the runtime to run a call", &log, async || ask_runtime(&at, &token, post.clone()).await.0 == 200).await;
    let lines = async |node: &one::node::Node| node.call(notes("read", json!({}))).await.map(|v| v.to_string()).unwrap_or_default();
    eventually("its write to reach another device", &b, async || lines(&b).await.contains("from the runtime")).await;

    // A call it passes on to another of its user's devices, as its device would.
    let there = ask_runtime(&at, &token, json!({ "app": "notes", "tool": "read", "input": {}, "device": "rt-other" })).await;
    assert_eq!(there.0, 200, "{}", there.1);
    assert!(there.1.to_string().contains("from the runtime"), "{}", there.1);

    // The device's schedules, run by its runtime alone, the device no longer listed to run anything.
    eventually("a scheduled run", &b, async || lines(&b).await.contains("on schedule")).await;
    let slots = k.dir().join("network/bucket/global").join(one::database::scopes::APP).join("slots/notes/tick");
    let taken: Vec<String> = std::fs::read_dir(slots).unwrap().map(|f| std::fs::read_to_string(f.unwrap().path()).unwrap()).collect();
    assert_eq!(taken, vec![one::keys::hex(&r.key)], "each slot's lease names who took it");
}

/// A computer's onet holds its device's key for the device's app runtime, an endpoint of its own key: the runtime asks
/// it for what that key opens, and onet asks the runtime, its own, with the token in the runtime's folder, which the
/// home's token isn't. A call made on onet runs there, and reaches the user's other devices.
#[tokio::test(flavor = "multi_thread")]
async fn a_computers_onet_holds_its_device_key_for_its_own_runtime() {
    use one::database::content::{self, Change};
    use one::database::scopes::Kind;
    let k = start("onet-device").await;
    create(&k, "Home", "Andre").await;
    add_notes(&k).await;
    let _relay = relay(&k).await;
    let b = start("onet-other").await;
    b.join(&k.pair().unwrap()).await.unwrap();
    let me = one::node::Caller::own(k.id(), k.user().unwrap());

    // The device's home as onet and its runtime share it: its key, network and token; the runtime's key in its folder,
    // listed by the device as a compute naming it, let in at the relay; then the device drops its own Compute.
    let home = crate::temp("onet-home");
    for file in ["device.json", one::api::LOCAL_TOKEN, "network/network.json"] {
        std::fs::create_dir_all(home.join(file).parent().unwrap()).unwrap();
        std::fs::copy(k.dir().join(file), home.join(file)).unwrap();
    }
    let key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let r = one::keys::Endpoint::of(&key);
    let folder = one::node::runtime_folder(&home);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("key"), one::keys::hex(&key.to_bytes())).unwrap();
    let listing = |key: &[u8; 32], seal: &[u8; 32], name: &str, roles: &[Kind], device: Option<&[u8; 32]>| {
        content::node(Change::Device, json!({ "key": one::keys::hex(key), "seal": one::keys::hex(seal), "user": me.user, "name": name, "roles": roles, "device": device.map(one::keys::hex) }))
    };
    k.member_content(&me, listing(&r.key, &r.seal, "onet-device's runtime", &[Kind::Compute], Some(&k.id()))).await.unwrap();
    one::settings::tell(&k, &k, json!({ "add": one::keys::hex(&r.key), "admin": false, "role": "compute" })).await.unwrap();
    k.member_content(&me, listing(&k.id(), &one::settings::Me::seal(&k), "onet-device", &[Kind::Client], None)).await.unwrap();

    let free = || format!("127.0.0.1:{}", std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port());
    let (onet_at, runtime_at) = (free(), free());
    let started = |name: String, args: Vec<String>, addr: &str, log: &str| {
        let log = std::fs::File::create(home.join(log)).unwrap();
        let mut program = std::process::Command::new(env!("CARGO_BIN_EXE_one"));
        std::os::unix::process::CommandExt::arg0(&mut program, name);
        program.args(args).env(one::config::env("HOME"), &home).env(one::config::env("ADDR"), addr).stdout(log.try_clone().unwrap()).stderr(log);
        Runtime(program.spawn().unwrap())
    };
    let _onet = started(onet::computer::name(), vec![format!("--listen={onet_at}"), format!("--runtime={runtime_at}")], &onet_at, "onet.log");
    let home_token = std::fs::read_to_string(home.join(one::api::LOCAL_TOKEN)).unwrap();
    let held = async || ask(&onet_at, "GET", "/api/holds", &home_token, None).await;
    polled("onet to answer", &home.join("onet.log"), async || held().await.0 == 200).await;
    assert_eq!(held().await.1["key"], one::keys::hex(&k.id()), "it holds the device's key");
    let _runtime = started(one::oned::name(), vec![one::database::scopes::APPS_PART.into(), format!("--listen={runtime_at}")], &onet_at, "runtime.log");
    polled("the runtime to start", &home.join("runtime.log"), async || ask(&runtime_at, "GET", "/api/state", "", None).await.0 == 403).await;
    assert_eq!(ask(&runtime_at, "GET", "/api/state", &home_token, None).await.0, 403, "the runtime takes only its own token");

    let post = json!({ "app": "notes", "tool": "post", "input": { "text": "through onet" } });
    // Its own runtime takes it, asked on its loopback, named by its device, once it has read what it holds: till then it
    // passes it on.
    polled("a call through onet that its own runtime runs", &home.join("runtime.log"), async || {
        let (status, _, ran) = ask(&onet_at, "POST", "/api/call", &home_token, Some(post.clone())).await;
        status == 200 && ran["steps"].as_array().into_iter().flatten().any(|s| s["here"] == true && s["result"] == "ran" && s["device"] == "onet-device")
    })
    .await;
    let lines = async |node: &one::node::Node| node.call(notes("read", json!({}))).await.map(|v| v.to_string()).unwrap_or_default();
    eventually("its write to reach another device", &b, async || lines(&b).await.contains("through onet")).await;
}

/// oned, started by a test as a computer's supervisor would: asked to stop as it is (SIGTERM), so it stops what it started.
struct Oned(std::process::Child);

impl Drop for Oned {
    fn drop(&mut self) {
        unsafe { libc::kill(self.0.id() as i32, libc::SIGTERM) };
        let _ = self.0.wait();
    }
}

/// A computer's home as a node that was its own device left it, started by this build's oned: its onet holds the same
/// key and its app runtime opens what that key opened, and the only steps written are the two listings that make the
/// runtime the device's, a compute of its own key naming the device, and the device a client alone.
#[tokio::test(flavor = "multi_thread")]
async fn a_computers_home_from_before_onet_is_the_same_device_with_its_runtime_listed() {
    use one::database::scopes::Kind;
    let k = start("switch-device").await;
    create(&k, "Home", "Andre").await;
    add_notes(&k).await;
    k.call(notes("post", json!({ "text": "before onet" }))).await.unwrap();
    let me = one::node::Caller::own(k.id(), k.user().unwrap());
    let before = k.global_steps(&me, 0, None).await.unwrap()["at"].as_u64().unwrap();

    // Its home, on a computer of its own: its bucket the folder in it.
    let home = crate::temp("switch-home");
    assert!(std::process::Command::new("cp").arg("-r").arg(k.dir().join(".")).arg(&home).status().unwrap().success());
    let mut network: Value = serde_json::from_slice(&std::fs::read(home.join("network/network.json")).unwrap()).unwrap();
    network["bucket"]["path"] = json!(home.join("network/bucket").display().to_string());
    std::fs::write(home.join("network/network.json"), network.to_string()).unwrap();
    let key = std::fs::read(home.join("device.json")).unwrap();
    assert!(!one::node::runtime_folder(&home).join("key").exists(), "a home from before onet has no runtime key");

    let at = format!("127.0.0.1:{}", std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port());
    let log = std::fs::File::create(home.join("oned.log")).unwrap();
    let mut oned = std::process::Command::new(env!("CARGO_BIN_EXE_one"));
    std::os::unix::process::CommandExt::arg0(&mut oned, one::oned::name());
    oned.env(one::config::env("HOME"), &home).env(one::config::env("ADDR"), &at).stdout(log.try_clone().unwrap()).stderr(log);
    let _oned = Oned(oned.spawn().unwrap());
    let token = std::fs::read_to_string(home.join(one::api::LOCAL_TOKEN)).unwrap();

    // What it read before, through onet, from its runtime opening the epochs sealed to its key.
    let read = json!({ "app": "notes", "tool": "read", "input": {} });
    let mut lines = Value::Null;
    polled("a call through the new onet", &home.join("oned.log"), async || {
        let (status, answer, _) = ask(&at, "POST", "/api/call", &token, Some(read.clone())).await;
        lines = answer;
        status == 200
    })
    .await;
    assert!(lines.to_string().contains("before onet"), "{lines}");
    assert_eq!(std::fs::read(home.join("device.json")).unwrap(), key, "its key is the one it had");
    assert_eq!(ask(&at, "GET", "/api/holds", &token, None).await.1["key"], one::keys::hex(&k.id()), "onet holds it");

    // Who's who: the two listings, and nothing else.
    let runtime = one::node::runtime_endpoint(&home).unwrap();
    let listed = async || {
        let state = ask(&at, "GET", "/api/state", &token, None).await.1;
        let devices = state["network"]["devices"].as_array().cloned().unwrap_or_default().into_iter().chain(state["network"]["clients"].as_array().cloned().unwrap_or_default());
        let roles = |key: &[u8; 32]| devices.clone().filter(|d| d["key"] == one::keys::hex(key)).flat_map(|d| d["roles"].as_array().cloned().unwrap_or_default()).collect::<Vec<_>>();
        (roles(&k.id()), roles(&runtime.key))
    };
    polled("the runtime listed", &home.join("oned.log"), async || listed().await == (vec![json!(Kind::Client)], vec![json!(Kind::Compute)])).await;
    let after = ask(&at, "GET", &format!("/api/steps/global?from={before}"), &token, None).await.1;
    assert_eq!((after["at"].as_u64(), after["steps"].as_array().map(Vec::len)), (Some(before + 2), Some(2)), "no step but the two listings: {after}");
}

/// Asks `at` as one of a computer's own programs would, with `token`, for a trace too. → status, answer, and the trace
/// of whoever answered, if it said.
async fn ask(at: &str, method: &str, path: &str, token: &str, body: Option<Value>) -> (u16, Value, Value) {
    let (url, method, token) = (format!("http://{at}{path}"), method.to_string(), token.to_string());
    tokio::task::spawn_blocking(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let request = ureq::http::Request::builder().method(method.as_str()).uri(&url).header(one::protocol::header::LOCAL_TOKEN, &token).header(one::protocol::header::TRACE, "1").header("content-type", "application/json");
        match agent.run(request.body(body.map(|b| b.to_string()).unwrap_or_default()).unwrap()) {
            Ok(mut answer) => {
                let trace = answer.headers().get(one::protocol::header::TRACE).and_then(|t| data_encoding::BASE64.decode(t.as_bytes()).ok()).and_then(|t| serde_json::from_slice(&t).ok()).unwrap_or_default();
                (answer.status().as_u16(), answer.body_mut().read_json().unwrap_or(Value::Null), trace)
            }
            Err(err) => (0, json!(err.to_string()), Value::Null),
        }
    })
    .await
    .unwrap()
}

/// Waits for what's expected of a process of its own, which tells this one nothing as it goes, so it's asked again
/// every so often; failing, says what it logged.
async fn polled(what: &str, log: &std::path::Path, mut check: impl AsyncFnMut() -> bool) {
    for _ in 0..600 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("timed out waiting for {what}: {}", std::fs::read_to_string(log).unwrap_or_default());
}
