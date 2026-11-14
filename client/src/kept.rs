//! What a client keeps: every answer a server gave to something that only reads, as a file in the client's home.
//! A view (an app, or the home screen) is shown what was kept at once and asked again behind it, so it opens as
//! fast with no server up as with one.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::{fs, process};

use anyhow::Result;
use futures_util::future::join_all;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::Client;

impl Client {
    /// Asks the server for `method`, which only reads, and keeps its answer.
    pub async fn fresh(&self, method: &str, params: &Value) -> Result<Value> {
        let answer = self.request(method, params.clone()).await?;
        self.keep(method, params, &answer)?;
        Ok(answer)
    }

    /// Keeps `answer` as what `method` was last answered with.
    pub(crate) fn keep(&self, method: &str, params: &Value, answer: &Value) -> Result<()> {
        let file = self.kept_file(method, params);
        fs::create_dir_all(self.home.join("kept"))?;
        // Written whole, under a name no other write has, then given its name: another ask, or another program of
        // this client, may be reading it or keeping the same answer.
        static WRITES: AtomicU64 = AtomicU64::new(0);
        let writing = file.with_extension(format!("{}-{}", process::id(), WRITES.fetch_add(1, Ordering::Relaxed)));
        fs::write(&writing, answer.to_string())?;
        fs::rename(writing, file)?;
        Ok(())
    }

    /// What a server last answered `method` with.
    pub fn kept(&self, method: &str, params: &Value) -> Option<Value> {
        serde_json::from_slice(&fs::read(self.kept_file(method, params)).ok()?).ok()
    }

    fn kept_file(&self, method: &str, params: &Value) -> PathBuf {
        self.home.join("kept").join(crate::hex(&Sha256::digest(json!([method, params]).to_string())))
    }
}

/// What one view has been answered since it opened. It asks through this, and every view gets the same, with
/// nothing to write: what only reads is answered from what was kept and asked again behind it, and asked again
/// after the view changes something, when its user returns to it, and when a server says something changed. When
/// an answer comes back different, the view is told to ask again, and is answered at once.
pub struct Shown {
    client: Arc<Client>,
    /// What only reads, by what was asked, with the answer the view was last given.
    answered: Mutex<HashMap<String, (String, Value, Value)>>,
    /// Tells the view that what it was answered has changed.
    changed: Box<dyn Fn() + Send + Sync>,
}

impl Shown {
    pub fn new(client: Arc<Client>, changed: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { client, answered: Mutex::default(), changed: Box::new(changed) })
    }

    /// Answers what the view asks: the result of `method`.
    pub async fn ask(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        match self.reads(method, &params).await {
            Some(Reads::Fixed) => self.fixed(method, &params).await,
            Some(Reads::Now) => self.read(method, params).await,
            None => {
                let answer = self.client.request(method, params).await?;
                self.again().await;
                Ok(answer)
            }
        }
    }

    /// Asks the server again for everything the view was answered, and tells the view if any of it changed.
    pub async fn again(&self) {
        let asked: Vec<String> = self.answered.lock().expect("no view panics").keys().cloned().collect();
        self.again_for(asked).await
    }

    /// Forgets what the view was answered: it closed, or loaded again.
    pub fn close(&self) {
        self.answered.lock().expect("no view panics").clear()
    }

    /// An answer that's never different, so it's asked for once and never again. A refusal isn't such an answer.
    async fn fixed(&self, method: &str, params: &Value) -> Result<Value> {
        match self.client.kept(method, params) {
            Some(answer) if answer["isError"] != true => Ok(answer),
            _ => self.client.fresh(method, params).await,
        }
    }

    async fn read(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        let asked = json!([method, params]).to_string();
        if let Some((_, _, answer)) = self.answered.lock().expect("no view panics").get(&asked) {
            return Ok(answer.clone());
        }
        let (answer, kept) = match self.client.kept(method, &params) {
            Some(answer) => (answer, true),
            None => (self.client.fresh(method, &params).await?, false),
        };
        self.answered
            .lock()
            .expect("no view panics")
            .insert(asked.clone(), (method.to_owned(), params, answer.clone()));
        if kept {
            let shown = self.clone();
            tokio::spawn(async move { shown.again_for(vec![asked]).await });
        }
        Ok(answer)
    }

    async fn again_for(&self, asked: Vec<String>) {
        let asks = asked.into_iter().filter_map(|asked| {
            let (method, params, _) = self.answered.lock().expect("no view panics").get(&asked)?.clone();
            Some(async move { (asked, self.client.fresh(&method, &params).await) })
        });
        let mut changed = false;
        for (asked, fresh) in join_all(asks).await {
            // With no server answering, what was kept stays what's shown.
            let Ok(fresh) = fresh else { continue };
            if let Some((_, _, answer)) = self.answered.lock().expect("no view panics").get_mut(&asked)
                && *answer != fresh
            {
                *answer = fresh;
                changed = true;
            }
        }
        if changed {
            (self.changed)()
        }
    }

    /// Whether asking `method` only reads, and what it reads. A tool does when the server's list of tools says so.
    async fn reads(self: &Arc<Self>, method: &str, params: &Value) -> Option<Reads> {
        match method {
            "tools/list" | "resources/list" | "resources/read" => Some(Reads::Now),
            "tools/call" => {
                let listed = self.read("tools/list", json!({})).await.ok()?;
                let tools = listed["tools"].as_array().map(Vec::as_slice).unwrap_or_default();
                let tool = tools.iter().find(|tool| tool["name"] == params["name"])?;
                match (tool["annotations"]["readOnlyHint"] == true, tool["_meta"]["fixed"] == true) {
                    (true, true) => Some(Reads::Fixed),
                    (true, false) => Some(Reads::Now),
                    (false, _) => None,
                }
            }
            _ => None,
        }
    }
}

/// What something that only reads gives: what's so now, which is asked again, or what's never different.
enum Reads {
    Now,
    Fixed,
}
