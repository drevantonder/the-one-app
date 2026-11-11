//! Services carried by their device's app runtime (`services::carried`): a node asks a service through the runtime of
//! the device it's part of, which hands the sealed ask to the part over its socket (`Parts`, long-polled on the
//! loopback) and passes back its sealed answer, reading neither; and a part's own asks (a call, what insights keeps)
//! come to this device's runtime, signed by it.

use crate::services::carried::{self, Carried};

use super::*;

impl Node {
    /// What each part of this device registered as, by its service's key (hex): what this device answers another that asks what it runs.
    pub(in crate::node) fn parts_described(&self) -> serde_json::Map<String, Value> {
        let device = self.0.holds.device();
        let registered = self.0.sockets.described();
        self.with_or(Default::default(), |i| {
            let m = i.r.db();
            let mut out = serde_json::Map::new();
            for (name, describe) in registered {
                for s in m.offering(&name)?.into_iter().filter(|s| s.device == Some(device)) {
                    out.insert(hex(&s.key), describe.clone());
                }
            }
            Ok(out)
        })
    }

    /// Asks the service `service` the call `call`, through the app runtime of the device it's part of (this one's own
    /// sockets, if it's here), waiting `limit`: → its answer (`{ ok }`, `{ error }` or `{ elsewhere }`).
    pub(in crate::node) async fn ask_service(&self, service: Key, call: Value, limit: Duration) -> Result<Value> {
        let (name, device, endpoint, runtime) = self.with(|i| {
            let m = i.r.db();
            let s = m.service(&service)?.with_context(|| format!("{} isn't a service of {YOURS}", short(&service)))?;
            let device = s.device.with_context(|| format!("{} isn't a part of a device", short(&service)))?;
            Ok((s.service.unwrap_or_default(), device, crate::keys::Endpoint { key: s.key, seal: s.seal }, m.runtime_for(&device)?))
        })?;
        let (sealed, at) = carried::ask(&self.0.key, &endpoint, json!({ "call": call, "traceparent": trace::traceparent() }))?;
        let ask = carried::b64(&sealed);
        let carried = if device == self.0.holds.device() {
            tokio::time::timeout(limit, self.0.sockets.hand(&name, |id| Carried { id, ask })).await.context("timed out")?
        } else {
            let runtime = runtime.with_context(|| format!("{}'s device has no app runtime to reach it through", short(&service)))?;
            self.send(&runtime, crate::protocol::message::SERVICE, &json!({ "service": hex(&service), "ask": ask }), true, limit).await
        }?;
        if let Some(err) = carried["error"].as_str() {
            bail!(crate::protocol::Coded::or_plain(carried["code"].as_str().map(str::to_string), err));
        }
        carried::open_answer(&self.0.key, &service, at, &carried::unb64(carried["answer"].as_str().context("its runtime carried back no answer")?)?)
    }

    /// Hands a part of this device an ask another device carried here (`{ service, ask }`): → the part's sealed
    /// `{ answer }`, or `{ error }` if it couldn't be handed.
    pub(in crate::node) async fn carry(&self, body: &Value) -> Value {
        let handed = async {
            let service = crate::keys::unhex(body["service"].as_str().unwrap_or(""))?;
            let name = self.with(|i| {
                let s = i.r.db().service(&service)?.with_context(|| format!("{} isn't a service of {YOURS}", short(&service)))?;
                ensure!(s.device == Some(self.0.holds.device()), "{} isn't a part of {}", s.service.as_deref().unwrap_or("that service"), self.device_name());
                Ok(s.service.unwrap_or_default())
            })?;
            let ask = body["ask"].as_str().context("it carries no ask")?.to_string();
            self.0.sockets.hand(&name, |id| Carried { id, ask }).await
        }
        .await;
        handed.unwrap_or_else(|err| json!({ "error": format!("{err:#}"), "code": crate::protocol::Coded::of(&err) }))
    }

    /// A part of this device asking, signed by its key (`carried::own_ask`): `{ call }`, run as its user (a service
    /// writes nothing itself), or `{ insights }`, what it says for Insights. → the call's answer.
    pub(in crate::node) async fn part_asked(&self, signed: &Value) -> Result<Value> {
        let (by, body, _) = carried::signed_by(signed, &self.0.holds.device(), &self.0.part_asks)?;
        self.with(|i| {
            let s = i.r.db().service(&by)?.context("not a service of this network")?;
            ensure!(s.device == Some(self.0.holds.device()), "only this device's own parts ask it");
            Ok(())
        })?;
        if body["call"].is_object() {
            return Ok(self.accept_service_call(by, &body["call"]).await.json());
        }
        self.service_said(&by, &body["insights"]).await;
        Ok(json!({}))
    }

    /// `global`'s scope database's steps after position `from`, opened, for this device's parts, which hold no keys
    /// and check each from the network's first freeze themselves: `{ steps: [{ at, step }] }`, each base64url.
    pub(in crate::node) async fn steps_for_parts(&self, from: u64) -> Result<Value> {
        let at = self.with(|i| Ok(i.r.at()))?.unwrap_or(0);
        if from >= at {
            return Ok(json!({ "steps": [] }));
        }
        let (bucket, keys) = (self.bucket()?, self.step_keys(&Scope::Global).await?);
        let made = self.with(|i| Ok(i.r.db().epoch(&Scope::Global)?))?;
        let steps = crate::database::steps::read(&bucket, &keys, &Scope::Global, from, made).await?.steps;
        let list: Vec<Value> = steps.iter().enumerate().map(|(n, s)| json!({ "at": from + n as u64 + 1, "step": s.as_ref().map(|s| data_encoding::BASE64URL_NOPAD.encode(&crate::database::steps::encoded(s))) })).collect();
        Ok(json!({ "steps": list }))
    }
}
