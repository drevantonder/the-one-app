//! Repos' rules, as an app runtime runs the app: who pushes to a repo and who deletes it, the objects a merge of packs keeps, and
//! whose GitHub sign-in and organization a user sees. It runs with the rest of `cargo test` (tests/main.rs), and a
//! deploy leaves it out of the app (`tests/`).

use serde_json::{Value, json};

use crate::e2e::{create, eventually, let_sam_in, signed_by, start};
use crate::temp;
use ::api::call::CallRequest;

/// In Repos, a repo's owner and its collaborators push to it, and its owners alone delete it: someone else is refused,
/// from a client device or a sign-in from elsewhere, whose git reaches Repos' `_http` as them, until its owner adds them
/// as a collaborator; a group's members push to its repos, and only its admins delete them. In its issues, anyone opens
/// and comments, and edits and closes their own; only those who push close, label and assign another's, and only its
/// owners delete one.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_repos_owner_and_collaborators_push_to_it_and_only_its_owners_delete_it() {
    use api::Caller;
    let a = start("repos-owner").await;
    create(&a, "Home", "Andre").await;
    let me = Caller::own(a.id(), a.user().unwrap());
    // Naming no scopes, as git's requests don't: Sam's own runs nowhere, as he picked nowhere, so his run without it.
    let repos = |tool: &str, input: Value| CallRequest { app: "repos".into(), tool: tool.into(), input, ..Default::default() };
    eventually("Repos to run", &a, async || a.call(repos("list", json!({}))).await.is_ok()).await;

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

    let body = database_runtime::put(&a.rt(), &push(&[(&zero, &head, "refs/heads/main")], &pack)).unwrap();
    let told = a.call(http("andre/home", Some(body))).await.unwrap();
    assert!(told["text"].as_str().unwrap().contains("ok refs/heads/main"), "{told}");
    assert_eq!(branches("andre/home").await, [("main".to_string(), head.clone())]);

    let sams = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let e = key::Endpoint::of(&sams);
    let sam_id = let_sam_in(&a, &sams).await;

    // Repos' keys in global, as Sam's device opens them and hands them to the app runtime with his calls.
    let global = scope::scopes::Scope::Global;
    let b64 = |v: &Value| data_encoding::BASE64URL_NOPAD.decode(v.as_str().unwrap().as_bytes()).unwrap();
    let epochs = a.sealed_keys(&me, &global, Some(&key::hex(&e.key)), None).await.unwrap();
    let mut keys = Vec::new();
    for (app, database) in [("repos", "main"), (key::APP, key::CoreDb::Vault.name())] {
        let sealed = a.sealed_keys(&me, &global, None, Some(app)).await.unwrap();
        for (n, epoch) in epochs["sealed"].as_object().unwrap() {
            let epoch: [u8; 32] = key::open_sealed(&key::seal_secret(&sams), &b64(epoch)).unwrap().try_into().unwrap();
            let key = key::open_database_key(&epoch, app, database, &b64(&sealed["sealed"][n])).unwrap();
            keys.push(::api::handed::Handed { scope: "global".into(), epoch: n.parse().unwrap(), app: app.into(), database: database.into(), key });
        }
    }

    let client = Caller { key: e.key, user: sam_id.clone(), agent: None, credential: api::Credential::Client(e.key) };
    {
        let (sam, handed_for) = (client.clone(), key::hex(&e.key));
        let handoff = ::api::handed::Handoff { for_: handed_for, until: u64::MAX, keys: keys.clone() };
        a.hand(&sam.credential, &sam.user, &::api::handed::seal_handoff(&a.endpoint().seal, &handoff).unwrap(), u64::MAX).unwrap();
        let call = |req: CallRequest| a.call_as(req, sam.clone());

        let (body, _) = database_runtime::put_from(&a.rt(), &sam, axum::body::Body::from(push(&[(&head, &zero, "refs/heads/main"), (&zero, &head, "refs/heads/sams")], &[]))).await.unwrap();
        let told = call(http("andre/home", Some(body))).await.unwrap();
        let text = told["text"].as_str().unwrap();
        assert!(text.contains("ng refs/heads/main ") && text.contains("ng refs/heads/sams ") && !text.contains("ok refs/"), "{told}");
        assert_eq!(branches("andre/home").await, [("main".to_string(), head.clone())]);

        let refused = call(repos("delete", json!({ "repo": "andre/home" }))).await.unwrap_err();
        assert!(format!("{refused:#}").contains("only Andre manage andre/home"), "{refused:#}");
        assert!(call(repos("add_collaborator", json!({ "repo": "andre/home", "user": "sam", "confirm": true }))).await.is_err());
        assert!(a.call(repos("get", json!({ "repo": "andre/home" }))).await.unwrap()["deleted"].is_null());
    }
    let call = |req: CallRequest, sam: &Caller| a.call_as(req, sam.clone());

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
    let sams_push = async |sam: &Caller, repo: &str, branch: &str, deleting: bool| {
        let r = format!("refs/heads/{branch}");
        let line = if deleting { (head.as_str(), zero.as_str(), r.as_str()) } else { (zero.as_str(), head.as_str(), r.as_str()) };
        let (body, _) = database_runtime::put_from(&a.rt(), sam, axum::body::Body::from(push(&[line], if deleting { &[] } else { &pack }))).await.unwrap();
        // A call reaches the groups he's in too, whose keys his device would hand; this one hands global's alone.
        let told = call(CallRequest { scopes: (!repo.starts_with("andre/")).then(|| vec!["global".into()]), ..http(repo, Some(body)) }, sam).await.unwrap();
        told["text"].as_str().unwrap().contains(&format!("ok {r}"))
    };

    a.call(repos("add_collaborator", json!({ "repo": "andre/home", "user": "sam", "confirm": true }))).await.unwrap();
    assert!(sams_push(&client, "andre/home", "sams", false).await);
    assert!(sams_push(&client, "andre/home", "sams", true).await);
    assert!(call(repos("delete", json!({ "repo": "andre/home" })), &client).await.is_err());
    a.call(repos("remove_collaborator", json!({ "repo": "andre/home", "user": "sam", "confirm": true }))).await.unwrap();
    assert!(!sams_push(&client, "andre/home", "sams", false).await);

    let group = scope::settings::answer(&a, "POST", "/api/group", br#"{ "name": "Team" }"#).await.unwrap().unwrap()["group"].as_str().unwrap().to_string();
    a.call(repos("create", json!({ "name": "site", "owner": "team" }))).await.unwrap();
    assert!(!sams_push(&client, "team/site", "sams", false).await);
    a.member_content(&me, scope::content::invite(&group, &sam_id, scope::scopes::Role::Member)).await.unwrap();
    signed_by(&a, &client, &sams, scope::content::accept(&group)).await;
    assert!(sams_push(&client, "team/site", "sams", false).await, "a member of the group pushes to its repo");
    let refused = call(CallRequest { scopes: Some(vec!["global".into()]), ..repos("delete", json!({ "repo": "team/site" })) }, &client).await.unwrap_err();
    assert!(format!("{refused:#}").contains("only Team's admins manage team/site"), "{refused:#}");

    // Everyone's repo: being in the network makes no one its maker or pusher, until its admin adds them.
    let core = format!("{}/core", custom_build::config::CLI);
    let as_sam = |tool: &str, input: Value| CallRequest { scopes: Some(vec!["global".into()]), ..repos(tool, input) };
    let refused = call(as_sam("create", json!({ "name": "core", "owner": custom_build::config::CLI })), &client).await.unwrap_err();
    assert!(format!("{refused:#}").contains("only Everyone's admins make its repos"), "{refused:#}");
    a.call(repos("create", json!({ "name": "core", "owner": custom_build::config::CLI }))).await.unwrap();
    assert!(!sams_push(&client, &core, "sams", false).await, "being in the network doesn't push to Everyone's repo");
    a.call(repos("add_collaborator", json!({ "repo": core, "user": "sam", "confirm": true }))).await.unwrap();
    assert!(sams_push(&client, &core, "sams", false).await, "a collaborator its admin added pushes");
    let refused = call(as_sam("delete", json!({ "repo": core })), &client).await.unwrap_err();
    assert!(format!("{refused:#}").contains(&format!("only Everyone's admins manage {core}")), "{refused:#}");

    // Transferring: only its owners may, and only to somewhere they make repos. Its old address leads to it, by the
    // same rights, until a repo is made there.
    let refused = call(as_sam("transfer", json!({ "repo": "team/site", "owner": "sam", "confirm": true })), &client).await.unwrap_err();
    assert!(format!("{refused:#}").contains("only Team's admins manage team/site"), "{refused:#}");
    let refused = a.call(repos("transfer", json!({ "repo": "team/site", "owner": "sam", "confirm": true }))).await.unwrap_err();
    assert!(format!("{refused:#}").contains("only Sam makes repos as sam"), "{refused:#}");
    a.call(repos("transfer", json!({ "repo": "team/site", "owner": custom_build::config::CLI, "confirm": true }))).await.unwrap();
    let site = format!("{}/site", custom_build::config::CLI);
    assert_eq!(a.call(repos("get", json!({ "repo": "team/site" }))).await.unwrap()["repo"], site.as_str());
    assert!(!sams_push(&client, "team/site", "after", false).await, "a member of the group it left no longer pushes, at either address");
    assert!(!sams_push(&client, &site, "after", false).await);
    let told = a.call(repos("_http", json!({ "method": "GET", "path": "team/site.git/info/refs", "query": "service=git-upload-pack" }))).await.unwrap();
    assert!(told["say"].as_str().unwrap().contains(&format!("moved to {site}")), "{told}");
    a.call(repos("create", json!({ "name": "site", "owner": "team" }))).await.unwrap();
    assert_eq!(a.call(repos("get", json!({ "repo": "team/site" }))).await.unwrap()["repo"], "team/site", "a repo made at the old address ends the redirect");
    a.call(repos("delete", json!({ "repo": site }))).await.unwrap();

    a.call(repos("delete", json!({ "repo": core }))).await.unwrap();
    a.call(repos("delete", json!({ "repo": "andre/home" }))).await.unwrap();
    a.call(repos("delete", json!({ "repo": "team/site" }))).await.unwrap();
}

