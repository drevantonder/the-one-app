//! The Vault: items, each in one scope, with named fields (secret ones set, never read back), the hosts its secret
//! fields go to, and the apps switched on for it. A scope's items are rows of the node's database `vault` there,
//! sealed under the scope's epoch like any data; and in each, its secret fields' values, its hosts and its apps are
//! sealed again to the scope's Vault key (`Sealed`), whose secret half only the machines its admins grant hold
//! (`keys::vault_key_name`). So a member who opens the scope adds an item and never reads one back, and nobody who
//! can't open an item changes where it goes or who uses it. Apps write `{{<item id>:field}}` (or `{{field}}`) where a
//! field goes, and a granted machine fills it in on the way out: into a web request to one of its hosts, or into the
//! input of a call to a service whose name is one of them.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::entry::{Scope, is_id};

/// The node's database of a scope's items.
pub const DATABASE: &str = "vault";

/// Its schema: each item as one row, its fields' values with it.
pub const SCHEMA: &str = "create table items (id text primary key, item text not null)";

/// A scope's items, by id.
pub type Items = BTreeMap<String, Item>;

/// An item: title, fields, hosts and the apps switched on for it.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Item {
    pub title: String,
    pub kind: String,
    pub domain: Option<String>,
    pub fields: BTreeMap<String, Field>,
    pub hosts: Vec<String>,
    pub apps: BTreeSet<String>,
    /// The app whose call made it (a sign-in's account), if one did.
    pub app: Option<String>,
    /// Who changed it last, and when; and when it was made.
    pub user: String,
    pub time: u64,
    pub made: u64,
    /// Its secret fields' values, hosts and apps, sealed to the scope's Vault key `n`: what counts of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<Sealed>,
    /// Whether this machine opened it, or it has nothing sealed: only then are its secrets, hosts and apps its own.
    #[serde(skip)]
    pub opened: bool,
}

/// What's sealed of an item: the number of the scope's Vault key it's sealed to, and `keys`'s `seal_to` of its
/// `Secrets` to that key's public half, base64url.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sealed {
    pub n: u32,
    pub sealed: String,
}

/// What an item's box holds: its id (so it opens as no other item), its secret fields' values, its hosts and apps.
#[derive(Serialize, Deserialize)]
struct Secrets {
    id: String,
    values: BTreeMap<String, String>,
    hosts: Vec<String>,
    apps: BTreeSet<String>,
}

/// How a write seals what it changes: with the Vault secret keys this machine holds, by number (only if it's granted
/// now), and the scope's newest public key, if it has one.
pub struct Sealing<'a> {
    pub keys: Option<&'a BTreeMap<u32, [u8; 32]>>,
    pub public: Option<(u32, [u8; 32])>,
    /// The scope's name and the machines that open its Vault, as a person reads them.
    pub scope: &'a str,
    pub machines: &'a str,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Field {
    pub value: String,
    pub secret: bool,
}

impl std::fmt::Debug for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let fields: Vec<String> = self.fields.iter().map(|(n, v)| if v.secret { format!("{n}: ••••") } else { format!("{n}: {:?}", v.value) }).collect();
        write!(f, "Item {{ {:?}, fields: [{}], hosts: {:?}, apps: {:?} }}", self.title, fields.join(", "), self.hosts, self.apps)
    }
}

/// A field's name, as a placeholder names it: letters, digits, `_`, `-` and `.`.
pub fn valid_field(name: &str) -> bool {
    crate::members::valid_secret_name(name)
}

