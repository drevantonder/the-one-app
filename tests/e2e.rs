//! Real nodes on this machine, reaching each other through the relay of a real kernel's gateway.

use std::time::{Duration, Instant};

use one::node::{CallRequest, Node};
use serde_json::{Value, json};

use crate::temp;

async fn start(name: &str) -> Node {
    let dir = temp(name);
    std::fs::write(dir.join("device.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": name }).to_string()).unwrap();
    Node::start(&dir, one::sidecars::built_in()).await.unwrap()
}

/// A kernel whose gateway's relay the devices of `a`'s install reach each other through, taking only `a` and those it
/// admits: `a` says it's the install's.
async fn relay(a: &Node) -> crate::kernel::Kernel {
    let k = tokio::task::block_in_place(|| crate::kernel::Kernel::for_devices(vec![one::entry::hex(&a.id())]));
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    a.set_setting(&me, "address", Some(&k.address)).await.unwrap();
    k
}

/// Waits for what's expected, as long as a busy machine may take: a limit only for what never happens.
async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(180) {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

fn board(op: &str, input: Value) -> CallRequest {
    CallRequest { app: "board".into(), op: op.into(), input, ..Default::default() }
}

#[tokio::test(flavor = "multi_thread")]
async fn backups_go_to_a_folder_encrypted_and_restore_with_the_recovery_key_alone() {
    let a = start("backup-a").await;
    let b = start("backup-b").await;
    let recovery = a.create("Home", "Andre").await.unwrap();
    let _relay = relay(&a).await;
    b.join(&a.pair().unwrap()).await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    eventually("the board to run", async || a.call(board("post", json!({ "text": "before the backup" }))).await.is_ok()).await;
    a.set_item(&me, &one::entry::Scope::User(me.user.clone()), json!({ "title": "Example key", "fields": { "token": { "value": "s3cret-value", "secret": true } }, "hosts": ["api.example.com"] })).await.unwrap();

    // A place that can't be written isn't added.
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

    // The folder's device restores it into a throwaway copy, against what it holds: entries, databases and blobs.
    let report = a.check_backups().await.unwrap();
    assert!(report[0]["error"].is_null() && !report[0]["scopes"].as_array().unwrap().is_empty(), "{report}");
    assert!(a.backups().unwrap()["places"][0]["checked"]["error"].is_null(), "and says it passed");

    // From nothing but the disk and the recovery key.
    let place = one::bucket::Place::Folder(dir.clone());
    let install = one::backups::installs(&place).unwrap().remove(0);
    let keys = one::backups::open_keys(&place, &install, &one::entry::unhex(&recovery).unwrap()).unwrap();
    assert!(keys.contains_key("global") && keys.contains_key(&format!("user:{}", me.user)), "{:?}", keys.keys());
    let checked = one::backups::check(&one::backups::Check { place: &place, install: &install, keys: &keys, root: None, live: None }).unwrap().0;
    assert_eq!(checked["problems"], json!([]), "{checked}");
    assert_eq!(checked["install"], "Home");
    assert_eq!(checked["people"], json!(["Andre"]));
    assert!(checked["apps"].as_array().unwrap().contains(&json!("board")), "{checked}");
    let mine = format!("user:{}", me.user);
    let items = backed_up(&dir, &install, &one::entry::unhex(&recovery).unwrap(), &mine, "node", "vault", "select item from items").await;
    assert!(items.iter().any(|i| i.contains("\"Example key\"")), "the Vault's item came back: {items:?}");
    let restored = one::backups::restore(&place, &install, "global", &keys["global"]).unwrap();
    assert!(restored.entries.iter().any(|e| e.content.kind == "genesis"));
    assert_eq!(backed_up_posts(&dir, &install, &one::entry::unhex(&recovery).unwrap()).await, ["before the backup"]);
    let steps_then = restored.snapshot.databases["board/main"].0;
    let files: Vec<Vec<u8>> = walk(&dir).iter().map(|f| std::fs::read(f).unwrap()).collect();
    assert!(!files.iter().any(|f| f.windows(12).any(|w| w == b"s3cret-value") || f.windows(17).any(|w| w == b"before the backup")), "nothing is readable on the disk");

    // A change is backed up on the next run; only what's new is sent.
    a.call(board("post", json!({ "text": "after the backup" }))).await.unwrap();
    a.back_up_now();
    // A run is done once it says when it last backed up, not once its copy of the bucket has the post: it copies the
    // bucket before writing the snapshot that names the copy, and backs up every scope.
    eventually("the second backup", async || a.backups().unwrap()["places"][0]["last"]["time"] != first).await;
    let again = one::backups::restore(&place, &install, "global", &keys["global"]).unwrap();
    assert!(again.snapshot.databases["board/main"].0 > steps_then);
    assert_eq!(backed_up_posts(&dir, &install, &one::entry::unhex(&recovery).unwrap()).await, ["before the backup", "after the backup"]);
    let checked = a.check_backups().await.unwrap();
    assert_eq!(checked[0]["place"], "T7 Shield", "{checked}");
    assert!(checked[0]["error"].is_null(), "{checked}");
    let shown = a.backups().unwrap();
    let moment = &shown["snapshots"][0];
    assert!(moment["scopes"].as_array().unwrap().contains(&json!("Everyone")) && moment["places"] == json!(["T7 Shield"]), "one snapshot of the install: {shown}");
    assert!(a.state()["backups"].is_null(), "no badge while backups work");

    // A disk that breaks: admins see what's wrong and its fix, on Settings, until a run works again.
    let copy = dir.join(&install).join("bucket");
    std::fs::rename(&copy, dir.join("bucket-aside")).unwrap();
    std::fs::write(&copy, b"not a folder").unwrap();
    // Its failure shows once it's done: nothing writes to the disk until the next run.
    a.call(board("post", json!({ "text": "broken" }))).await.unwrap();
    a.back_up_now();
    eventually("a failed run", async || a.backups().unwrap()["places"][0]["problem"]["text"].as_str().is_some_and(|t| t.contains("couldn't be used") || t.contains("won't let"))).await;
    let problem = a.state()["backups"].clone();
    assert_eq!(problem["fix"], "Check the disk on backup-a is plugged in and has room.", "{problem}");
    assert!(problem["text"].as_str().unwrap().starts_with("T7 Shield "), "{problem}");
    std::fs::remove_file(&copy).unwrap();
    std::fs::rename(dir.join("bucket-aside"), &copy).unwrap();
    a.call(board("post", json!({ "text": "fixed" }))).await.unwrap();
    a.back_up_now();
    eventually("the badge to clear", async || a.state()["backups"].is_null()).await;

    // A folder named only by a Vault item, which anyone may write, is never written to: a folder is its own device's
    // choice. Nor is an S3 place no admin's setting names.
    let global = one::entry::Scope::Global;
    let planted = temp("backup-planted").join("home");
    let fields = json!({ "path": { "value": planted }, "device": { "value": one::entry::hex(&a.id()) } });
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

/// The board's posts in `global` as a place's copy of the install's bucket holds them, opened with the recovery key
/// alone: each epoch's key sealed to it, each app's key in the epoch sealed under that.
async fn backed_up_posts(dir: &std::path::Path, install: &str, recovery: &[u8; 32]) -> Vec<String> {
    backed_up(dir, install, recovery, "global", "board", "main", "select text from posts order by time").await
}

/// The first column of what `sql` reads from `app`'s `database` in `scope`, as the backup's copy of the bucket has it,
/// opened with the recovery key alone.
async fn backed_up(dir: &std::path::Path, install: &str, recovery: &[u8; 32], scope: &str, app: &str, database: &str, sql: &str) -> Vec<String> {
    let copy = one::bucket::Place::Folder(dir.join(install).join("bucket"));
    let folder = one::bucket::folder(scope);
    let mut keys = one::db::Keys::new();
    for n in copy.list(&format!("{folder}/_epochs")).unwrap().iter().filter_map(|n| n.parse::<u32>().ok()) {
        let epoch = one::backups::unseal(recovery, &copy.get(&one::keys::epoch_name(scope, n, "recovery")).unwrap().unwrap()).unwrap();
        let Ok(Some(sealed)) = copy.get(&one::keys::app_key_name(scope, app, n)) else { continue };
        keys.insert(n, one::kernel::database_key(&one::kernel::open_app_key(&epoch, app, &sealed).unwrap(), database));
    }
    match one::db::Db::open(&copy, &format!("{folder}/{app}/{database}"), keys).await {
        Ok(db) => db.query(sql, &[]).unwrap().into_iter().filter_map(|r| r.values().next().and_then(|v| v.as_str()).map(str::to_string)).collect(),
        Err(_) => Vec::new(),
    }
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
    std::fs::write(dir.join("device.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir, one::sidecars::built_in()).await.unwrap();
    a.create("Home", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    eventually("the board to run", async || a.call(board("post", json!({ "text": "hi" }))).await.is_ok()).await;
    let global = one::entry::Scope::Global;
    let new_key = || ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let asks = |key: &ed25519_dalek::SigningKey, user: &str, proof: Option<String>| {
        let e = one::keys::Endpoint::of(key);
        DeviceRequest { key: one::entry::hex(&e.key), seal: one::entry::hex(&e.seal), name: "Chrome".into(), user: user.into(), person: String::new(), proof, service: None, shared: false, enrollment: None }
    };
    let opens = async |key: &ed25519_dalek::SigningKey, scope: &one::entry::Scope| -> Vec<u32> {
        let got = a.sealed_keys(&me, scope, Some(&one::entry::hex(&key.verifying_key().to_bytes())), None).await.unwrap();
        let sealed = got["sealed"].as_object().unwrap();
        sealed.iter().filter(|(_, v)| one::kernel::open_sealed(&one::keys::seal_secret(key), &data_encoding::BASE64URL_NOPAD.decode(v.as_str().unwrap().as_bytes()).unwrap()).is_ok()).map(|(n, _)| n.parse().unwrap()).collect()
    };

    // Andre's new laptop, approved from his machine, opens global.
    let laptop = new_key();
    a.approve_request(&me, &asks(&laptop, &me.user, None), None).await.unwrap();
    assert_eq!(opens(&laptop, &global).await, vec![1]);

    // Sam is let in only with the proof of the secret Andre's invite made.
    let secret = "the-invites-secret";
    let sam = Invited { secret: secret.into(), name: "Sam".into(), user: one::entry::new_id(), role: one::members::Role::Member, until: u64::MAX };
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

    // The laptop, removed, opens no epoch made after; Andre's machine and Sam's device do, and the board goes on.
    let removed = a.remove_device(&me, &one::entry::hex(&laptop.verifying_key().to_bytes())).await.unwrap();
    assert!(removed["new_epochs"].as_array().unwrap().iter().any(|e| e["scope"] == "global" && e["epoch"] == 2), "{removed}");
    assert_eq!(opens(&laptop, &global).await, vec![1]);
    for f in walk(&dir).into_iter().filter(|p| p.to_string_lossy().contains("/_epochs/2/")) {
        assert!(one::kernel::open_sealed(&one::keys::seal_secret(&laptop), &std::fs::read(&f).unwrap()).is_err(), "{} opens for the removed laptop", f.display());
    }
    assert_eq!(opens(&sams, &global).await, vec![1, 2]);
    a.call(board("post", json!({ "text": "after" }))).await.unwrap();
}

/// A file put from elsewhere (a client device's, a sign-in's) is read back by whoever put it alone, or by someone who
/// may use a scope that names it: knowing its hash isn't enough. A file put to answer one request (git's push, to
/// Repos' `_http`) goes once it's answered, unless something keeps it.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_put_from_elsewhere_is_read_only_by_whoever_put_it() {
    let a = start("put-from").await;
    a.create("Home", "Andre").await.unwrap();
    let caller = |user: &str| one::node::Caller { key: a.id(), user: user.into(), agent: None, token: Some(format!("{user}'s")) };
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
