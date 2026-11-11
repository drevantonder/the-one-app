//! Calls from other devices: checking who passed one on, and the keys they handed with it.

use ::custom_build::allowed;
use super::*;

impl Node {
    /// Runs a call another device, `remote`, asked for. One passed on with keys runs on those keys alone, never this
    /// device's own, and only in scopes that node runs. Any other runs only from a client device of its caller, as that
    /// device's own: only then are this device's keys used, and only for its own user, whose device that is.
    pub(in crate) async fn accept_call(&self, remote: Key, request: &Value) -> Reply {
        match self.accept(remote, request) {
            Ok((mut req, me)) => {
                req.puts = match self.take_carried(&me, request) {
                    Ok(puts) => puts,
                    Err(err) => return Reply::failed(&err),
                };
                let reply = self.run(&req, &me).await;
                for hash in &req.puts {
                    self.drop_put(hash);
                }
                reply
            }
            Err(err) => Reply::failed(&err),
        }
    }

    /// Runs a call one of the network's services asked for, as its user, wherever it runs: a service writes nothing
    /// itself, and asks for the calls the program it offers makes (`/api/call` where it's hosted).
    pub(in crate) async fn accept_service_call(&self, remote: Key, request: &Value) -> Reply {
        match self.accept_service(remote, request) {
            // Its own service on another of its user's devices: sent there.
            Ok((req, me)) if req.device.is_some() => match self.route(req, me).await {
                Ok(ok) => Reply::Ok(ok),
                Err(err) => Reply::failed(&err),
            },
            Ok((req, me)) => self.run(&req, &me).await,
            Err(err) => Reply::failed(&err),
        }
    }

    /// What a service may have run, as `service::limited` says: only its own app, in its user's own scope, here on
    /// a node of that user's own, whose keys it is run with until grants land, calling no other app. A hacked
    /// service gets no more than its own app's tools for its own user.
    pub(super) fn accept_service(&self, remote: Key, request: &Value) -> Result<(CallRequest, Caller)> {
        let me = self.check_caller(remote, request)?;
        let (name, owner) = self.with(|i| Ok(i.r.db().service(&remote)?.map(|s| (s.service.clone().unwrap_or_default(), s.user.clone()))))?.context("not a service of this network")?;
        ensure!(me.key == remote && me.user == owner && owner == self.user()?, "a service's calls run only on its own user's node");
        let mut req: CallRequest = serde_json::from_value(one::oned::part::limited(&name, request)?)?;
        let resolved = self.resolve(req.clone());
        ensure!(resolved.app == name, "a service asks only for its own app, {name}");
        (req.from, req.depth) = (None, MAX_DEPTH);
        Ok((req, me))
    }

    pub(super) fn accept(&self, remote: Key, request: &Value) -> Result<(CallRequest, Caller)> {
        let mut me = self.check_caller(remote, request)?;
        let passed = request["keys"].is_string();
        if let Some(keys) = request["keys"].as_str() {
            // Passed on: run on the keys it was handed alone, never this device's own.
            me.credential = self.take_passed(&remote, &me.user, request, keys, true)?;
        }
        let mut req = call_in(request)?;
        let (own, serves) = self.with(|i| Ok((!passed && i.r.db().client(&remote)?.is_some_and(|c| c.user == me.user), i.r.db().serves(&remote, &me.user)?)))?;
        // Only a node that runs the user's calls says an app made a call (whose items fill it in), and how deep it is.
        if !serves {
            (req.from, req.depth) = (None, 0);
        }
        // A service that runs as this device's own user (Shell, say) runs only for a device of theirs, never for a
        // call a node passes on.
        let resolved = self.resolve(req.clone());
        ensure!(own || resolved.service.is_none(), "{} runs only for its user's own devices", resolved.app);
        if !own && req.service.is_none() {
            let req = self.resolve(req.clone());
            let named = req.scopes.clone();
            self.with(|i| {
                let scopes: Vec<Scope> = match &named {
                    None => reachable(i, &req.app, &me.user)?,
                    Some(named) => named.iter().map(|s| Scope::parse_for(s, &me.user)).collect::<Result<_>>()?,
                };
                let m = i.r.db();
                ensure!(scopes.iter().all(|s| m.runs_for(&remote, s)), "{} doesn't run every scope that call reaches", short(&remote));
                Ok(())
            })?;
        }
        Ok((req, me))
    }

    /// The keys a call `from` another node passed on for `user` was handed (`sealed`): sealed here for
    /// `from`'s key, of the scopes the call names alone, every one of which `user` may use and, for a node on a machine
    /// (`machine`), `from` runs. → its credential.
    pub(crate) fn take_passed(&self, from: &Key, user: &str, call: &Value, sealed: &str, machine: bool) -> Result<Credential> {
        let service = self.resolve(call_in(call)?).service.is_some();
        let named = call["scopes"].as_array().context("a call passed on names its scopes")?;
        let scopes: Vec<Scope> = named.iter().map(|s| Scope::parse_for(s.as_str().context("a scope is its name")?, user)).collect::<Result<_>>()?;
        self.with(|i| {
            let m = i.r.db();
            allowed!(scopes.iter().all(|s| m.may(user, "use", &scopes::object(s))), "that call reaches a scope its user may not use");
            ensure!(!machine || scopes.iter().all(|s| m.runs_for(from, s)), "{} doesn't run every scope that call names", short(from));
            Ok(())
        })?;
        self.hand_passed(from, user, sealed, &scopes, service)
    }

    /// Who a call another device passed on is for: only the user whose client device `remote` is (or whose client
    /// device's app runtime it is), asking itself; a node passing on keys it was handed, as itself; a user's service
    /// asks as them, as itself.
    pub(in crate) fn check_caller(&self, remote: Key, request: &Value) -> Result<Caller> {
        let key = unhex(request["me"]["key"].as_str().context("who's asking?")?)?;
        let user = request["me"]["user"].as_str().context("who's asking?")?.to_string();
        // Passed on by another node, the keys it was handed with it, which `take_passed` checks.
        let passed = request["keys"].is_string();
        self.with(|i| {
            let m = i.r.db();
            let client = m.client(&m.device_of(&remote))?.is_some_and(|c| c.user == user);
            let service = m.service(&remote)?.is_some_and(|s| s.user == user);
            let allowed = match () {
                _ if passed => remote == key && m.compute(&remote)?.is_some(),
                _ => (client || service) && remote == key,
            };
            ensure!(allowed, "{} may not call as that user", short(&remote));
            Ok(())
        })?;
        let agent = request["me"]["agent"].as_str().map(str::to_string);
        Ok(Caller { agent, ..Caller::own(key, user) })
    }
}

/// The call in a request another node sent: what's left once who asks and the keys and files it brings are taken off.
pub(super) fn call_in(request: &Value) -> Result<CallRequest> {
    let mut call = request.as_object().cloned().unwrap_or_default();
    for envelope in ["me", "keys", "blobs"] {
        call.remove(envelope);
    }
    Ok(serde_json::from_value(Value::Object(call))?)
}
