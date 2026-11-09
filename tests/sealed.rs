//! What a build sealed, kept as bytes, which every later build must open: each file in `tests/sealed/` was written by the
//! build it names, once, and is never written again, so a change to a format, a label or a key's derivation fails here.
//! A new format adds a file; none is ever edited.

use data_encoding::{BASE64URL_NOPAD as B64U, HEXLOWER};
use ed25519_dalek::SigningKey;
use one::backups::{Check, check, check_databases, open_keys, unseal};
use one::bucket::Place;
use one::database::scopes::{APP, CoreDb};
use one::database::stored::Keys;
use one::keys;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const WRITTEN: [&str; 2] = [include_str!("sealed/0.json"), include_str!("sealed/1.json")];

fn written() -> Vec<Value> {
    WRITTEN.iter().map(|text| serde_json::from_str(text).unwrap()).collect()
}

fn b(v: &Value) -> Vec<u8> {
    B64U.decode(v.as_str().unwrap().as_bytes()).unwrap()
}

fn key(v: &Value) -> [u8; 32] {
    HEXLOWER.decode(v.as_str().unwrap().as_bytes()).unwrap().try_into().unwrap()
}

fn list(v: &Value) -> &Vec<Value> {
    v.as_array().unwrap()
}

#[tokio::test]
async fn every_sealed_format_an_earlier_build_wrote_still_opens() {
    for w in written() {
        for s in list(&w["seal"]) {
            let (name, sealed) = (s["name"].as_str().unwrap(), b(&s["sealed"]));
            assert_eq!(u64::from(keys::epoch_of(&sealed).unwrap()), s["epoch"].as_u64().unwrap(), "{}", s["what"]);
            assert_eq!(keys::open(&key(&s["key"]), name, &sealed).unwrap(), b(&s["plain"]), "{}", s["what"]);
            assert!(keys::open(&key(&s["key"]), "another name", &sealed).is_err(), "{}: only as itself", s["what"]);
        }
        for s in list(&w["seal_to"]) {
            let secret = key(&s["secret"]);
            assert_eq!(keys::open_sealed(&secret, &b(&s["sealed"])).unwrap(), b(&s["plain"]), "{}", s["what"]);
            assert_eq!(keys::open_sealed_by(&secret, &b(&s["sealed"])).await.unwrap(), b(&s["plain"]), "{}, by what opens it", s["what"]);
        }
        for s in list(&w["seal_under"]) {
            let secret = s["secret"].as_str().unwrap().as_bytes();
            assert_eq!(keys::open_under(secret, &b(&s["sealed"])).unwrap(), b(&s["plain"]), "{}", s["what"]);
            assert_eq!(keys::id_of(secret), s["id"], "{}'s id", s["what"]);
        }
        for s in list(&w["seal_app_key"]) {
            let (epoch, app) = (key(&s["epoch_key"]), s["app"].as_str().unwrap());
            assert_eq!(keys::open_app_key(&epoch, app, &b(&s["sealed"])).unwrap(), key(&s["app_key"]), "{app}'s key");
            assert!(keys::open_app_key(&epoch, "another app", &b(&s["sealed"])).is_err(), "only as {app}'s");
        }

        let h = &w["handoff"];
        let opened = keys::open_handoff(&key(&h["secret"]), h["sealed"].as_str().unwrap()).unwrap();
        assert_eq!((json!(opened.for_), json!(opened.until), opened.keys[0].key), (h["for"].clone(), h["until"].clone(), key(&h["key"])), "a handoff");

        let n = &w["note"];
        let plain = keys::open_sealed(&key(&n["secret"]), &b(&n["sealed"])).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&plain).unwrap(), n["note"], "a note");

        // Everything a key is made from: a label or a name that changes makes every one of these different.
        let d = &w["derived"];
        let app_key = key(&d["app_key"]);
        for (at, sealed_as) in d["database"].as_object().unwrap() {
            let (app, database) = at.split_once('/').unwrap();
            assert_eq!(keys::database_key(&app_key, app, database), key(sealed_as), "{at}'s key");
        }
        assert_eq!(d["database"][format!("{APP}/{}", CoreDb::Scope.name())], d["database"][format!("{APP}/{}", CoreDb::Log.name())], "the scope database's steps are sealed with the log's key");
        let (recovery, device) = (SigningKey::from_bytes(&key(&w["recovery"])), SigningKey::from_bytes(&key(&w["device"])));
        assert_eq!(keys::twin(&recovery.to_bytes()), (key(&d["twin"]["secret"]), key(&d["twin"]["public"])), "the recovery key's twin");
        assert_eq!(keys::twin_public(recovery.verifying_key().as_bytes()).unwrap(), key(&d["twin"]["public"]), "and from its public half");
        assert_eq!((keys::seal_secret(&device), keys::Endpoint::of(&device).seal), (key(&d["seal_secret"]), key(&d["seal_public"])), "a device's seal key");
        assert_eq!(keys::epoch_hash(&key(&w["epoch"])), d["epoch_hash"], "an epoch's hash, which the signed log names");
        assert_eq!(keys::answers_key(&device), key(&d["answers_key"]), "the key a device's kept answers are under");
        let (sign, seal) = keys::passkey_keys(&key(&d["prf"]));
        assert_eq!((sign.to_bytes(), seal), (key(&d["passkey_sign"]), key(&d["passkey_seal"])), "a passkey's device keys");
        assert_eq!(keys::invite_proof(b"an invite's secret", &keys::Endpoint::of(&device)), d["invite_proof"], "an invite's proof");
    }
}