/// Applies a change to an item (`{ id, title?, type?, domain?, fields?, hosts?, apps?, app?, deleted? }`), made by
/// `user` at `time`, to `items`. `known` says whether an app name may be switched on (it isn't erased). Fails,
/// changing nothing, if it doesn't make sense.
pub fn apply(items: &mut BTreeMap<String, Item>, data: &Value, user: &str, time: u64, known: impl Fn(&str) -> bool) -> Result<String> {
    let id = data["id"].as_str().filter(|id| is_id(id)).context("an item's id is 16 hex characters")?.to_string();
    if data["deleted"] == true {
        ensure!(items.remove(&id).is_some(), "no such item");
        return Ok(id);
    }
    let mut item = match items.get(&id) {
        Some(item) => item.clone(),
        None => Item { made: time, app: data["app"].as_str().map(str::to_string), ..Default::default() },
    };
    if let Some(title) = data["title"].as_str() {
        item.title = title.trim().to_string();
    }
    ensure!(!item.title.is_empty(), "an item has a title");
    if let Some(kind) = data["type"].as_str() {
        item.kind = kind.trim().to_string();
    }
    if !data["domain"].is_null() {
        item.domain = data["domain"].as_str().filter(|d| crate::members::valid_host(d)).map(str::to_string);
    }
    if let Some(fields) = data["fields"].as_object() {
        let before = item.fields.clone();
        for (name, field) in fields {
            ensure!(valid_field(name), "a field's name is letters, digits, _, - and .");
            if field.is_null() {
                item.fields.remove(name);
                continue;
            }
            let value = match field["from"].as_str() {
                Some(from) => before.get(from).with_context(|| format!("no field {from} to take"))?.value.clone(),
                None => field["value"].as_str().context("a field's value is text")?.to_string(),
            };
            let was = before.get(name).map(|f| f.secret).or_else(|| field["from"].as_str().and_then(|from| before.get(from)).map(|f| f.secret));
            let secret = field["secret"].as_bool().or(was).unwrap_or(false);
            item.fields.insert(name.clone(), Field { value, secret });
        }
    }
    if let Some(hosts) = data["hosts"].as_array() {
        item.hosts = hosts.iter().filter_map(|h| h.as_str().map(str::to_string)).collect();
        ensure!(item.hosts.iter().all(|h| crate::members::valid_host(h)), "a host is a host name, like api.example.com");
    }
    ensure!(!item.hosts.is_empty() || !item.fields.values().any(|f| f.secret), "an item with a secret field is sent to one host or more");
    if let Some(apps) = data["apps"].as_array() {
        item.apps = apps.iter().filter_map(|a| a.as_str()).filter(|a| known(a)).map(str::to_string).collect();
    }
    item.user = user.to_string();
    item.time = time;
    items.insert(id.clone(), item);
    Ok(id)
}

/// A scope's items, as its database has them.
pub fn load(conn: &Connection) -> Result<Items> {
    let mut out = Items::new();
    for row in crate::database::stored::query(conn, "select id, item from items", &[])? {
        let (Some(id), Some(item)) = (row["id"].as_str(), row["item"].as_str()) else { continue };
        let mut item: Item = serde_json::from_str(item).context("an item that isn't one")?;
        item.opened = !item.fields.values().any(|f| f.secret);
        out.insert(id.to_string(), item);
    }
    Ok(out)
}

/// Opens each item sealed to a Vault key in `keys`: its secret fields' values, hosts and apps become the sealed ones.
/// One that doesn't open, or opens as another item, stays shut.
pub fn open_all(items: &mut Items, keys: &BTreeMap<u32, [u8; 32]>) {
    for (id, item) in items.iter_mut() {
        let Some(sealed) = &item.sealed else { continue };
        let Some(key) = keys.get(&sealed.n) else { continue };
        let opened = data_encoding::BASE64URL_NOPAD.decode(sealed.sealed.as_bytes()).ok().and_then(|b| crate::cloudflare::open_sealed(key, &b).ok()).and_then(|b| serde_json::from_slice::<Secrets>(&b).ok());
        let Some(s) = opened.filter(|s| s.id == *id) else {
            tracing::warn!("{:?}'s secrets didn't open with {}'s Vault key {}", item.title, id, sealed.n);
            continue;
        };
        for (name, f) in item.fields.iter_mut().filter(|(_, f)| f.secret) {
            f.value = s.values.get(name).cloned().unwrap_or_default();
        }
        (item.hosts, item.apps, item.opened) = (s.hosts, s.apps, true);
    }
}

/// Seals an item's secrets, hosts and apps to the Vault key `n` (`public`).
fn seal_item(id: &str, item: &mut Item, n: u32, public: &[u8; 32]) -> Result<()> {
    let values = item.fields.iter().filter(|(_, f)| f.secret).map(|(n, f)| (n.clone(), f.value.clone())).collect();
    let s = Secrets { id: id.to_string(), values, hosts: item.hosts.clone(), apps: item.apps.clone() };
    item.sealed = Some(Sealed { n, sealed: data_encoding::BASE64URL_NOPAD.encode(&crate::cloudflare::seal_to(public, &serde_json::to_vec(&s)?)?) });
    Ok(())
}

