//! The `push` ops (`crate::push`), which every node answers itself for any caller, through the install's push Worker:
//! never a service a device runs.

use crate::config::YOURS;
use super::*;
use data_encoding::BASE64URL_NOPAD as B64U;

use crate::push::{self, Send};

impl Node {
    /// Runs a push op for `me`: from app `from`, if an app asked.
    pub(super) async fn push(&self, op: &str, input: &Value, me: &Caller, from: Option<&str>) -> Result<Value> {
        let text = |name: &str| input[name].as_str().unwrap_or_default().to_string();
        match op {
            "send" => {
                let note = crate::services::input::<Send>(input)?.note(from.unwrap_or(""))?;
                self.push_note(&me.user, &note).await
            }
            // A phone from before notes, by its APNs token: its person's, until it registers itself.
            "register" | "unregister" => {
                let token = text("token").to_lowercase();
                ensure!((16..=200).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_hexdigit()), "a device token is hex");
                let name = Some(text("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| self.user_name(&me.user));
                let env = if text("env") == "sandbox" { "sandbox" } else { "production" };
                self.ask_push(&format!("/push/{op}"), json!({ "legacy": { "user": me.user, "token": token, "env": env, "name": name } })).await?;
                Ok(json!(if op == "register" { "Registered for push." } else { "Unregistered." }))
            }
            "devices" => {
                let mine = self.devices_of(&me.user)?;
                let said = self.ask_push("/push/devices", json!({ "user": me.user, "keys": mine.iter().map(|(k, _, _)| hex(k)).collect::<Vec<_>>() })).await?;
                let name = |key: &str| mine.iter().find(|(k, _, _)| hex(k) == key).map(|(_, _, n)| n.clone()).unwrap_or_default();
                let mut out: Vec<Value> = said["registered"].as_array().into_iter().flatten().map(|d| json!({ "name": name(d["key"].as_str().unwrap_or("")), "via": d["via"], "since": d["at"] })).collect();
                out.extend(said["legacy"].as_array().into_iter().flatten().map(|p| json!({ "name": p["name"], "via": "apns", "since": p["at"], "old": true })));
                Ok(json!(out))
            }
            "status" => {
                let address = self.address()?;
                let said = tokio::task::spawn_blocking(move || -> Result<Value> { Ok(agent().get(format!("{address}/push/key")).call()?.body_mut().read_json()?) }).await??;
                Ok(json!({ "via": said["via"] }))
            }
            op => bail!("push has no operation {op}"),
        }
    }

    /// Pushes `note` to each of `user`'s devices, sealed to each, and to their phones from before notes: → how many it
    /// went to, and how many of their devices get pushes.
    pub(super) async fn push_note(&self, user: &str, note: &push::Note) -> Result<Value> {
        let key = self.signing_key();
        let now = now_ms();
        let notes: Vec<Value> = self
            .devices_of(user)?
            .into_iter()
            .map(|(to, seal, _)| Ok(json!({ "to": hex(&to), "note": B64U.encode(&push::seal_note(&key, &seal, note, now)?) })))
            .collect::<Result<_>>()?;
        let link = (!note.app.is_empty()).then(|| json!({ "app": note.app, "screen": note.screen, "params": note.params.clone().unwrap_or_default() }));
        let legacy = json!({ "user": user, "title": note.title, "body": note.body, "link": link, "badge": note.badge });
        let said = self.ask_push("/push/send", json!({ "notes": notes, "legacy": legacy })).await?;
        let to = said["to"].as_object().cloned().unwrap_or_default();
        let sent = to.values().filter(|v| v["sent"] == true).count() as u64 + said["legacy"]["sent"].as_u64().unwrap_or(0);
        let devices = to.values().filter(|v| v["none"].is_null() && v["gone"].is_null()).count() as u64 + said["legacy"]["phones"].as_u64().unwrap_or(0);
        let mut out = json!({ "sent": sent, "devices": devices });
        if devices == 0 {
            out["why"] = json!("none of your devices get notifications yet: allow them in the app or the browser");
        } else if let Some(why) = to.values().find_map(|v| v["failed"].as_str()).or(said["legacy"]["failed"].as_str()) {
            out["why"] = json!(why);
        }
        Ok(out)
    }

    /// A person's own devices that might get pushes, as the install's signed entries list them: (key, seal key, name).
    fn devices_of(&self, user: &str) -> Result<Vec<(Key, Key, String)>> {
        self.with(|i| Ok(i.r.db().clients().into_iter().filter(|m| m.user == user && !m.removed && m.passkey.is_none()).map(|m| (m.key, m.seal, m.name)).collect()))
    }

    fn user_name(&self, user: &str) -> String {
        self.with(|i| Ok(i.r.db().user(user).map(|u| u.name))).ok().flatten().unwrap_or_default()
    }

    /// Asks the push Worker something as this device, signed with its key (`one/push`). → its answer.
    async fn ask_push(&self, path: &str, mut fields: Value) -> Result<Value> {
        let (address, key, path) = (self.address()?, self.signing_key(), path.to_string());
        tokio::task::spawn_blocking(move || {
            fields["route"] = json!(format!("POST {path}"));
            let signed = crate::keys::signed_request(&key, push::DOMAIN, fields, crate::keys::next_at(now_ms()));
            let mut resp = agent().post(format!("{address}{path}")).send_json(&signed)?;
            let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
            ensure!(resp.status().is_success(), "{}", out["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("{YOURS}'s push service didn't answer")));
            Ok(out)
        })
        .await?
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into()
}
