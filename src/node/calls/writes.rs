//! What a call wrote: its appends checked into steps, the blobs they keep put in the bucket first, and its work kept
//! running after it returns (`ctx.waitUntil`).

use super::*;

impl Node {
    pub(in crate::node) async fn write_appends(&self, app: &str, appends: Vec<Append>, scopes: &[Scope], me: &Caller, from: Option<&str>) -> Result<()> {
        let prepared = self.prepare_appends(app, appends, scopes, me, from).await?;
        self.write_prepared(app, prepared, scopes, me).await
    }

    /// A call's appends, checked, before any is written: a call that fails writes nothing. The blobs its steps keep go
    /// in the bucket now, so what's slow and may fail comes before it writes.
    pub(in crate::node) async fn prepare_appends(&self, app: &str, appends: Vec<Append>, scopes: &[Scope], me: &Caller, from: Option<&str>) -> Result<Prepared> {
        // What it spent on AI counts against the app that called it, if one did.
        let (costs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Cost { .. }));
        for cost in costs {
            if let Append::Cost { model, usd, input, output } = cost {
                let to = from.unwrap_or(app);
                let said = status::Said::Cost { app: to.into(), model, usd, input, output };
                self.write_insight(to, me, said);
            }
        }
        let (adds, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Add { .. }));
        let (execs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Exec { .. }));
        let (items, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Vault { .. }));
        // Each scope's statements, in the order the call ran them, and the blobs they keep.
        let mut statements: Vec<(Scope, Vec<Statement>, Vec<String>)> = Vec::new();
        for exec in execs {
            let Append::Exec { scope, sql, params, blobs, check } = exec else { continue };
            let scope = Scope::parse(&scope)?;
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            ensure!(blobs.iter().all(|b| b.len() == 64 && b.bytes().all(|c| c.is_ascii_hexdigit())), "a blob is named by its hash");
            match statements.iter_mut().find(|(s, _, _)| *s == scope) {
                Some((_, list, kept)) => {
                    list.push((sql, params, check));
                    kept.extend(blobs);
                }
                None => statements.push((scope, vec![(sql, params, check)], blobs)),
            }
        }
        let mut writes = Vec::new();
        for append in appends {
            let title = match &append {
                Append::Switch { app, .. } | Append::Retire { app, .. } | Append::Source { app, .. } => self.title(app).await,
                _ => String::new(),
            };
            writes.push(self.with(|i| check_append(i, append, scopes, me, &title))?);
        }
        let mut kept: Vec<(Scope, String)> = statements.iter().flat_map(|(s, _, kept)| kept.iter().map(|b| (s.clone(), b.clone()))).collect();
        kept.extend(writes.iter().flat_map(|(s, c)| c.blobs.iter().map(|b| (s.clone(), b.clone()))));
        let mut put = Vec::new();
        for (scope, blob) in kept {
            self.share_blob(&scope, &blob, false).await.with_context(|| format!("blob {} isn't here: put it first", &blob[..12.min(blob.len())]))?;
            put.push(format!("{scope} {blob}"));
        }
        Ok(Prepared { statements, writes, items, adds, put })
    }

    /// Writes a call's prepared appends as its caller. Apps added come last, since adding a built-in app the network
    /// doesn't have yet brings it in first.
    pub(in crate::node) async fn write_prepared(&self, app: &str, prepared: Prepared, scopes: &[Scope], me: &Caller) -> Result<()> {
        let Prepared { statements, mut writes, items, adds, put } = prepared;
        // The blobs it put in the bucket aren't put again by the steps that keep them.
        self.0.blobs.put_first.lock().unwrap().extend(put.iter().cloned());
        let wrote = async {
            let wrote = !statements.is_empty();
            for (scope, list, kept) in statements {
                let (db, bucket) = self.call_database(me, &scope, app, scopes::APP_DATABASE, None).await?;
                let mut db = db.lock().await;
                let at = db.at();
                self.write_in(&scope, &mut db, &bucket, |c| crate::database::stored::statements(c, &list)).await?;
                if db.at() > at {
                    self.ping(&scope, Some((app, scopes::APP_DATABASE, db.at())));
                }
                drop(db);
                if !kept.is_empty() {
                    writes.push((scope, content::keep(kept)));
                }
            }
            if wrote {
                self.emit(Event::Changed { apps: vec![app.to_string()] });
            }
            for item in items {
                if let Append::Vault { id, scope, item: data, same, release } = item {
                    self.call_item(app, me, scopes, &id, scope, data, same, release).await?;
                }
            }
            for append in adds {
                if let Append::Add { app, scope, added, node } = append {
                    let scope = Scope::parse_for(&scope, &me.user)?;
                    writes.extend(self.add_entries(me, &app, &scope, added, node.as_deref()).await?);
                }
            }
            for (scope, content) in writes {
                self.append_as(&scope, &me.user, content).await?;
            }
            Ok(())
        }
        .await;
        let mut first = self.0.blobs.put_first.lock().unwrap();
        for p in &put {
            first.remove(p);
        }
        wrote
    }

    /// Keeps a call's grant after it returned, while the work it left running (`ctx.waitUntil`) goes on: its requests
    /// still go out as the call's, and it hands in its appends (`POST /appends`) until it says it's done, or for
    /// `LATER_LIMIT` at most.
    pub(in crate::node) fn keep_for_later(&self, granted: Granted) {
        let done = Arc::new(tokio::sync::Notify::new());
        self.0.calls.later.lock().unwrap().insert(granted.1.clone(), done.clone());
        let node = self.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(LATER_LIMIT, done.notified()).await;
            node.0.calls.later.lock().unwrap().remove(&granted.1);
            drop(granted);
        });
    }
}

