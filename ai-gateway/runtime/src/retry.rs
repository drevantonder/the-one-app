//! A `retryKey`'s answer, kept a day in the caller's own `node/ai-gateway`: an answer is the caller's data, sealed like
//! any database, never Insights'. Held as `sent` before the call goes, so two devices racing on one key send it once.

use anyhow::Result;
use serde_json::{Value, json};

use ai_gateway::{Failed, RETRY_KEPT_MS};

use crate::{Host, Whose};

/// `node/ai-gateway`'s schema, as its changes, oldest first.
pub const SCHEMA: &[&str] = &["
    create table retries (app text not null, key text not null, input text not null, state text not null, answer text, until integer not null, primary key (app, key));
    create table defaults (kind text primary key, model text not null);
"];

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The answer kept for this key and input, if there is one; refused if the key was used for other input, or its call
/// was sent and never answered.
pub async fn kept(host: &dyn Host, app: &str, key: &str, hash: &str) -> Result<Option<Value>> {
    let rows = host.query(Whose::Own, "select input, state, answer from retries where app = ? and key = ? and until > ?", vec![json!(app), json!(key), json!(now_ms())]).await?;
    let Some(row) = rows.first() else { return Ok(None) };
    if row["input"] != hash {
        return Err(Failed::RetryChanged.into());
    }
    match row["state"].as_str() {
        Some("done") => Ok(Some(serde_json::from_str(row["answer"].as_str().unwrap_or("null"))?)),
        _ => Err(Failed::Uncertain.into()),
    }
}

/// Holds the key as sent, unless another call took it first; and lets go of those past their day.
pub async fn sending(host: &dyn Host, app: &str, key: &str, hash: &str) -> Result<()> {
    let (app, key, hash) = (app.to_string(), key.to_string(), hash.to_string());
    host.write(
        Whose::Own,
        Box::new(move |c| {
            let now = now_ms();
            c.execute("delete from retries where until <= ?1", [now as i64])?;
            let taken: bool = c.query_row("select exists(select 1 from retries where app = ?1 and key = ?2)", [&app, &key], |r| r.get(0))?;
            if taken {
                return Err(Failed::Uncertain.into());
            }
            c.execute("insert into retries (app, key, input, state, until) values (?1, ?2, ?3, 'sent', ?4)", rusqlite::params![app, key, hash, (now + RETRY_KEPT_MS) as i64])?;
            Ok(Value::Null)
        }),
    )
    .await?;
    Ok(())
}

pub async fn done(host: &dyn Host, app: &str, key: &str, answer: &Value) -> Result<()> {
    let (app, key, answer) = (app.to_string(), key.to_string(), answer.to_string());
    host.write(
        Whose::Own,
        Box::new(move |c| {
            c.execute("update retries set state = 'done', answer = ?3 where app = ?1 and key = ?2", [&app, &key, &answer])?;
            Ok(Value::Null)
        }),
    )
    .await?;
    Ok(())
}

/// A call that failed lets go of its key, so a retry sends it again.
pub async fn forget(host: &dyn Host, app: &str, key: &str) -> Result<()> {
    let (app, key) = (app.to_string(), key.to_string());
    host.write(
        Whose::Own,
        Box::new(move |c| {
            c.execute("delete from retries where app = ?1 and key = ?2", [&app, &key])?;
            Ok(Value::Null)
        }),
    )
    .await?;
    Ok(())
}
