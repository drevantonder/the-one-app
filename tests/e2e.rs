//! Real nodes on this machine, reaching each other through the relay at a real network's gateway.

use std::time::{Duration, Instant};

use one::node::{CallRequest, Node};
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

/// Waits for what's expected, as long as a busy machine may take: a limit only for what never happens.
pub(crate) async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(180) {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

/// `content` written to `global`'s scope database as a step `caller`'s own device (`key`) signs, as a client does.
pub(crate) async fn signed_by(a: &Node, caller: &one::node::Caller, key: &ed25519_dalek::SigningKey, content: one::database::content::Content) {
    let global = one::database::scopes::Scope::Global;
    let next = a.global_steps(caller, one::database::steps::NUMBER_MAX, None).await.unwrap();
    let (at, epoch) = (next["at"].as_u64().unwrap(), next["epoch"].as_u64().unwrap() as u32);
    let step = one::database::steps::sign(key, &global, at + 1, &caller.user, epoch, one::node::now_ms(), content).unwrap();
    a.put_signed_steps(caller, vec![step]).await.unwrap();
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
    a.install(&me, "notes", &one::database::scopes::Scope::Global, true, None).await.unwrap();
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
    eventually("notes to run", async || a.call(notes("post", json!({ "text": "before the backup" }))).await.is_ok()).await;
    a.set_item(&me, &one::database::scopes::Scope::User(me.user.clone()), json!({ "title": "Example key", "fields": { "token": { "value": "s3cret-value", "secret": true } }, "hosts": ["api.example.com"] })).await.unwrap();

    let bad = a.add_place(&me, &json!({ "endpoint": "http://127.0.0.1:9", "bucket": "b", "key_id": "k", "secret": "s" })).await.unwrap();
    assert!(bad["id"].is_null() && bad["tried"][0]["error"].is_string(), "{bad}");
    assert!(a.backups().unwrap()["places"].as_array().unwrap().is_empty());

    let dir = temp("backup-disk").join("One Backups");
    let added = a.add_place(&me, &json!({ "path": dir, "title": "T7 Shield" })).await.unwrap();
    assert!(added["id"].is_string() && added["tried"].as_array().unwrap().iter().all(|s| s["error"].is_null()), "{added}");
    eventually("the first backup", async || !a.backups().unwrap()["places"][0]["last"].is_null()).await;
    let shown = a.backups().unwrap();
    assert_eq!(shown["places"][0]["title"], "T7 Shield");
    assert_eq!(shown["places"][0]["last"]["device"], "backup-a", "the folder's own device backs it up: {shown}");
    let first = shown["places"][0]["last"]["time"].clone();
    eventually("b to see it", async || !b.backups().unwrap()["places"][0]["last"].is_null()).await;

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
    assert_eq!(checked["people"], json!(["Andre"]));
    assert!(checked["apps"].as_array().unwrap().contains(&json!("notes")), "{checked}");
    let mine = format!("user:{}", me.user);
    let items = backed_up(&dir, &network, &one::keys::unhex(&recovery).unwrap(), &mine, "node", "vault", "select item from items").await;
    assert!(items.iter().any(|i| i.contains("\"Example key\"")), "the Vault's item came back: {items:?}");
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
    eventually("the second backup", async || a.backups().unwrap()["places"][0]["last"]["time"] != first).await;
    let again = one::backups::restore(&place, &network, "global", &keys["global"]).unwrap();
    assert!(again.snapshot.databases["notes/main"].0 > steps_then);
    assert_eq!(backed_up_posts(&dir, &network, &one::keys::unhex(&recovery).unwrap()).await, ["before the backup", "after the backup"]);
    let checked = a.check_backups().await.unwrap();
    assert_eq!(checked[0]["place"], "T7 Shield", "{checked}");
    assert!(checked[0]["error"].is_null(), "{checked}");
    let shown = a.backups().unwrap();
    let moment = &shown["snapshots"][0];
    assert!(moment["scopes"].as_array().unwrap().contains(&json!("Everyone")) && moment["places"] == json!(["T7 Shield"]), "one snapshot of the network: {shown}");
    assert!(a.state()["backups"].is_null(), "no badge while backups work");

    // A disk that breaks: admins see what's wrong and its fix, on Settings, until a run works again.
    let copy = dir.join(&network).join("bucket");
    std::fs::rename(&copy, dir.join("bucket-aside")).unwrap();
    std::fs::write(&copy, b"not a folder").unwrap();
    // Its failure shows once it's done: nothing writes to the disk until the next run.
    a.call(notes("post", json!({ "text": "broken" }))).await.unwrap();
    a.back_up_now();
    eventually("a failed run", async || a.backups().unwrap()["places"][0]["problem"]["text"].as_str().is_some_and(|t| t.contains("couldn't be used") || t.contains("won't let"))).await;
    let problem = a.state()["backups"].clone();
    assert_eq!(problem["fix"], "Check the disk on backup-a is plugged in and has room.", "{problem}");
    assert!(problem["text"].as_str().unwrap().starts_with("T7 Shield "), "{problem}");
    std::fs::remove_file(&copy).unwrap();
    std::fs::rename(dir.join("bucket-aside"), &copy).unwrap();
    a.call(notes("post", json!({ "text": "fixed" }))).await.unwrap();
    a.back_up_now();
    eventually("the badge to clear", async || a.state()["backups"].is_null()).await;

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
    eventually("the planted folder's run to fail", async || place("Planted").is_some_and(|p| !p["problem"].is_null())).await;
    assert!(!planted.exists(), "nothing was written there");
    assert!(place("Unnamed bucket").is_none(), "{}", a.backups().unwrap());

    // A place typed into the sheet on the person's device is a place too.
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

/// A device is approved only by its person's own device, or a newcomer's by the admin whose invite their proof was made
/// with; a device removed opens no epoch made after.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_persons_own_device_or_their_inviter_lets_a_device_in() {
    use one::node::{DeviceRequest, Invited};
    let dir = temp("approve");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::keys::hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    eventually("notes to run", async || a.call(notes("post", json!({ "text": "hi" }))).await.is_ok()).await;
    let global = one::database::scopes::Scope::Global;
    let new_key = || ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let asks = |key: &ed25519_dalek::SigningKey, user: &str, proof: Option<String>| {
        let e = one::keys::Endpoint::of(key);
        DeviceRequest { key: one::keys::hex(&e.key), seal: one::keys::hex(&e.seal), name: "Chrome".into(), user: user.into(), person: String::new(), proof, device: None, made: None, place: None, sign_in: None }
    };
    let opens = async |key: &ed25519_dalek::SigningKey, scope: &one::database::scopes::Scope| -> Vec<u32> {
        let got = a.sealed_keys(&me, scope, Some(&one::keys::hex(&key.verifying_key().to_bytes())), None).await.unwrap();
        let sealed = got["sealed"].as_object().unwrap();
        sealed.iter().filter(|(_, v)| one::keys::open_sealed(&one::keys::seal_secret(key), &data_encoding::BASE64URL_NOPAD.decode(v.as_str().unwrap().as_bytes()).unwrap()).is_ok()).map(|(n, _)| n.parse().unwrap()).collect()
    };

    // Andre's new laptop, approved from his machine, opens global.
    let laptop = new_key();
    a.approve_request(&me, &asks(&laptop, &me.user, None), None).await.unwrap();
    assert_eq!(opens(&laptop, &global).await, vec![1]);

    // Sam is let in only with the proof of the secret Andre's invite made.
    let secret = "the-invites-secret";
    let sam = Invited { secret: secret.into(), name: "Sam".into(), user: one::database::scopes::new_id(), role: one::database::scopes::Role::Member, until: u64::MAX };
    let sams = new_key();
    let wrong = one::keys::invite_proof(b"another-secret", &one::keys::Endpoint::of(&sams));
    assert!(a.approve_request(&me, &asks(&sams, &sam.user, Some(wrong)), Some(&sam)).await.is_err());
    assert!(!a.is_user(&sam.user), "a request whose proof doesn't match lets no one in");
    let right = one::keys::invite_proof(secret.as_bytes(), &one::keys::Endpoint::of(&sams));
    a.approve_request(&me, &asks(&sams, &sam.user, Some(right)), Some(&sam)).await.unwrap();
    assert!(a.is_user(&sam.user));
    assert_eq!(opens(&sams, &global).await, vec![1]);

    // Sam's next device is his to approve, not Andre's.
    let refused = a.approve_request(&me, &asks(&new_key(), &sam.user, None), None).await.unwrap_err();
    assert!(format!("{refused:#}").contains("one of their own devices"), "{refused:#}");
    // Nor does Andre's machine approve anything for Sam, asked from Sam's device over the network: it isn't Sam's.
    let from_sam = one::node::Caller { key: sams.verifying_key().to_bytes(), user: sam.user.clone(), agent: None, credential: one::node::Credential::SignIn("Sam's".into()) };
    let refused = a.approve(&from_sam, "ABCDEFGH", true).await.unwrap_err();
    assert!(format!("{refused:#}").contains("isn't one of your devices"), "{refused:#}");

    // The laptop, removed, opens no epoch made after; Andre's machine and Sam's device do, and the notes go on.
    let removed = a.remove_device(&me, &one::keys::hex(&laptop.verifying_key().to_bytes())).await.unwrap();
    assert!(removed["new_epochs"].as_array().unwrap().iter().any(|e| e["scope"] == "global" && e["epoch"] == 2), "{removed}");
    assert_eq!(opens(&laptop, &global).await, vec![1]);
    for f in walk(&dir).into_iter().filter(|p| p.to_string_lossy().contains("/_epochs/2/")) {
        assert!(one::keys::open_sealed(&one::keys::seal_secret(&laptop), &std::fs::read(&f).unwrap()).is_err(), "{} opens for the removed laptop", f.display());
    }
    assert_eq!(opens(&sams, &global).await, vec![1, 2]);
    a.call(notes("post", json!({ "text": "after" }))).await.unwrap();
}

/// A file put from elsewhere (a client device's, a sign-in's) is read back by whoever put it alone, or by someone who
/// may use a scope that names it: knowing its hash isn't enough. A file put to answer one request (git's push, to
/// Repos' `_http`) goes once it's answered, unless something keeps it.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_put_from_elsewhere_is_read_only_by_whoever_put_it() {
    let a = start("put-from").await;
    create(&a, "Home", "Andre").await;
    let caller = |user: &str| one::node::Caller { key: a.id(), user: user.into(), agent: None, credential: one::node::Credential::SignIn(format!("{user}'s")) };
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
    eventually("notes to run", async || a.call(notes("post", json!({ "text": "a private thought" }))).await.is_ok()).await;
    let t = trace::Trace::new();
    let read = trace::within(Some(t.clone()), a.call(notes("read", json!({ "format": "json" })))).await.unwrap();
    assert!(read.to_string().contains("a private thought"), "{read}");
    let said = t.done(&a.device_name()).to_string();
    assert!(said.contains(r#""step":"device""#) && said.contains(r#""op":"read""#) && said.contains(r#""step":"run""#), "{said}");
    assert!(!said.contains("private thought") && !said.contains(&a.user().unwrap()), "a trace holds names and timings alone: {said}");
}

/// A device's log is read over the network by whoever may remove the device (its person, and the network's admins), never
/// by another member, and what's returned is capped.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_devices_person_or_an_admin_reads_its_logs() {
    use one::node::Invited;
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
    let sam = Invited { secret: "s".into(), name: "Sam".into(), user: one::database::scopes::new_id(), role: one::database::scopes::Role::Member, until: u64::MAX };
    let key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let e = one::keys::Endpoint::of(&key);
    let request = one::node::DeviceRequest { key: one::keys::hex(&e.key), seal: one::keys::hex(&e.seal), name: "Sam's".into(), user: sam.user.clone(), person: String::new(), proof: Some(one::keys::invite_proof(b"s", &e)), device: None, made: None, place: None, sign_in: None };
    a.approve_request(&andre, &request, Some(&sam)).await.unwrap();
    let from_sam = one::node::Caller::own(a.id(), sam.user.clone());
    let refused = a.call_as(logs(json!({})), from_sam.clone()).await.unwrap_err();
    assert!(format!("{refused:#}").contains("reads its logs"), "{refused:#}");
    assert!(!a.may_read_logs(&sam.user).unwrap() && a.may_read_logs(&andre.user).unwrap());
    a.member_as(&andre, "role", json!({ "user": sam.user, "role": one::database::scopes::Role::Admin })).await.unwrap();
    assert!(a.may_read_logs(&sam.user).unwrap(), "an admin of the network reads any device's logs");
}

/// A node runs a build of an app's service only if its app's latest deploy names it: one whose bytes aren't that hash's
/// is refused, a deploy naming none for this platform runs none, and only someone who may deploy the app names one.
#[tokio::test(flavor = "multi_thread")]
async fn a_service_runs_only_the_build_its_apps_deploy_names() {
    use one::node::Build;
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
    let sam = one::node::Caller { key: a.id(), user: "sam".into(), agent: None, credential: one::node::Credential::SignIn("Sam's".into()) };
    assert!(a.deploy(&sam, "tiny", &named, false).await.is_err());
    // Nor does Andre, from elsewhere, name a build he can't read: one Sam put, by its hash.
    let away = one::node::Caller { key: [7; 32], credential: one::node::Credential::SignIn("Andre's".into()), ..me.clone() };
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

/// A client device's MCP request (the remote MCP server's, say) is answered as its person, never as the node's own: its
/// tools run as them, with the keys they hand, and a tool needing keys they didn't hand is answered 428 with what it
/// needs, for them to hand and ask again.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_devices_mcp_runs_as_its_person() {
    use one::api::client::{InProcess, Via};
    let a = start("mcp").await;
    create(&a, "Home", "Andre").await;
    add_notes(&a).await;
    let me = one::node::Caller::own(a.id(), a.user().unwrap());
    eventually("notes to run", async || a.call(notes("post", json!({ "text": "hi" }))).await.is_ok()).await;
    let secret = "the-invites-secret";
    let sam = one::node::Invited { secret: secret.into(), name: "Sam".into(), user: one::database::scopes::new_id(), role: one::database::scopes::Role::Member, until: u64::MAX };
    let device = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let e = one::keys::Endpoint::of(&device);
    let asks = one::node::DeviceRequest { key: one::keys::hex(&e.key), seal: one::keys::hex(&e.seal), name: "Chrome".into(), user: sam.user.clone(), person: String::new(), proof: Some(one::keys::invite_proof(secret.as_bytes(), &e)), device: None, made: None, place: None, sign_in: None };
    a.approve_request(&me, &asks, Some(&sam)).await.unwrap();
    // Asked as the node asks its routes for a request over iroh: as the device its connection proved.
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(one::database::HEADER, one::database::READS.into());
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
    assert!(names.contains(&"notes_post") && !names.contains(&"logs"), "a sign-in from elsewhere reads no device's log: {names:?}");
    let (status, needs) = mcp(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "notes_post", "arguments": { "text": "from Sam" } } })).await;
    assert_eq!(status, 428, "{needs}");
    assert!(needs["need"].as_array().is_some_and(|n| !n.is_empty()), "{needs}");
}