/// An item as its row keeps it: its secret fields' values only in its box.
fn row(item: &Item) -> Result<String> {
    let mut item = item.clone();
    for f in item.fields.values_mut().filter(|f| f.secret) {
        f.value = String::new();
    }
    Ok(serde_json::to_string(&item)?)
}

/// Applies a change (as `apply`) to a scope's database, inside a write, sealing what it changes (`sealing`): →
/// the item's id. An item with secret fields is sealed anew, to the scope's newest Vault key, only once every secret
/// value is known: given in this change, or opened here. Otherwise only its title, type, domain and plain fields
/// change, so nobody who can't open it changes where it goes or who uses it. A machine granted the newest key seals
/// each item it opened under an older one to it too.
pub fn change(conn: &Connection, data: &Value, user: &str, time: u64, known: impl Fn(&str) -> bool, sealing: &Sealing) -> Result<String> {
    let mut items = load(conn)?;
    if let Some(keys) = sealing.keys {
        open_all(&mut items, keys);
    }
    let before = items.get(data["id"].as_str().unwrap_or("")).cloned();
    let id = apply(&mut items, data, user, time, known)?;
    let given = |name: &str| data["fields"][name]["value"].is_string();
    if let Some(item) = items.get_mut(&id) {
        let secrets: Vec<String> = item.fields.iter().filter(|(_, f)| f.secret).map(|(n, _)| n.clone()).collect();
        let open = before.as_ref().is_some_and(|b| b.opened);
        // Each secret value: given now, or there before and known here (opened, or a plain field made secret).
        let all_known = secrets.iter().all(|n| given(n) || before.as_ref().and_then(|b| b.fields.get(n)).is_some_and(|f| open || !f.secret));
        match (secrets.is_empty(), all_known) {
            (true, _) => (item.sealed, item.opened) = (None, true),
            (false, true) => {
                let (n, public) = sealing.public.with_context(|| format!("No machine opens {}’s Vault yet, so nothing secret can be kept there: its admins grant one with one vault grant <machine>.", sealing.scope))?;
                seal_item(&id, item, n, &public)?;
                item.opened = true;
            }
            (false, false) => {
                // Shut here: its box stays as it was, and what's sealed in it with it.
                let b = before.as_ref().context("a secret field's value is given when it's made")?;
                let moved = !data["hosts"].is_null() || !data["apps"].is_null() || secrets.len() != b.fields.values().filter(|f| f.secret).count();
                ensure!(!moved, "Only a machine that opens {}’s Vault ({}) changes where {} is sent, which apps use it, or its secret fields, unless every secret field is given again.", sealing.scope, sealing.machines, b.title);
                (item.sealed, item.hosts, item.apps) = (b.sealed.clone(), b.hosts.clone(), b.apps.clone());
            }
        }
    }
    // Sealed anew to the newest key, each this machine opened under an older one.
    if let (Some(_), Some((n, public))) = (sealing.keys, sealing.public) {
        for (other, item) in items.iter_mut().filter(|(i, it)| **i != id && it.opened && it.sealed.as_ref().is_some_and(|s| s.n < n)) {
            seal_item(other, item, n, &public)?;
            crate::database::stored::execute(conn, "insert or replace into items values (?, ?)", &[json!(other), json!(row(item)?)])?;
        }
    }
    match items.get(&id) {
        Some(item) => crate::database::stored::execute(conn, "insert or replace into items values (?, ?)", &[json!(id), json!(row(item)?)])?,
        None => crate::database::stored::execute(conn, "delete from items where id = ?", &[json!(id)])?,
    };
    Ok(id)
}

/// The items a call of `app` for `user` may fill its placeholders from: those switched on for it in the scopes the
/// call reached, each opened with that scope's own key. An erased app's name is never switched on.
pub struct Lookup {
    pub app: String,
    pub user: String,
    pub scopes: Vec<(Scope, Items)>,
}

