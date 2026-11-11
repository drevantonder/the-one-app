//! A service carried by its device's runtime: what a hacked runtime carrying it can't do. Here, not in scope's own
//! tests, since it needs services' sealing of asks and oned's check of who may call, which scope sits below.

use ed25519_dalek::SigningKey;
use serde_json::json;

use ::key::{Endpoint, Hash, Key};
use ::scope::content::{Content, device, first, listed_service};
use ::scope::scopes::{Kind, Role, Scope, ScopeDb};
use ::scope::steps;
use one::oned::part::may_call;
use services::carried;

const ANDRE: &str = "a000000000000001";
const BOB: &str = "b000000000000002";
const BOTH: &[Kind] = &[Kind::Client, Kind::Compute];

struct Dev(SigningKey);

impl Dev {
    fn new(seed: u8) -> Self {
        Dev(SigningKey::from_bytes(&[seed; 32]))
    }
    fn id(&self) -> Key {
        self.0.verifying_key().to_bytes()
    }
    fn endpoint(&self) -> Endpoint {
        Endpoint::of(&self.0)
    }
}

/// `global`'s scope database, each step applied as its readers apply it: those that don't count are passed over.
struct Net {
    db: ScopeDb,
    at: u64,
}

impl Net {
    /// A network whose first device is `mac`, Andre's client and node.
    fn new(mac: &Dev) -> Net {
        let start = first("Home", ANDRE, "Andre", &mac.endpoint(), "mac", BOTH, &[7; 32], &"e".repeat(64));
        Net { db: steps::first(Hash::from_bytes([5; 32]), &start, 0).unwrap(), at: 1 }
    }

    /// Writes `content` as `user` from `dev`: whether it counted.
    fn step(&mut self, dev: &Dev, user: &str, content: Content) -> bool {
        self.at += 1;
        let s = steps::sign(&dev.0, &Scope::Global, self.at, user, 1, self.at, content).unwrap();
        match steps::apply(&self.db, None, &Scope::Global, &s) {
            Ok(a) => {
                self.db = ScopeDb::open(a.db, 0).unwrap();
                true
            }
            Err(_) => false,
        }
    }
}

/// A service carried by its device's runtime takes only asks sealed to it and signed by a node of the network, each
/// once, and a node calls it only as its own user: a hacked runtime carrying it can't forge, replay or read an ask,
/// nor call as anyone else.
#[test]
fn a_carried_ask_is_taken_once_from_whoever_signed_it_and_only_as_their_own_user() -> anyhow::Result<()> {
    let mac = Dev::new(1);
    let mut net = Net::new(&mac);
    let (bobs, linux, bobs_linux, part) = (Dev::new(3), Dev::new(4), Dev::new(6), Dev::new(8));
    net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
    net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", BOTH, None));
    net.step(&bobs, BOB, device(&bobs_linux.endpoint(), BOB, "bob's linux", BOTH, None));
    assert!(net.step(&linux, ANDRE, listed_service(&part.endpoint(), ANDRE, "linux", "shell", &linux.id())));
    let once = carried::Once::default();
    let call = json!({ "function": "run" });

    // Andre's mac asks it: opened, as the mac's, once.
    let (sealed, _) = carried::ask(&mac.0, &part.endpoint(), json!({ "call": call }))?;
    let (by, body, _) = carried::open_ask(&part.0, &sealed, &once)?;
    assert_eq!((by, &body["call"]), (mac.id(), &call));
    assert!(carried::open_ask(&part.0, &sealed, &once).is_err(), "a replayed ask was taken");
    // A byte changed, or sealed to someone else: refused.
    let (mut forged, _) = carried::ask(&mac.0, &part.endpoint(), json!({ "call": call }))?;
    let last = forged.len() - 1;
    forged[last] ^= 1;
    assert!(carried::open_ask(&part.0, &forged, &once).is_err(), "a changed ask was taken");
    let (elsewhere, _) = carried::ask(&mac.0, &bobs.endpoint(), json!({ "call": call }))?;
    assert!(carried::open_ask(&part.0, &elsewhere, &once).is_err(), "an ask sealed to another was opened");

    // Andre's node calls as Andre; Bob's node, carrying it or not, never as Andre, and Andre's shell serves only him.
    may_call(&net.db, &part.id(), &mac.id(), &mac.id(), ANDRE)?;
    assert!(may_call(&net.db, &part.id(), &bobs_linux.id(), &mac.id(), ANDRE).is_err(), "Bob's node called as Andre");
    assert!(may_call(&net.db, &part.id(), &bobs_linux.id(), &bobs_linux.id(), BOB).is_err(), "Andre's own service served Bob");
    assert!(may_call(&net.db, &part.id(), &bobs.id(), &bobs.id(), BOB).is_err(), "a client that isn't a node was answered");
    Ok(())
}
