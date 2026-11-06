//! Real nodes on this machine, reaching each other through the relay at a real network's gateway.

use std::time::{Duration, Instant};

use one::node::{CallRequest, Node};
use serde_json::{Value, json};

use crate::temp;

async fn start(name: &str) -> Node {
    let dir = temp(name);
    std::fs::write(dir.join("device.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": name }).to_string()).unwrap();
    Node::start(&dir).await.unwrap()
}

/// Workers whose relay the devices of `a`'s network reach each other through, taking only `a` and those it
/// admits: `a` says it's the network's.
async fn relay(a: &Node) -> crate::cloudflare::Workers {
    let k = tokio::task::block_in_place(|| crate::cloudflare::Workers::for_devices(vec![one::entry::hex(&a.id())]));
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

/// `content` written to `global`'s scope database as a step `caller`'s own device (`key`) signs, as a client does.
async fn signed_by(a: &Node, caller: &one::node::Caller, key: &ed25519_dalek::SigningKey, content: one::entry::Content) {
    let global = one::entry::Scope::Global;
    let at = a.global_steps(caller, 9007199254740991, None).await.unwrap()["at"].as_u64().unwrap();
    let epoch = a.entry_heads(caller, &global).await.unwrap()["epoch"].as_u64().unwrap() as u32;
    let step = one::database::steps::sign(key, &global, at + 1, &caller.user, epoch, one::node::now_ms(), content).unwrap();
    a.put_signed_steps(caller, vec![step]).await.unwrap();
}

fn notes(tool: &str, input: Value) -> CallRequest {
    CallRequest { app: "notes".into(), tool: tool.into(), input, ..Default::default() }
}

/// Deploys the tests' own app, `tests/notes/app.js` (lines of text in `global`'s database), for everyone, as an admin
/// would add an app, and waits for this device to run it.
async fn add_notes(a: &Node) {
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    let bundle = a.put_blob(&one::app::bundle::pack(&[("app.js".to_string(), include_bytes!("notes/app.js").to_vec())]).unwrap()).unwrap();
    a.deploy_making(&me, "notes", &bundle, Some("admins"), true).await.unwrap();
    a.install(&me, "notes", &one::entry::Scope::Global, true, None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn backups_go_to_a_folder_encrypted_and_restore_with_the_recovery_key_alone() {
    let a = start("backup-a").await;
    let b = start("backup-b").await;
    let recovery = a.create("Home", "Andre").await.unwrap();
    add_notes(&a).await;
    let _relay = relay(&a).await;
    b.join(&a.pair().unwrap()).await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    eventually("notes to run", async || a.call(notes("post", json!({ "text": "before the backup" }))).await.is_ok()).await;
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
    let network = one::backups::networks(&place).unwrap().remove(0);
    let keys = one::backups::open_keys(&place, &network, &one::entry::unhex(&recovery).unwrap()).unwrap();
    assert!(keys.contains_key("global") && keys.contains_key(&format!("user:{}", me.user)), "{:?}", keys.keys());
    let checked = one::backups::check(&one::backups::Check { place: &place, network: &network, keys: &keys, root: None, live: None }).unwrap().0;
    assert_eq!(checked["problems"], json!([]), "{checked}");
    assert_eq!(checked["network"], "Home");
    assert_eq!(checked["people"], json!(["Andre"]));
    assert!(checked["apps"].as_array().unwrap().contains(&json!("notes")), "{checked}");
    let mine = format!("user:{}", me.user);
    let items = backed_up(&dir, &network, &one::entry::unhex(&recovery).unwrap(), &mine, "node", "vault", "select item from items").await;
    assert!(items.iter().any(|i| i.contains("\"Example key\"")), "the Vault's item came back: {items:?}");
    let restored = one::backups::restore(&place, &network, "global", &keys["global"]).unwrap();
    assert!(restored.entries.iter().any(|e| e.content.kind == "genesis"));
    assert_eq!(backed_up_posts(&dir, &network, &one::entry::unhex(&recovery).unwrap()).await, ["before the backup"]);
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
    assert_eq!(backed_up_posts(&dir, &network, &one::entry::unhex(&recovery).unwrap()).await, ["before the backup", "after the backup"]);
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

/// The notes' lines in `global` as a place's copy of the network's bucket holds them, opened with the recovery key
/// alone: each epoch's key sealed to it, each app's key in the epoch sealed under that.
async fn backed_up_posts(dir: &std::path::Path, install: &str, recovery: &[u8; 32]) -> Vec<String> {
    backed_up(dir, install, recovery, "global", "notes", "main", "select text from posts order by n").await
}

/// The first column of what `sql` reads from `app`'s `database` in `scope`, as the backup's copy of the bucket has it,
/// opened with the recovery key alone.
async fn backed_up(dir: &std::path::Path, install: &str, recovery: &[u8; 32], scope: &str, app: &str, database: &str, sql: &str) -> Vec<String> {
    let (copy, keys) = backed_up_keys(dir, install, recovery, scope, app, database);
    match one::db::Db::open(&copy, &format!("{}/{app}/{database}", one::bucket::folder(scope)), keys).await {
        Ok(db) => db.query(sql, &[]).unwrap().into_iter().filter_map(|r| r.values().next().and_then(|v| v.as_str()).map(str::to_string)).collect(),
        Err(_) => Vec::new(),
    }
}

/// The backup's copy of the network's bucket, and the keys of `app`'s `database` in `scope` there, opened with the
/// recovery key alone.
fn backed_up_keys(dir: &std::path::Path, install: &str, recovery: &[u8; 32], scope: &str, app: &str, database: &str) -> (one::bucket::Place, one::db::Keys) {
    let copy = one::bucket::Place::Folder(dir.join(install).join("bucket"));
    let folder = one::bucket::folder(scope);
    let mut keys = one::db::Keys::new();
    for n in copy.list(&format!("{folder}/_epochs")).unwrap().iter().filter_map(|n| n.parse::<u32>().ok()) {
        let epoch = one::backups::unseal(recovery, &copy.get(&one::keys::epoch_name(scope, n, "recovery")).unwrap().unwrap()).unwrap();
        let Ok(Some(sealed)) = copy.get(&one::keys::app_key_name(scope, app, n)) else { continue };
        keys.insert(n, one::cloudflare::database_key(&one::cloudflare::open_app_key(&epoch, app, &sealed).unwrap(), database));
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
    std::fs::write(dir.join("device.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    a.create("Home", "Andre").await.unwrap();
    add_notes(&a).await;
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    eventually("notes to run", async || a.call(notes("post", json!({ "text": "hi" }))).await.is_ok()).await;
    let global = one::entry::Scope::Global;
    let new_key = || ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let asks = |key: &ed25519_dalek::SigningKey, user: &str, proof: Option<String>| {
        let e = one::keys::Endpoint::of(key);
        DeviceRequest { key: one::entry::hex(&e.key), seal: one::entry::hex(&e.seal), name: "Chrome".into(), user: user.into(), person: String::new(), proof, device: None, made: None, place: None, sign_in: None }
    };
    let opens = async |key: &ed25519_dalek::SigningKey, scope: &one::entry::Scope| -> Vec<u32> {
        let got = a.sealed_keys(&me, scope, Some(&one::entry::hex(&key.verifying_key().to_bytes())), None).await.unwrap();
        let sealed = got["sealed"].as_object().unwrap();
        sealed.iter().filter(|(_, v)| one::cloudflare::open_sealed(&one::keys::seal_secret(key), &data_encoding::BASE64URL_NOPAD.decode(v.as_str().unwrap().as_bytes()).unwrap()).is_ok()).map(|(n, _)| n.parse().unwrap()).collect()
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
    // Nor does Andre's machine approve anything for Sam, asked from Sam's device over the network: it isn't Sam's.
    let from_sam = one::node::Caller { key: sams.verifying_key().to_bytes(), user: sam.user.clone(), agent: None, token: Some("Sam's".into()) };
    let refused = a.approve(&from_sam, "ABCDEFGH", true).await.unwrap_err();
    assert!(format!("{refused:#}").contains("isn't one of your devices"), "{refused:#}");

    // The laptop, removed, opens no epoch made after; Andre's machine and Sam's device do, and the notes go on.
    let removed = a.remove_device(&me, &one::entry::hex(&laptop.verifying_key().to_bytes())).await.unwrap();
    assert!(removed["new_epochs"].as_array().unwrap().iter().any(|e| e["scope"] == "global" && e["epoch"] == 2), "{removed}");
    assert_eq!(opens(&laptop, &global).await, vec![1]);
    for f in walk(&dir).into_iter().filter(|p| p.to_string_lossy().contains("/_epochs/2/")) {
        assert!(one::cloudflare::open_sealed(&one::keys::seal_secret(&laptop), &std::fs::read(&f).unwrap()).is_err(), "{} opens for the removed laptop", f.display());
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

/// In Repos, a repo's owner and its collaborators push to it, and its owners alone delete it: someone else is refused,
/// from a client device or a sign-in from elsewhere, whose git reaches Repos' `_http` as them, until its owner adds them
/// as a collaborator; a group's members push to its repos, and only its admins delete them. In its issues, anyone opens
/// and comments, and edits and closes their own; only those who push close, label and assign another's, and only its
/// owners delete one.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_repos_owner_and_collaborators_push_to_it_and_only_its_owners_delete_it() {
    use one::node::{Caller, DeviceRequest, Invited};
    let a = start("repos-owner").await;
    a.create("Home", "Andre").await.unwrap();
    let me = Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    // Naming no scopes, as git's requests don't: Sam's own runs nowhere, as he picked nowhere, so his run without it.
    let repos = |tool: &str, input: Value| CallRequest { app: "repos".into(), tool: tool.into(), input, ..Default::default() };
    eventually("Repos to run", async || a.call(repos("list", json!({}))).await.is_ok()).await;

    // A real one-commit pack, as git sends it.
    let dir = temp("repos-owner-git");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git").args(["-c", "user.name=Andre", "-c", "user.email=andre@example.com"]).args(args).current_dir(&dir).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        out.stdout
    };
    git(&["init", "-q"]);
    std::fs::write(dir.join("README"), "hello\n").unwrap();
    git(&["add", "README"]);
    git(&["commit", "-qm", "First"]);
    let head = String::from_utf8(git(&["rev-parse", "HEAD"])).unwrap().trim().to_string();
    let mut packing = std::process::Command::new("git").args(["pack-objects", "--stdout", "--revs"]).current_dir(&dir).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    std::io::Write::write_all(&mut packing.stdin.take().unwrap(), b"HEAD\n").unwrap();
    let pack = packing.wait_with_output().unwrap().stdout;

    // git's push: one pkt-line per branch moved (old, new, ref), a flush, then the pack.
    let zero = "0".repeat(40);
    let push = |commands: &[(&str, &str, &str)], pack: &[u8]| {
        let mut body = Vec::new();
        for (i, (old, new, r)) in commands.iter().enumerate() {
            let line = format!("{old} {new} {r}{}\n", if i == 0 { "\0report-status" } else { "" });
            body.extend(format!("{:04x}{line}", line.len() + 4).bytes());
        }
        body.extend(b"0000");
        body.extend(pack);
        body
    };
    let http = |repo: &str, body: Option<String>| repos("_http", json!({ "method": "POST", "path": format!("{repo}.git/git-receive-pack"), "query": "", "body": body }));
    let branches = async |repo: &str| {
        let got = a.call(repos("get", json!({ "repo": repo }))).await.unwrap();
        got["branches"].as_array().unwrap().iter().map(|b| (b["branch"].as_str().unwrap().to_string(), b["head"].as_str().unwrap().to_string())).collect::<Vec<_>>()
    };

    // Andre's push makes the repo, his, at his handle.
    let body = a.put_blob(&push(&[(&zero, &head, "refs/heads/main")], &pack)).unwrap();
    let told = a.call(http("andre/home", Some(body))).await.unwrap();
    assert!(told["text"].as_str().unwrap().contains("ok refs/heads/main"), "{told}");
    assert_eq!(branches("andre/home").await, [("main".to_string(), head.clone())]);

    // Sam, a person of the network, with a device of his.
    let secret = "the-invites-secret";
    let sam = Invited { secret: secret.into(), name: "Sam".into(), user: one::entry::new_id(), role: one::members::Role::Member, until: u64::MAX };
    let sams = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let e = one::keys::Endpoint::of(&sams);
    let proof = one::keys::invite_proof(secret.as_bytes(), &e);
    let asks = DeviceRequest { key: one::entry::hex(&e.key), seal: one::entry::hex(&e.seal), name: "Sam's laptop".into(), user: sam.user.clone(), person: String::new(), proof: Some(proof), device: None, made: None, place: None, sign_in: None };
    a.approve_request(&me, &asks, Some(&sam)).await.unwrap();

    // Repos' keys in global, as Sam's device opens them and hands them to the node with his calls.
    let global = one::entry::Scope::Global;
    let b64 = |v: &Value| data_encoding::BASE64URL_NOPAD.decode(v.as_str().unwrap().as_bytes()).unwrap();
    // Repos' own, and the Vault's, which fills in its items, as a client hands them when the node asks.
    let epochs = a.sealed_keys(&me, &global, Some(&one::entry::hex(&e.key)), None).await.unwrap();
    let mut keys = Vec::new();
    for (app, database) in [("repos", "main"), (one::members::APP, one::vault::DATABASE)] {
        let sealed = a.sealed_keys(&me, &global, None, Some(app)).await.unwrap();
        for (n, epoch) in epochs["sealed"].as_object().unwrap() {
            let epoch: [u8; 32] = one::cloudflare::open_sealed(&one::keys::seal_secret(&sams), &b64(epoch)).unwrap().try_into().unwrap();
            let key = one::cloudflare::open_app_key(&epoch, app, &b64(&sealed["sealed"][n])).unwrap();
            keys.push(one::keys::Handed { scope: "global".into(), epoch: n.parse().unwrap(), app: app.into(), database: database.into(), key: one::cloudflare::database_key(&key, database) });
        }
    }

    let client = Caller { key: e.key, user: sam.user.clone(), agent: None, token: Some(one::node::client_token(&e.key)) };
    let signed_in = Caller { key: a.id(), user: sam.user.clone(), agent: None, token: Some("Sam's sign-in".into()) };
    let mut both = Vec::new();
    for (sam, handed_for) in [(client, one::entry::hex(&e.key)), (signed_in, one::keys::token_id("Sam's sign-in"))] {
        let handoff = one::keys::Handoff { for_: handed_for, until: u64::MAX, keys: keys.clone() };
        a.hand(sam.token.as_deref().unwrap(), &sam.user, &one::keys::seal_handoff(&a.endpoint().seal, &handoff).unwrap(), u64::MAX).unwrap();
        let call = |req: CallRequest| a.call_as(a.for_sign_in(req).unwrap(), sam.clone());

        // His push, deleting main and making a branch of his at its head, is refused, and nothing moves.
        let (body, _) = a.put_blob_from(&sam, axum::body::Body::from(push(&[(&head, &zero, "refs/heads/main"), (&zero, &head, "refs/heads/sams")], &[]))).await.unwrap();
        let told = call(http("andre/home", Some(body))).await.unwrap();
        let text = told["text"].as_str().unwrap();
        assert!(text.contains("ng refs/heads/main ") && text.contains("ng refs/heads/sams ") && !text.contains("ok refs/"), "{told}");
        assert_eq!(branches("andre/home").await, [("main".to_string(), head.clone())]);

        // Nor does he delete it, nor make himself a collaborator.
        let refused = call(repos("delete", json!({ "repo": "andre/home" }))).await.unwrap_err();
        assert!(format!("{refused:#}").contains("only Andre manage andre/home"), "{refused:#}");
        assert!(call(repos("add_collaborator", json!({ "repo": "andre/home", "user": "sam", "confirm": true }))).await.is_err());
        assert!(a.call(repos("get", json!({ "repo": "andre/home" }))).await.unwrap()["deleted"].is_null());
        both.push(sam);
    }
    let [client, signed_in] = <[Caller; 2]>::try_from(both).unwrap();
    let call = |req: CallRequest, sam: &Caller| a.call_as(a.for_sign_in(req).unwrap(), sam.clone());

    // Its issues: Sam opens one and comments, as anyone may, and edits and closes his own; he doesn't close, edit or
    // delete Andre's, label his own or make a label, which only those who push do, nor delete his own, which only its
    // owners do.
    let issue = |tool: &str, number: u64, mut input: Value| {
        (input["repo"], input["number"]) = (json!("andre/home"), json!(number));
        repos(tool, input)
    };
    assert_eq!(a.call(repos("open_issue", json!({ "repo": "andre/home", "title": "Andre's" }))).await.unwrap()["number"], 1);
    assert_eq!(call(repos("open_issue", json!({ "repo": "andre/home", "title": "Sam's" })), &client).await.unwrap()["number"], 2);
    call(issue("comment", 1, json!({ "body": "Me too" })), &client).await.unwrap();
    let refused = call(issue("close_issue", 1, json!({})), &client).await.unwrap_err();
    assert!(format!("{refused:#}").contains("only its author and the owners and collaborators of andre/home close #1"), "{refused:#}");
    for req in [issue("edit_issue", 1, json!({ "title": "Sam's now" })), issue("edit_issue", 2, json!({ "labels": ["bug"] })), issue("delete_issue", 2, json!({ "confirm": true })), repos("set_label", json!({ "repo": "andre/home", "name": "sams" }))] {
        let tool = req.tool.clone();
        assert!(call(req, &client).await.is_err(), "Sam's {tool} is refused");
    }
    call(issue("edit_issue", 2, json!({ "title": "Sam's, edited" })), &client).await.unwrap();
    call(issue("close_issue", 2, json!({ "state_reason": "not_planned" })), &client).await.unwrap();
    a.call(issue("edit_issue", 1, json!({ "labels": ["bug"], "assignees": ["sam"] }))).await.unwrap();
    let all = a.call(repos("issues", json!({ "repo": "andre/home", "state": "all" }))).await.unwrap();
    let [his, andres] = [&all["issues"][0], &all["issues"][1]];
    assert_eq!((his["title"].as_str(), his["state"].as_str(), his["state_reason"].as_str()), (Some("Sam's, edited"), Some("closed"), Some("not_planned")), "{all}");
    assert_eq!((andres["labels"][0]["name"].as_str(), andres["assignees"][0]["handle"].as_str(), andres["comments"].as_u64()), (Some("bug"), Some("sam"), Some(1)), "{all}");
    a.call(issue("delete_issue", 2, json!({ "confirm": true }))).await.unwrap();
    // A push of Sam's: making `branch` at the commit, with its pack, or deleting it: → whether it was taken.
    let sams_push = async |sam: &Caller, repo: &str, branch: &str, deleting: bool| {
        let r = format!("refs/heads/{branch}");
        let line = if deleting { (head.as_str(), zero.as_str(), r.as_str()) } else { (zero.as_str(), head.as_str(), r.as_str()) };
        let (body, _) = a.put_blob_from(sam, axum::body::Body::from(push(&[line], if deleting { &[] } else { &pack }))).await.unwrap();
        // A call reaches the groups he's in too, whose keys his device would hand; this one hands global's alone.
        let told = call(CallRequest { scopes: repo.starts_with("team/").then(|| vec!["global".into()]), ..http(repo, Some(body)) }, sam).await.unwrap();
        told["text"].as_str().unwrap().contains(&format!("ok {r}"))
    };

    // Once Andre adds him as a collaborator, Sam pushes, from either; he still doesn't delete it. Taken out, he's refused.
    a.call(repos("add_collaborator", json!({ "repo": "andre/home", "user": "sam", "confirm": true }))).await.unwrap();
    assert!(sams_push(&client, "andre/home", "sams", false).await);
    assert!(sams_push(&signed_in, "andre/home", "sams", true).await);
    assert!(call(repos("delete", json!({ "repo": "andre/home" })), &client).await.is_err());
    a.call(repos("remove_collaborator", json!({ "repo": "andre/home", "user": "sam", "confirm": true }))).await.unwrap();
    assert!(!sams_push(&client, "andre/home", "sams", false).await);

    // A group's repo: its admins own it. Sam pushes to it only once he's in the group, and doesn't delete it.
    let group = a.make_group(&me, "Team").await.unwrap();
    a.call(repos("create", json!({ "name": "site", "owner": "team" }))).await.unwrap();
    assert!(!sams_push(&client, "team/site", "sams", false).await);
    a.member_as(&me, "invite", json!({ "group": group, "user": sam.user, "role": "member" })).await.unwrap();
    signed_by(&a, &client, &sams, one::members::accept(&group)).await;
    assert!(sams_push(&client, "team/site", "sams", false).await, "a member of the group pushes to its repo");
    let refused = call(CallRequest { scopes: Some(vec!["global".into()]), ..repos("delete", json!({ "repo": "team/site" })) }, &client).await.unwrap_err();
    assert!(format!("{refused:#}").contains("only Team's admins manage team/site"), "{refused:#}");

    // Andre deletes his, and the group's as its admin.
    a.call(repos("delete", json!({ "repo": "andre/home" }))).await.unwrap();
    a.call(repos("delete", json!({ "repo": "team/site" }))).await.unwrap();
}

/// In Repos, each push's pack is merged into the ones before it, which are then let go of from the bucket: after many
/// pushes, each of whose packs holds deltas, the packs a repo keeps still hold every object of every head it had, as
/// git itself reads them.
#[tokio::test(flavor = "multi_thread")]
async fn a_repos_merged_packs_hold_every_object_pushed() {
    let a = start("repos-merge").await;
    a.create("Home", "Andre").await.unwrap();
    let repos = |tool: &str, input: Value| CallRequest { app: "repos".into(), tool: tool.into(), input, ..Default::default() };
    eventually("Repos to run", async || a.call(repos("list", json!({}))).await.is_ok()).await;

    let dir = temp("repos-merge-git");
    let git = |dir: &std::path::Path, args: &[&str], input: &[u8]| {
        let mut child = std::process::Command::new("git").args(["-c", "user.name=Andre", "-c", "user.email=andre@example.com"]).args(args).current_dir(dir)
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        std::io::Write::write_all(&mut child.stdin.take().unwrap(), input).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        out.stdout
    };
    git(&dir, &["init", "-q"], b"");

    let zero = "0".repeat(40);
    let (mut old, mut heads) = (zero.clone(), Vec::new());
    for push in 0..12 {
        // Two commits a push, each growing the same file, so the pack holds deltas.
        for n in 0..2 {
            let text: String = (0..200 * (2 * push + n + 1)).map(|i| format!("line {i}\n")).collect();
            std::fs::write(dir.join("notes"), text).unwrap();
            git(&dir, &["add", "notes"], b"");
            git(&dir, &["commit", "-qm", &format!("Push {push}, commit {n}")], b"");
        }
        let head = String::from_utf8(git(&dir, &["rev-parse", "HEAD"], b"")).unwrap().trim().to_string();
        let revs = if old == zero { "HEAD\n".to_string() } else { format!("HEAD\n^{old}\n") };
        let pack = git(&dir, &["pack-objects", "--stdout", "--revs"], revs.as_bytes());
        let line = format!("{old} {head} refs/heads/main\0report-status\n");
        let mut body = format!("{:04x}{line}0000", line.len() + 4).into_bytes();
        body.extend(pack);
        let body = a.put_blob(&body).unwrap();
        let told = a.call(repos("_http", json!({ "method": "POST", "path": "andre/home.git/git-receive-pack", "query": "", "body": body }))).await.unwrap();
        assert!(told["text"].as_str().unwrap().contains("ok refs/heads/main"), "{told}");
        heads.push(head.clone());
        old = head;
    }

    // What the repo keeps, read by git from nothing else.
    let packs = a.call(repos("get", json!({ "repo": "andre/home" }))).await.unwrap()["packs"].as_array().unwrap().clone();
    assert!(packs.len() < heads.len(), "pushes' packs are merged: {} packs", packs.len());
    let bare = temp("repos-merge-bare");
    git(&bare, &["init", "-q", "--bare"], b"");
    for (i, hash) in packs.iter().enumerate() {
        let path = bare.join(format!("objects/pack/kept-{i}.pack"));
        std::fs::write(&path, a.blob(hash.as_str().unwrap()).await.unwrap()).unwrap();
        git(&bare, &["index-pack", path.to_str().unwrap()], b"");
    }
    for head in &heads {
        git(&bare, &["rev-list", "--objects", "--missing=error", head], b"");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_groups_github_organization_shows_only_to_its_members_and_a_persons_github_sign_in_only_to_them() {
    use one::node::{Caller, DeviceRequest, Invited};
    use std::sync::{Arc, Mutex};
    let a = start("repos-github").await;
    a.create("Home", "Andre").await.unwrap();
    let me = Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    let repos = |tool: &str, input: Value| CallRequest { app: "repos".into(), tool: tool.into(), input, ..Default::default() };
    eventually("Repos to run", async || a.call(repos("list", json!({}))).await.is_ok()).await;

    // GitHub, as far as this needs: Andre's own repo for his token, an organization's for the App's installation,
    // which it gives a token for with a JWT. Each request's path and Authorization is kept.
    let seen: Arc<Mutex<Vec<(String, String)>>> = Default::default();
    let kept = seen.clone();
    let repo = |full: &str| json!({ "full_name": full, "name": full.split('/').nth(1), "owner": { "login": full.split('/').next() }, "default_branch": "main", "html_url": format!("https://github.com/{full}"), "private": true, "permissions": { "push": true }, "pushed_at": "2026-11-01T00:00:00Z" });
    let github = axum::Router::new().fallback(move |req: axum::extract::Request| {
        let kept = kept.clone();
        async move {
            let auth = req.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let path = req.uri().path().to_string();
            kept.lock().unwrap().push((path.clone(), auth.clone()));
            let body = match (path.as_str(), auth.as_str()) {
                ("/api/v3/app/installations/7/access_tokens", jwt) if jwt.starts_with("Bearer ey") && jwt.matches('.').count() == 2 => json!({ "token": "org-token", "expires_at": "2099-01-01T00:00:00Z" }),
                ("/api/v3/installation/repositories", "Bearer org-token") => json!({ "repositories": [repo("org/site")] }),
                ("/api/v3/user/repos", "Bearer andre-token") => json!([repo("andre-gh/mine")]),
                _ => return (axum::http::StatusCode::NOT_FOUND, axum::Json(json!({ "message": "Not Found" }))),
            };
            (axum::http::StatusCode::OK, axum::Json(body))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, github).await.unwrap() });

    // The network's GitHub App in Everyone's Vault, its key opened on this machine, and Andre's own GitHub sign-in in his.
    let pem = String::from_utf8(std::process::Command::new("openssl").args(["genrsa", "-traditional", "2048"]).output().unwrap().stdout).unwrap();
    let global = one::entry::Scope::Global;
    a.vault_grant(&me, &global, &a.device_name(), true).await.unwrap();
    a.set_item(&me, &global, json!({ "title": "GitHub App", "type": "GitHub App", "hosts": ["127.0.0.1"], "apps": ["repos"], "fields": {
        "app_id": { "value": "1" }, "slug": { "value": "app" }, "client_id": { "value": "Iv1" }, "client_secret": { "value": "s", "secret": true },
        "private_key": { "value": pem, "secret": true }, "web": { "value": base }, "api": { "value": format!("{base}/api/v3") } } })).await.unwrap();
    a.set_item(&me, &one::entry::Scope::User(me.user.clone()), json!({ "title": "GitHub andre-gh", "type": "GitHub account", "hosts": ["127.0.0.1"], "apps": ["repos"], "fields": {
        "login": { "value": "andre-gh" }, "access_token": { "value": "andre-token", "secret": true }, "expires": { "value": "0" } } })).await.unwrap();

    // Andre, an admin of Team, connects the organization for it.
    let group = a.make_group(&me, "Team").await.unwrap();
    a.call(repos("connect_org", json!({ "group": "team" }))).await.unwrap();
    a.call(repos("finish_connect_org", json!({ "back": "http://x/back?installation_id=7&setup_action=install" }))).await.unwrap();
    let names = |list: Value| list.as_array().unwrap().iter().map(|r| r["repo"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(names(a.call(repos("list", json!({ "source": "github" }))).await.unwrap()), ["github.com/andre-gh/mine", "github.com/org/site"]);

    // Sam, a person of the network but not in Team, with a device of his that hands its keys with his calls: Repos'
    // and the Vault's in global.
    let secret = "the-invites-secret";
    let sam = Invited { secret: secret.into(), name: "Sam".into(), user: one::entry::new_id(), role: one::members::Role::Member, until: u64::MAX };
    let sams = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let e = one::keys::Endpoint::of(&sams);
    let proof = one::keys::invite_proof(secret.as_bytes(), &e);
    let asks = DeviceRequest { key: one::entry::hex(&e.key), seal: one::entry::hex(&e.seal), name: "Sam's laptop".into(), user: sam.user.clone(), person: String::new(), proof: Some(proof), device: None, made: None, place: None, sign_in: None };
    a.approve_request(&me, &asks, Some(&sam)).await.unwrap();
    let b64 = |v: &Value| data_encoding::BASE64URL_NOPAD.decode(v.as_str().unwrap().as_bytes()).unwrap();
    let epochs = a.sealed_keys(&me, &global, Some(&one::entry::hex(&e.key)), None).await.unwrap();
    let mut keys = Vec::new();
    for (app, database) in [("repos", "main"), (one::members::APP, one::vault::DATABASE)] {
        let sealed = a.sealed_keys(&me, &global, None, Some(app)).await.unwrap();
        for (n, epoch) in epochs["sealed"].as_object().unwrap() {
            let epoch: [u8; 32] = one::cloudflare::open_sealed(&one::keys::seal_secret(&sams), &b64(epoch)).unwrap().try_into().unwrap();
            let key = one::cloudflare::open_app_key(&epoch, app, &b64(&sealed["sealed"][n])).unwrap();
            keys.push(one::keys::Handed { scope: "global".into(), epoch: n.parse().unwrap(), app: app.into(), database: database.into(), key: one::cloudflare::database_key(&key, database) });
        }
    }
    let client = Caller { key: e.key, user: sam.user.clone(), agent: None, token: Some(one::node::client_token(&e.key)) };
    let handoff = one::keys::Handoff { for_: one::entry::hex(&e.key), until: u64::MAX, keys };
    a.hand(client.token.as_deref().unwrap(), &sam.user, &one::keys::seal_handoff(&a.endpoint().seal, &handoff).unwrap(), u64::MAX).unwrap();
    let sams_list = async || {
        let req = CallRequest { scopes: Some(vec!["global".into()]), ..repos("list", json!({ "source": "github" })) };
        let before = seen.lock().unwrap().len();
        let list = a.call_as(a.for_sign_in(req).unwrap(), client.clone()).await.unwrap();
        (names(list), seen.lock().unwrap()[before..].to_vec())
    };

    // Not in Team, he sees none of GitHub's repos, and nothing goes to GitHub for him.
    assert_eq!(sams_list().await, (Vec::<String>::new(), vec![]));
    let refused = a.call_as(a.for_sign_in(CallRequest { scopes: Some(vec!["global".into()]), ..repos("tree", json!({ "repo": "github.com/andre-gh/mine" })) }).unwrap(), client.clone()).await.unwrap_err();
    assert!(format!("{refused:#}").contains("Sign in to GitHub first"), "{refused:#}");

    // Once he's in Team, he sees the organization's repo, as its installation, and never Andre's, nor with his token.
    a.member_as(&me, "invite", json!({ "group": group, "user": sam.user, "role": "member" })).await.unwrap();
    signed_by(&a, &client, &sams, one::members::accept(&group)).await;
    let (list, sent) = sams_list().await;
    assert_eq!(list, ["github.com/org/site"]);
    assert!(!sent.is_empty() && sent.iter().all(|(_, auth)| !auth.contains("andre-token")), "{sent:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trace_says_where_a_call_went_and_never_what_it_carried() {
    use one::node::trace;
    let a = start("trace-a").await;
    a.create("Home", "Andre").await.unwrap();
    add_notes(&a).await;
    eventually("notes to run", async || a.call(notes("post", json!({ "text": "a private thought" }))).await.is_ok()).await;
    let t = trace::Trace::new();
    let read = trace::within(Some(t.clone()), a.call(notes("read", json!({ "format": "json" })))).await.unwrap();
    assert!(read.to_string().contains("a private thought"), "{read}");
    let said = t.done(&a.device_name()).to_string();
    assert!(said.contains(r#""step":"device""#) && said.contains(r#""op":"read""#) && said.contains(r#""step":"run""#), "{said}");
    assert!(!said.contains("private thought") && !said.contains(&a.user().unwrap()), "a trace holds names and timings alone: {said}");
}

/// A node runs a build of an app's service only if its app's latest deploy names it: one whose bytes aren't that hash's
/// is refused, a deploy naming none for this platform runs none, and only someone who may deploy the app names one.
#[tokio::test(flavor = "multi_thread")]
async fn a_service_runs_only_the_build_its_apps_deploy_names() {
    use one::node::Build;
    let dir = temp("service-build");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": "linux" }).to_string()).unwrap();
    let a = Node::start(&dir).await.unwrap();
    a.create("Home", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
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
    let sam = one::node::Caller { key: a.id(), user: "sam".into(), agent: None, token: Some("Sam's".into()) };
    assert!(a.deploy(&sam, "tiny", &named, false).await.is_err());
    // Nor does Andre, from elsewhere, name a build he can't read: one Sam put, by its hash.
    let away = one::node::Caller { key: [7; 32], token: Some("Andre's".into()), ..me.clone() };
    let (sams, _) = a.put_blob_from(&sam, axum::body::Body::from("Sam's program")).await.unwrap();
    let (his, _) = a.put_blob_from(&away, axum::body::Body::from(one::app::bundle::pack(&[("app.js".to_string(), b"export default {}".to_vec()), (one::app::bundle::SERVICES.to_string(), json!({ one::config::TARGET: sams }).to_string().into_bytes())]).unwrap())).await.unwrap();
    let refused = a.deploy(&away, "tiny", &his, false).await.unwrap_err();
    assert!(format!("{refused:#}").contains("put it first"), "{refused:#}");
}