impl Lookup {
    fn items(&self, scope: &Scope) -> impl Iterator<Item = (&String, &Item)> {
        self.scopes.iter().filter(move |(s, _)| s == scope).flat_map(|(_, items)| items.iter())
    }

    fn oldest<'a>(&'a self, scope: &Scope, field: &'a str) -> Option<(&'a Item, &'a str)> {
        let found = self.items(scope).filter(|(_, i)| i.apps.contains(&self.app) && i.fields.contains_key(field)).min_by_key(|(_, i)| i.made)?;
        Some((found.1, field))
    }

    /// The item and field a placeholder names: `<item id>:field` of an item in any of the scopes, `field` of the
    /// oldest item that has it in the user's own scope, else in `global`, or `<group id>/field` in that group's;
    /// always of an item the app is switched on for.
    fn resolve<'a>(&'a self, name: &'a str) -> Option<(&'a Item, &'a str)> {
        match (name.split_once(':'), name.split_once('/')) {
            (Some((id, field)), _) => self.scopes.iter().find_map(|(_, items)| items.get(id)).filter(|it| it.apps.contains(&self.app)).map(|it| (it, field)),
            (None, Some((group, field))) => self.oldest(&Scope::Group(group.to_string()), field),
            (None, None) => self.oldest(&Scope::User(self.user.clone()), name).or_else(|| self.oldest(&Scope::Global, name)),
        }
    }

    /// What a placeholder names (`resolve`), as (hosts, value; no hosts for a plain field), if it's opened here.
    pub fn get(&self, name: &str) -> Option<(Vec<String>, String)> {
        self.resolve(name).and_then(|(it, field)| filled(it, field))
    }

    /// Refuses `text` if a placeholder in it names a secret field this machine can't open: it isn't granted the
    /// item's scope's Vault, so it can't fill it in.
    pub fn refuse_shut(&self, text: &str) -> Result<()> {
        for name in placeholders(text) {
            if let Some((it, field)) = self.resolve(name)
                && it.fields.get(field).is_some_and(|f| f.secret)
                && !it.opened
            {
                bail!("{name} is opened only by the machines its scope's admins grant its Vault (one vault grant), and this isn't one");
            }
        }
        Ok(())
    }

    /// The items the app is switched on for, as its calls see them (`ctx.vault.items()`): the user's own first, then
    /// their groups', then `global`'s, each scope's oldest first.
    pub fn for_app(&self) -> Vec<Value> {
        let mut scopes: Vec<&(Scope, Items)> = self.scopes.iter().collect();
        scopes.sort_by_key(|(s, _)| match s {
            Scope::User(_) => 0,
            Scope::Group(_) => 1,
            Scope::Global => 2,
        });
        let mut out = Vec::new();
        for (scope, items) in scopes {
            let mut mine: Vec<(&String, &Item)> = items.iter().filter(|(_, it)| it.apps.contains(&self.app)).collect();
            mine.sort_by_key(|(_, it)| it.made);
            out.extend(mine.into_iter().map(|(id, it)| for_app(id, &scope.to_string(), it)));
        }
        out
    }
}

/// What a placeholder gives: the field's value, and the hosts it may go to (none for a plain field: anywhere).
pub fn filled(item: &Item, field: &str) -> Option<(Vec<String>, String)> {
    let f = item.fields.get(field).filter(|f| !f.secret || item.opened)?;
    Some((if f.secret { item.hosts.clone() } else { Vec::new() }, f.value.clone()))
}

/// An item as an app sees it (`ctx.vault.items()`): plain fields' values, and which are secret.
pub fn for_app(id: &str, scope: &str, item: &Item) -> Value {
    let fields: serde_json::Map<String, Value> = item.fields.iter().map(|(n, f)| (n.clone(), if f.secret { json!({ "secret": true }) } else { json!({ "value": f.value }) })).collect();
    json!({ "id": id, "scope": scope, "title": item.title, "type": item.kind, "domain": item.domain, "hosts": item.hosts, "fields": fields, "app": item.app, "time": item.made })
}

/// An item's fields as a person sees them: each name, whether it's secret, and a plain one's value.
pub fn fields_shown(item: &Item) -> Vec<Value> {
    item.fields.iter().map(|(n, f)| if f.secret { json!({ "name": n, "secret": true }) } else { json!({ "name": n, "secret": false, "value": f.value }) }).collect()
}