/// In Repos, each push's pack is merged into the ones before it, which are then let go of from the bucket: after many
/// pushes, each of whose packs holds deltas, the packs a repo keeps still hold every object of every head it had, as
/// git itself reads them.
#[tokio::test(flavor = "multi_thread")]
async fn a_repos_merged_packs_hold_every_object_pushed() {
    let a = start("repos-merge").await;
    create(&a, "Home", "Andre").await;
    let repos = |tool: &str, input: Value| CallRequest { app: "repos".into(), tool: tool.into(), input, ..Default::default() };
    eventually("Repos to run", &a, async || a.call(repos("list", json!({}))).await.is_ok()).await;

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
        let body = database_runtime::put(&a.rt(), &body).unwrap();
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
        std::fs::write(&path, database_runtime::blob(&a.rt(), hash.as_str().unwrap()).await.unwrap()).unwrap();
        git(&bare, &["index-pack", path.to_str().unwrap()], b"");
    }
    for head in &heads {
        git(&bare, &["rev-list", "--objects", "--missing=error", head], b"");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_groups_github_organization_shows_only_to_its_members_and_a_users_github_sign_in_only_to_them() {
    use api::Caller;
    use std::sync::{Arc, Mutex};
    let a = start("repos-github").await;
    create(&a, "Home", "Andre").await;
    let me = Caller::own(a.id(), a.user().unwrap());
    let repos = |tool: &str, input: Value| CallRequest { app: "repos".into(), tool: tool.into(), input, ..Default::default() };
    eventually("Repos to run", &a, async || a.call(repos("list", json!({}))).await.is_ok()).await;

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

    let pem = String::from_utf8(std::process::Command::new("openssl").args(["genrsa", "-traditional", "2048"]).output().unwrap().stdout).unwrap();
    let global = scope::scopes::Scope::Global;
    scope::settings::answer(&a, "POST", "/api/vault/grant", json!({ "scope": "global", "device": a.device_name() }).to_string().as_bytes()).await.unwrap().unwrap();
    vault_runtime::set_item(&a.rt(), &me, &global, json!({ "title": "GitHub App", "type": "GitHub App", "hosts": ["127.0.0.1"], "apps": ["repos"], "fields": {
        "app_id": { "value": "1" }, "slug": { "value": "app" }, "client_id": { "value": "Iv1" }, "client_secret": { "value": "s", "secret": true },
        "private_key": { "value": pem, "secret": true }, "web": { "value": base }, "api": { "value": format!("{base}/api/v3") } } })).await.unwrap();
    vault_runtime::set_item(&a.rt(), &me, &scope::scopes::Scope::User(me.user.clone()), json!({ "title": "GitHub andre-gh", "type": "GitHub account", "hosts": ["127.0.0.1"], "apps": ["repos"], "fields": {
        "login": { "value": "andre-gh" }, "access_token": { "value": "andre-token", "secret": true }, "expires": { "value": "0" } } })).await.unwrap();

    let group = scope::settings::answer(&a, "POST", "/api/group", br#"{ "name": "Team" }"#).await.unwrap().unwrap()["group"].as_str().unwrap().to_string();
    a.call(repos("connect_org", json!({ "group": "team" }))).await.unwrap();
    a.call(repos("finish_connect_org", json!({ "back": "http://x/back?installation_id=7&setup_action=install" }))).await.unwrap();
    let names = |list: Value| list.as_array().unwrap().iter().map(|r| r["repo"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(names(a.call(repos("list", json!({ "source": "github" }))).await.unwrap()), ["github.com/andre-gh/mine", "github.com/org/site"]);

    let sams = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let e = key::Endpoint::of(&sams);
    let sam_id = let_sam_in(&a, &sams).await;
    let b64 = |v: &Value| data_encoding::BASE64URL_NOPAD.decode(v.as_str().unwrap().as_bytes()).unwrap();
    let epochs = a.sealed_keys(&me, &global, Some(&key::hex(&e.key)), None).await.unwrap();
    let mut keys = Vec::new();
    for (app, database) in [("repos", "main"), (key::APP, key::CoreDb::Vault.name())] {
        let sealed = a.sealed_keys(&me, &global, None, Some(app)).await.unwrap();
        for (n, epoch) in epochs["sealed"].as_object().unwrap() {
            let epoch: [u8; 32] = key::open_sealed(&key::seal_secret(&sams), &b64(epoch)).unwrap().try_into().unwrap();
            let key = key::open_database_key(&epoch, app, database, &b64(&sealed["sealed"][n])).unwrap();
            keys.push(::api::handed::Handed { scope: "global".into(), epoch: n.parse().unwrap(), app: app.into(), database: database.into(), key });
        }
    }
    let client = Caller { key: e.key, user: sam_id.clone(), agent: None, credential: api::Credential::Client(e.key) };
    let handoff = ::api::handed::Handoff { for_: key::hex(&e.key), until: u64::MAX, keys };
    a.hand(&client.credential, &sam_id, &::api::handed::seal_handoff(&a.endpoint().seal, &handoff).unwrap(), u64::MAX).unwrap();
    let sams_list = async || {
        let req = CallRequest { scopes: Some(vec!["global".into()]), ..repos("list", json!({ "source": "github" })) };
        let before = seen.lock().unwrap().len();
        let list = a.call_as(req, client.clone()).await.unwrap();
        (names(list), seen.lock().unwrap()[before..].to_vec())
    };

    assert_eq!(sams_list().await, (Vec::<String>::new(), vec![]));
    let refused = a.call_as(CallRequest { scopes: Some(vec!["global".into()]), ..repos("tree", json!({ "repo": "github.com/andre-gh/mine" })) }, client.clone()).await.unwrap_err();
    assert!(format!("{refused:#}").contains("Sign in to GitHub first"), "{refused:#}");

    a.member_content(&me, scope::content::invite(&group, &sam_id, scope::scopes::Role::Member)).await.unwrap();
    signed_by(&a, &client, &sams, scope::content::accept(&group)).await;
    let (list, sent) = sams_list().await;
    assert_eq!(list, ["github.com/org/site"]);
    assert!(!sent.is_empty() && sent.iter().all(|(_, auth)| !auth.contains("andre-token")), "{sent:?}");
}
