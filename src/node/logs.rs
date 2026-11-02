//! Each scope's log in the bucket, reading and writing its entries, and what servers tell each other: pings and presence.

use super::*;

impl Node {
    /// Reads what each scope this server holds has in the bucket after what it last read, `global` first, until it
    /// gains no scope by what it read.
    pub(super) async fn read_logs(&self) -> Result<()> {
        loop {
            let scopes = self.with(|i| {
                let mut scopes: Vec<Scope> = i.r.members.scopes().into_iter().filter(|s| i.r.holds(s)).collect();
                scopes.sort_by_key(|s| *s != Scope::Global);
                Ok(scopes)
            })?;
            let mut gained = false;
            for scope in scopes {
                gained |= self.read_log(&scope).await?;
            }
            if !gained {
                break;
            }
        }
        self.with(|i| {
            i.read = true;
            Ok(())
        })
    }

    /// Reads a scope's entries in the bucket after the last this server read: whether it now holds a scope it didn't.
    pub(super) async fn read_log(&self, scope: &Scope) -> Result<bool> {
        let _write = self.0.writes.lock().await;
        self.read_log_now(scope).await
    }

    /// `read_log`, while this server holds its writes. A server that has read none of a scope's entries starts from
    /// their newest copy.
    pub(super) async fn read_log_now(&self, scope: &Scope) -> Result<bool> {
        let (bucket, key, log) = (self.bucket()?, self.log_key(scope).await?, scope.to_string());
        let prefix = crate::log::prefix(&log);
        let from = self.with(|i| i.r.store.position(&log))?;
        let (at, entries) = match from {
            0 => crate::log::open(&bucket, &key, &prefix, &log).await?,
            _ => crate::log::read(&bucket, &key, &prefix, &log, from).await?,
        };
        if at == from {
            self.hint_log(scope, &bucket, &key, at).await;
            return Ok(false);
        }
        let handled = self.with(|i| {
            let handled = i.r.receive(entries, now_ms())?;
            i.r.store.set_position(&log, at)?;
            Ok(handled)
        })?;
        let gained = handled.gained;
        self.after(handled).await?;
        self.hint_log(scope, &bucket, &key, at).await;
        Ok(gained)
    }

    /// Once this server has read a scope's entries up to step `at`, and hasn't said so since it started: a copy of them
    /// in the bucket if it's due, and their newest hint moved on (`src/log.rs`).
    pub(super) async fn hint_log(&self, scope: &Scope, bucket: &impl crate::bucket::Bucket, key: &[u8; 32], at: u64) {
        let log = scope.to_string();
        if self.0.hinted.lock().unwrap().get(&log) == Some(&at) {
            return;
        }
        let entries = || self.with(|i| i.r.store.scope(&log));
        match crate::log::hint(bucket, key, &crate::log::prefix(&log), at, entries).await {
            Ok(()) => drop(self.0.hinted.lock().unwrap().insert(log, at)),
            Err(err) => tracing::debug!("{log}'s copy: {err:#}"),
        }
    }

    /// Writes an entry this server made to its scope's entries in the bucket, at the first position free.
    pub(super) async fn put_entry(&self, scope: &Scope, entry: &Entry) -> Result<()> {
        use crate::bucket::Bucket;
        let (bucket, key, prefix) = (self.bucket()?, self.log_key(scope).await?, crate::log::prefix(&scope.to_string()));
        let bytes = crate::entry::pack(&[entry]);
        let written = self.0.written.lock().unwrap().get(scope).copied().unwrap_or(0);
        let mut at = self.with(|i| i.r.store.position(&scope.to_string()))?.max(written);
        for _ in 0..1000 {
            at += 1;
            let name = crate::db::step_name(&prefix, at);
            if Bucket::create(&bucket, &name, &crate::db::seal(&key, &name, &bytes)).await? {
                self.0.written.lock().unwrap().insert(scope.clone(), at);
                return Ok(());
            }
        }
        bail!("{scope} changed too often to write to")
    }