/// Fills each `{{<item id>:field}}`, `{{field}}` or `{{<group id>/field}}` in `text` with what `lookup` finds for
/// it, as `(hosts, value)`; refuses when a secret field isn't sent to `host`. A placeholder nothing has stays as it is.
/// `{{rs256 <placeholder> <claims>}}` is a JWT of `claims` (base64url JSON) signed with the PEM private key the
/// placeholder names, for a site that takes one instead of the key: sent only to the key's hosts, as the key would be.
pub fn fill(text: &str, host: &str, lookup: impl Fn(&str) -> Option<(Vec<String>, String)>) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        out.push_str(&rest[..start]);
        let name = &after[..end];
        let (key, claims) = match name.strip_prefix("rs256 ").and_then(|r| r.split_once(' ')) {
            Some((key, claims)) => (key, Some(claims)),
            None => (name, None),
        };
        match placeholder(key).then(|| lookup(key)).flatten() {
            Some((hosts, value)) => {
                if !hosts.is_empty() && !hosts.iter().any(|h| h == host) {
                    bail!("{key} is sent only to {}, not {host}", hosts.join(", "));
                }
                match claims {
                    Some(claims) => {
                        ensure!(!hosts.is_empty(), "{key} isn't secret, so it signs nothing");
                        out.push_str(&rs256(&value, claims)?);
                    }
                    None => out.push_str(&value),
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// A JWT of `claims` (base64url JSON), signed RS256 with `pem`, an RSA private key (PKCS#1 or PKCS#8), on the device
/// that holds it: never in a browser or the Cloudflare node.
#[cfg(feature = "node")]
fn rs256(pem: &str, claims: &str) -> Result<String> {
    use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
    ensure!(!claims.is_empty() && claims.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "a JWT's claims go as base64url JSON");
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect::<Vec<_>>().concat();
    let der = data_encoding::BASE64.decode(body.trim().as_bytes()).context("that key isn't a PEM private key")?;
    let key = if pem.contains("BEGIN RSA PRIVATE KEY") { RsaKeyPair::from_der(&der) } else { RsaKeyPair::from_pkcs8(&der) }.map_err(|e| anyhow::anyhow!("that key isn't an RSA private key: {e}"))?;
    let signing = format!("{}.{claims}", data_encoding::BASE64URL_NOPAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#));
    let mut sig = vec![0; key.public().modulus_len()];
    key.sign(&RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), signing.as_bytes(), &mut sig).map_err(|_| anyhow::anyhow!("signing with that key failed"))?;
    Ok(format!("{signing}.{}", data_encoding::BASE64URL_NOPAD.encode(&sig)))
}
#[cfg(not(feature = "node"))]
fn rs256(_: &str, _: &str) -> Result<String> {
    bail!("a key in the Vault signs only on one of your devices: turn one on, and try again")
}

/// The names of the placeholders in `text`, a JWT's key's for `rs256 …`.
fn placeholders(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let name = &after[..end];
        let name = name.strip_prefix("rs256 ").and_then(|r| r.split_once(' ')).map_or(name, |(key, _)| key);
        if placeholder(name) {
            out.push(name);
        }
        rest = &after[end + 2..];
    }
    out
}

/// Fills every text in `value` as `fill` fills one, for a call to a service (`host` is its name).
pub fn fill_value(value: &mut Value, host: &str, lookup: &impl Fn(&str) -> Option<(Vec<String>, String)>) -> Result<()> {
    match value {
        Value::String(text) if text.contains("{{") => *text = fill(text, host, lookup)?,
        Value::Array(all) => all.iter_mut().try_for_each(|v| fill_value(v, host, lookup))?,
        Value::Object(all) => all.values_mut().try_for_each(|v| fill_value(v, host, lookup))?,
        _ => {}
    }
    Ok(())
}

/// Blots each of `secrets` out of every text in `value`, wherever an answer echoes one back.
pub fn blot(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(text) => {
            for s in secrets.iter().filter(|s| !s.is_empty()) {
                if text.contains(s.as_str()) {
                    *text = text.replace(s.as_str(), "••••");
                }
            }
        }
        Value::Array(all) => all.iter_mut().for_each(|v| blot(v, secrets)),
        Value::Object(all) => all.values_mut().for_each(|v| blot(v, secrets)),
        _ => {}
    }
}

/// Gives each field of an item's change that asks for one (`{ generate: true }`) a new random value, a secret one:
/// a key nobody, not even the app that asked, ever sees.
pub fn generate(data: &mut Value) {
    for field in data["fields"].as_object_mut().into_iter().flat_map(|f| f.values_mut()) {
        if field["generate"] == true {
            *field = json!({ "value": data_encoding::BASE64URL_NOPAD.encode(&rand::random::<[u8; 32]>()), "secret": true });
        }
    }
}

/// Whether `name` could be a placeholder: `field`, `<group id>/field` or `<item id>:field`.
fn placeholder(name: &str) -> bool {
    let field = match (name.split_once('/'), name.split_once(':')) {
        (Some((group, field)), None) if is_id(group) => field,
        (None, Some((item, field))) if is_id(item) => field,
        (None, None) => name,
        _ => return false,
    };
    valid_field(field)
}

/// The Vault is the node's, not an app's: agents list its items (never a secret field's value), set and remove
/// fields, switch apps on for items, and ask the person for one.
#[cfg(feature = "node")]
pub fn tools() -> Vec<Value> {
    use crate::skills::{confirm_input, hints};
    let scope = json!({ "type": "string", "description": "user (yours, the default), global (everyone's), or a group's name or id" });
    let item = json!({ "type": "string", "description": "the item's title or id" });
    let mut web = hints(false, false);
    web["openWorldHint"] = json!(true);
    vec![
        json!({ "name": "vault_list", "description": "The Vault's items you may change: title, type, fields (a secret one never with its value), the sites they're sent to (hosts), the apps switched on for each (apps) and those that want it (wanted_by), and app when an app's sign-in keeps it.", "inputSchema": { "type": "object", "properties": { "scope": scope } }, "annotations": hints(true, false) }),
        json!({ "name": "vault_set", "description": "Keep a key, token or password someone pasted (or any field) in an item, making the item if no item has that title. An app puts {{<item id>:<field>}} in a request and a machine that opens the Vault fills it in, a secret field only for the item's hosts; nobody reads a secret back, not you either. When the person has the value, use vault_request instead: never ask them to paste it into the chat.", "inputSchema": { "type": "object", "required": ["item", "field", "value"], "properties": { "item": item, "field": { "type": "string", "description": "the field's name, like token or password" }, "value": { "type": "string", "description": "the field's value" }, "secret": { "type": "boolean", "description": "never shown again; true for keys, passwords and tokens" }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names its secret fields are sent to, like api.example.com" }, "app": { "type": "string", "description": "an app to switch on for it" }, "scope": scope } }, "annotations": hints(false, false) }),
        json!({ "name": "vault_use", "description": "Switch an app on (or off) for an item, so its calls may use the item's fields. Only the person switches agents on (for vault_fetch): ask them with vault_request.", "inputSchema": { "type": "object", "required": ["item", "app"], "properties": { "item": item, "app": { "type": "string", "description": "the app's name, like gmail" }, "on": { "type": "boolean", "description": "false switches it off" }, "scope": scope } }, "annotations": hints(false, false) }),
        json!({ "name": "vault_grant", "description": format!("Let a machine of {} open a Vault's secrets and fill them into requests (or, with on false, no longer). Only the Vault's admins say which: the person for their own, a group's admins, the network's for everyone's. Nobody else reads a secret back, not even on their own device.", crate::config::YOURS), "inputSchema": { "type": "object", "required": ["device"], "properties": { "device": { "type": "string", "description": "the machine's name" }, "on": { "type": "boolean", "description": "false: it no longer opens it" }, "scope": scope } }, "annotations": hints(false, false) }),
        json!({ "name": "vault_remove", "annotations": hints(false, true), "description": "Delete an item, or with field, one of its fields.", "inputSchema": { "type": "object", "required": ["item"], "properties": { "item": item, "field": { "type": "string", "description": "only this field, by name; the whole item when not given" }, "scope": scope, "confirm": confirm_input() } } }),
        json!({ "name": "vault_request", "description": "Ask the person for a key, token, password or sign-in an app or agents may use, instead of handling its values yourself: need names what the app declares (its vault), or give title, fields and hosts. Returns a link to give them; it opens a sheet where they pick one of their items or make one (and their Settings lists it). You never see a secret. Tell the person; check vault_status when they say they've done it.", "inputSchema": { "type": "object", "properties": { "app": { "type": "string", "description": "the app that will use it; leave it out for one you use yourself with vault_fetch" }, "need": { "type": "string", "description": "what the app declares it needs, by name, like client for gmail; leave it out and give title, fields and hosts for anything else" }, "title": { "type": "string", "description": "the item's title, for one no app declares" }, "fields": { "type": "object", "description": "field name → { secret: true|false }", "additionalProperties": { "type": "object", "properties": { "secret": { "type": "boolean" } } } }, "hosts": { "type": "array", "items": { "type": "string" }, "description": "host names its secret fields are sent to, like api.example.com" }, "why": { "type": "string", "description": "one line the person sees: what it's for and where to find it" }, "scope": scope } }, "annotations": hints(false, false) }),
        json!({ "name": "vault_fetch", "description": "Make an HTTP request with your person's Vault items filled in, without seeing them: write {{<field>}} (or {{<item id>:<field>}}) where a value goes, in url, headers or body, and a machine that opens the Vault fills it in from the items they let agents use, a secret field only for its item's sites. You get { status, headers, body }; a secret the site echoes back is blotted out. To get an item, ask with vault_request, leaving out app.", "inputSchema": { "type": "object", "required": ["url"], "properties": { "url": { "type": "string", "description": "the URL, with {{<field>}} where a value goes" }, "method": { "type": "string", "description": "GET by default, POST with a body" }, "headers": { "type": "object", "additionalProperties": { "type": "string" }, "description": "like { \"authorization\": \"Bearer {{token}}\" }" }, "body": { "type": "string", "description": "the request's body, with {{<field>}} where a value goes" } } }, "annotations": web }),
        json!({ "name": "vault_status", "description": "How a request for an item went: waiting, set, declined or expired, and who answered. Never a value.", "inputSchema": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string", "description": "the request's id, from vault_request" } } }, "annotations": hints(true, false) }),
    ]
}

#[test]
fn secrets_hosts_and_apps_open_only_with_the_vault_key() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(SCHEMA, []).unwrap();
    let secret: [u8; 32] = rand::random();
    let keys = BTreeMap::from([(1, secret)]);
    let public = Some((1, crate::cloudflare::public_of(&secret)));
    let member = Sealing { keys: None, public, scope: "Everyone's", machines: "linux" };
    let granted = Sealing { keys: Some(&keys), public, scope: "Everyone's", machines: "linux" };
    let id = "0123456789abcdef";
    // A member adds an item: sealed to the Vault key, so the row they can read has no secret.
    let made = json!({ "id": id, "title": "Key", "fields": { "user": { "value": "andre" }, "token": { "value": "s3cret", "secret": true } }, "hosts": ["api.example.com"], "apps": ["two"] });
    change(&conn, &made, "bob", 1, |_| true, &member).unwrap();
    let rows = crate::database::stored::query(&conn, "select item from items", &[]).unwrap();
    assert!(!rows[0]["item"].as_str().unwrap().contains("s3cret"), "the row has it only sealed");
    let shut = load(&conn).unwrap();
    let lookup = Lookup { app: "two".into(), user: "bob".into(), scopes: vec![(Scope::Global, shut.clone())] };
    assert_eq!(lookup.get("token"), None, "not opened without the Vault key");
    assert!(lookup.refuse_shut("Bearer {{token}}").is_err() && lookup.get("user").is_some(), "a plain field still fills");
    // Nobody who can't open it changes where it goes or which apps use it, though its title is anyone's to change.
    assert!(change(&conn, &json!({ "id": id, "hosts": ["evil.example.com"] }), "bob", 2, |_| true, &member).is_err());
    assert!(change(&conn, &json!({ "id": id, "apps": ["evil"] }), "bob", 2, |_| true, &member).is_err());
    change(&conn, &json!({ "id": id, "title": "Key, renamed" }), "bob", 2, |_| true, &member).unwrap();
    // Nor by rewriting its row: what's sealed counts.
    let mut row: Value = serde_json::from_str(crate::database::stored::query(&conn, "select item from items", &[]).unwrap()[0]["item"].as_str().unwrap()).unwrap();
    (row["hosts"], row["apps"]) = (json!(["evil.example.com"]), json!(["evil"]));
    crate::database::stored::execute(&conn, "update items set item = ?", &[json!(row.to_string())]).unwrap();
    let mut opened = load(&conn).unwrap();
    open_all(&mut opened, &keys);
    let it = &opened[id];
    assert_eq!((it.hosts.clone(), it.apps.iter().cloned().collect::<Vec<_>>(), it.title.as_str()), (vec!["api.example.com".to_string()], vec!["two".to_string()], "Key, renamed"));
    let lookup = Lookup { app: "two".into(), user: "bob".into(), scopes: vec![(Scope::Global, opened.clone())] };
    assert_eq!(fill("Bearer {{token}}", "api.example.com", |n| lookup.get(n)).unwrap(), "Bearer s3cret");
    assert!(fill("{{token}}", "evil.example.com", |n| lookup.get(n)).is_err(), "only to its hosts");
    // Another item's box passed off as this one's doesn't open.
    let other = "fedcba9876543210";
    change(&conn, &json!({ "id": other, "title": "Other", "fields": { "token": { "value": "x", "secret": true } }, "hosts": ["evil.example.com"] }), "bob", 3, |_| true, &member).unwrap();
    let mut items = load(&conn).unwrap();
    let theirs = items[other].sealed.clone();
    items.get_mut(id).unwrap().sealed = theirs;
    open_all(&mut items, &keys);
    assert!(!items[id].opened, "a box opens only as its own item");
    // A granted machine changes its hosts and apps, since it opens it.
    change(&conn, &json!({ "id": id, "hosts": ["api2.example.com"], "apps": ["two", "three"] }), "andre", 4, |_| true, &granted).unwrap();
    let mut items = load(&conn).unwrap();
    open_all(&mut items, &keys);
    assert_eq!((items[id].hosts.clone(), items[id].fields["token"].value.as_str()), (vec!["api2.example.com".to_string()], "s3cret"));
}

#[cfg(feature = "node")]
#[test]
fn a_key_signs_a_jwt_only_for_its_hosts_and_the_jwt_carries_no_key() {
    use ring::signature::{KeyPair, RSA_PKCS1_2048_8192_SHA256, RsaKeyPair, UnparsedPublicKey};
    // GitHub gives an App's key as PKCS#1; OpenSSL 3 makes PKCS#8 unless told otherwise.
    for traditional in [true, false] {
        let out = std::process::Command::new("openssl").arg("genrsa").args(traditional.then_some("-traditional")).arg("2048").output().unwrap();
        let pem = String::from_utf8(out.stdout).unwrap();
        let lookup = |n: &str| (n == "0123456789abcdef:private_key").then(|| (vec!["api.github.com".to_string()], pem.clone()));
        let claims = data_encoding::BASE64URL_NOPAD.encode(br#"{"iss":"1","iat":1,"exp":2}"#);
        let jwt = fill(&format!("Bearer {{{{rs256 0123456789abcdef:private_key {claims}}}}}"), "api.github.com", lookup).unwrap();
        let jwt = jwt.strip_prefix("Bearer ").unwrap();
        let (signed, sig) = jwt.rsplit_once('.').unwrap();
        assert!(signed.ends_with(&claims) && !jwt.contains("PRIVATE"));
        let der: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = data_encoding::BASE64.decode(der.as_bytes()).unwrap();
        let key = if traditional { RsaKeyPair::from_der(&der) } else { RsaKeyPair::from_pkcs8(&der) }.unwrap();
        UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, key.public_key().as_ref()).verify(signed.as_bytes(), &data_encoding::BASE64URL_NOPAD.decode(sig.as_bytes()).unwrap()).unwrap();
        assert!(fill(&format!("{{{{rs256 0123456789abcdef:private_key {claims}}}}}"), "evil.example.com", lookup).is_err(), "only to its hosts");
    }
}