/// The whole road back from nothing but the recovery key: its key, each epoch's key sealed to it, a backup's snapshot and
/// objects, who's who from its first freeze, and an app's database from its copy and the steps after.
#[tokio::test]
async fn a_backup_an_earlier_build_made_restores_with_only_the_recovery_key() {
    for w in written() {
        let dir = crate::temp("sealed-backup");
        let place = Place::Folder(dir.clone());
        for (name, bytes) in w["place"].as_object().unwrap() {
            place.put(name, &b(bytes)).unwrap();
        }
        let (network, recovery) = (w["network"].as_str().unwrap(), key(&w["recovery"]));

        let opened: BTreeMap<String, Vec<[u8; 32]>> = open_keys(&place, network, &recovery).unwrap();
        assert_eq!(opened["global"], [key(&w["epoch"])], "the recovery key opens each epoch's key");
        let sealed = w["place"][format!("{network}/bucket/{}", keys::epoch_name("global", 1, "recovery"))].clone();
        assert_eq!(unseal(&recovery, &b(&sealed)).unwrap(), key(&w["epoch"]), "as the bucket keeps it too");

        let root = keys::Hash::from_hex(w["root"].as_str().unwrap()).unwrap();
        let (report, restored) = check(&Check { place: &place, network, keys: &opened, root: Some(root) }).unwrap();
        assert_eq!(report["problems"], json!([]), "{report}");
        assert_eq!((report["network"].clone(), report["users"].clone()), (json!("Home"), json!(["Andre"])), "who's who from its steps");
        let snapshot = &restored[0].snapshot;
        assert_eq!(restored[0].blobs, [(w["blob_hash"].as_str().unwrap().to_string(), b(&w["blob"]))], "a blob");
        let databases: Value = serde_json::to_value(&snapshot.databases).unwrap();
        assert_eq!(databases, w["snapshot_databases"], "what the snapshot holds");
        assert_eq!(databases["tally/main"], json!([w["at"], w["copy"]]));

        let epochs = Keys::from([(1, key(&w["epoch"]))]);
        assert_eq!(check_databases(&place, network, snapshot, &epochs, None::<&Place>).await, Vec::<String>::new(), "every database restores to its last step");
    }
}

pub fn fixtures() -> Vec<Value> {
    written()
}