/// One of a call's statements: its SQL, its parameters, and what it fails with when its check finds nothing.
pub(super) type Statement = (String, Vec<Value>, Option<String>);

/// A call's appends, checked, with the blobs they keep put in the bucket: what `write_prepared` writes.
pub(in crate::node) struct Prepared {
    statements: Vec<(Scope, Vec<Statement>, Vec<String>)>,
    writes: Vec<(Scope, Content)>,
    items: Vec<Append>,
    adds: Vec<Append>,
    /// `<scope> <hash>` of each blob put in the bucket.
    put: Vec<String>,
}

/// Checks one of a call's appends: into a scope it named, which the caller may write. `title` is what users call
/// the app a switch, retire or source names.
pub(in crate::node) fn check_append(i: &Network, append: Append, scopes: &[Scope], me: &Caller, title: &str) -> Result<(Scope, Content)> {
    Ok(match append {
        Append::Switch { app, schedule, scope, on } => {
            let scope = Scope::parse_for(&scope, &me.user)?;
            (scope.clone(), switch_entry(i, &me.user, &scope, &app, title, &schedule, on)?)
        }
        Append::Forget { scope, blob } => {
            let scope = Scope::parse(&scope)?;
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            ensure!(blob.len() == 64 && blob.bytes().all(|b| b.is_ascii_hexdigit()), "a blob is named by its hash");
            (scope, content::forget(&blob))
        }
        Append::Retire { app, how } => (Scope::Global, crate::settings::retire(i.r.db(), &me.user, &app, &how, title)?),
        Append::Source { app: name, link } => {
            let db = i.r.db();
            let a = db.app(&name)?.filter(|a| !a.erased).with_context(|| format!("no app named {name}"))?;
            ensure!(db.may(&me.user, "deploy", &scopes::app(&name)), "Only {title}'s owners can set where its code is kept.");
            let link = scopes::source_link(&json!(link))?;
            (Scope::Global, content::source(&name, &a.owner.to_string(), &link))
        }
        Append::Service { device, name, on, scope } => {
            let scope = match scope {
                Some(s) => Some(Scope::parse_for(&s, &me.user)?),
                None if on => added_scope(i, &me.user, &name)?,
                None => None,
            };
            (Scope::Global, crate::settings::service(i.r.db(), &me.user, &device, &name, on, scope.as_ref())?)
        }
        Append::Add { .. } | Append::Cost { .. } | Append::Exec { .. } | Append::Vault { .. } => bail!("apps added, costs, statements and items are written on their own"),
    })
}