    /// The key a scope's entries are sealed with in the bucket: the node's `log` of the scope.
    pub(super) async fn log_key(&self, scope: &Scope) -> Result<[u8; 32]> {
        self.database_key(scope, members::APP, crate::log::NAME).await
    }

    /// Tells the servers that hold `scope` it changed, so they read it now rather than at their next look: its entries,
    /// or with `wrote`, step `at` of an app's database. In the background, and only a hint: each reads what changed from
    /// the bucket.
    pub(super) fn ping(&self, scope: &Scope, wrote: Option<(&str, &str, u64)>) {
        let Ok((servers, me)) = self.with(|i| {
            let m = &i.r.members;
            Ok((m.servers().filter(|s| s.key != self.0.id && m.holds(&s.key, scope)).map(|s| s.key).collect::<Vec<_>>(), Caller { key: self.0.id, user: i.user.clone(), agent: None, token: None }))
        }) else {
            return;
        };
        if servers.is_empty() {
            return;
        }
        let mut input = json!({ "scope": scope.to_string() });
        if let Some((app, database, at)) = wrote {
            input = json!({ "scope": scope.to_string(), "app": app, "database": database, "at": at });
        }
        self.notify(&servers, &input);
        // Over iroh too, to servers not connected to the relay.
        let req = CallRequest { app: members::APP.into(), op: "changed".into(), input, ..Default::default() };
        for server in servers {
            let (node, req, me) = (self.clone(), req.clone(), me.clone());
            tokio::spawn(async move {
                let _ = node.ask_server(server, &req, &me).await;
            });
        }
    }

    /// What another server said changed in `scope`, which this server holds: its entries, which it reads now, or with
    /// `wrote`, step `at` of a database, which it fetches into its copy now if it has the database open, and until then
    /// reads from the bucket first.
    pub(super) fn changed_there(&self, scope: &Scope, wrote: Option<(&str, &str, u64)>) {
        if !self.with(|i| Ok(i.r.holds(scope))).unwrap_or(false) {
            return;
        }
        let (node, scope) = (self.clone(), scope.clone());
        let Some((app, database, at)) = wrote.map(|(a, d, at)| (a.to_string(), d.to_string(), at)) else {
            tokio::spawn(async move {
                let _ = node.read_log(&scope).await;
            });
            return;
        };
        let id = (scope, format!("{app}/{database}"));
        self.0.behind.lock().unwrap().insert(id.clone());
        tokio::spawn(async move {
            let open = node.0.dbs.lock().unwrap().get(&id).cloned();
            let filled = match (open, node.bucket()) {
                (Some(db), Ok(bucket)) => match db.try_lock().map(|db| (db.prefix().to_string(), db.at())) {
                    Ok((prefix, from)) => bucket.fill(&prefix, from).await.is_ok_and(|n| from + n >= at),
                    Err(_) => false,
                },
                _ => false,
            };
            if filled {
                node.0.behind.lock().unwrap().remove(&id);
            }
            if app != members::APP {
                node.emit(Event::Changed { apps: vec![app] });
            }
        });
    }

    /// Tells every other server which sidecars serve its local user here and how loaded it is, now and then.
    pub(super) async fn presence_loop(self) {
        loop {
            if self.has_install() {
                self.say_hello();
            }
            tokio::time::sleep(PRESENCE_EVERY).await;
        }
    }

    /// Says hello to every other server: as now and then, and when this server's sidecars change.
    pub(super) fn say_hello(&self) {
        let Ok((servers, me, sidecars)) = self.with(|i| {
            let servers: Vec<Key> = i.r.members.servers().map(|s| s.key).filter(|k| *k != self.0.id).collect();
            Ok((servers, Caller { key: self.0.id, user: i.user.clone(), agent: None, token: None }, self.sidecars_here(i)))
        }) else {
            return;
        };
        let req = CallRequest { app: members::APP.into(), op: "hello".into(), input: json!({ "sidecars": sidecars, "load": crate::load::current() }), ..Default::default() };
        for server in servers {
            let (node, req, me) = (self.clone(), req.clone(), me.clone());
            tokio::spawn(async move {
                let _ = node.ask_server(server, &req, &me).await;
            });
        }
    }
}
