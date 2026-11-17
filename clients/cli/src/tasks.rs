//! A tool called as a command, as MCP's Tasks have one (`OneCore.Tasks`): it says it takes tasks, so one its plugin
//! makes a task is answered as one, which it follows, saying each stage, asking what it asks and printing its answer.
//! Ctrl+C cancels it. `--task` prints the task's id instead, for `one task <id>` to follow.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Result, bail};
use client::Client;
use serde_json::{Map, Value, json};

/// What a request says to say its client takes tasks.
fn takes_tasks() -> Value {
    json!({"io.modelcontextprotocol/clientCapabilities": {"extensions": {"io.modelcontextprotocol/tasks": {}}}})
}

/// Calls the tool `name` with `inputs`, as a task if its plugin makes it one, and prints its answer, or with `task`,
/// its task's id. A call that isn't a task says what it's doing as its progress does.
pub async fn call(client: &Client, name: &str, inputs: Value, task: bool) -> Result<ExitCode> {
    let id = client::request_id();
    let params = json!({"name": name, "arguments": inputs, "_meta": takes_tasks()});
    let said = std::cell::RefCell::new(None::<String>);
    let progress = |line: &[u8]| {
        if let Some(doing) = client::progress_message(line) {
            say(&said, &doing);
        }
    };
    let request = client.request_notified(id, "tools/call", params, progress);
    let result = tokio::select! {
        result = request => result?,
        _ = tokio::signal::ctrl_c() => {
            let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": id}});
            client.ask(cancel.to_string().as_bytes()).await?;
            eprintln!("Cancelled.");
            return Ok(ExitCode::from(130));
        }
    };
    if result["resultType"] != "task" {
        return crate::answered(client, &result);
    }
    let id = result["taskId"].as_str().unwrap_or_default().to_owned();
    if task {
        println!("{id}");
        return Ok(ExitCode::SUCCESS);
    }
    followed(client, &id, result).await
}

/// Says how the task `id` is going, or with `result`, follows it to its answer and prints that.
pub async fn show(client: &Client, id: &str, result: bool) -> Result<ExitCode> {
    let task = get(client, id).await?;
    if result {
        return followed(client, id, task).await;
    }
    let message = task["statusMessage"].as_str().map(|message| format!(": {message}")).unwrap_or_default();
    println!("{}{message}", task["status"].as_str().unwrap_or("unknown"));
    Ok(ExitCode::SUCCESS)
}

async fn get(client: &Client, id: &str) -> Result<Value> {
    client.request("tasks/get", json!({"taskId": id, "_meta": takes_tasks()})).await
}

/// Follows the task `id`, from how it was last seen, `task`, to its end, as often as it asks to be checked on:
/// saying each stage, asking the person what it asks, and printing its answer. Ctrl+C cancels it.
async fn followed(client: &Client, id: &str, mut task: Value) -> Result<ExitCode> {
    let said = std::cell::RefCell::new(None::<String>);
    let mut asked = std::collections::HashSet::new();
    loop {
        if let Some(doing) = task["statusMessage"].as_str() {
            say(&said, doing);
        }
        match task["status"].as_str().unwrap_or_default() {
            "completed" => return crate::answered(client, &task["result"]),
            "failed" => {
                bail!("{}", task["error"]["message"].as_str().or(task["statusMessage"].as_str()).unwrap_or("It failed"))
            }
            "cancelled" => {
                eprintln!("It was cancelled.");
                return Ok(ExitCode::FAILURE);
            }
            "input_required" => {
                let mut answers = Map::new();
                for (key, request) in task["inputRequests"].as_object().into_iter().flatten() {
                    if asked.insert(key.clone()) {
                        answers.insert(key.clone(), answer(&request["params"])?);
                    }
                }
                if !answers.is_empty() {
                    client
                        .request(
                            "tasks/update",
                            json!({"taskId": id, "inputResponses": answers, "_meta": takes_tasks()}),
                        )
                        .await?;
                }
            }
            _ => {}
        }
        let every = Duration::from_millis(task["pollIntervalMs"].as_u64().unwrap_or(2_000));
        tokio::select! {
            _ = tokio::time::sleep(every) => {}
            _ = tokio::signal::ctrl_c() => {
                client.request("tasks/cancel", json!({"taskId": id, "_meta": takes_tasks()})).await?;
                eprintln!("Cancelled.");
                return Ok(ExitCode::from(130));
            }
        }
        task = get(client, id).await?;
    }
}

/// Says what a call is doing on stderr, once each time it changes.
fn say(said: &std::cell::RefCell<Option<String>>, doing: &str) {
    if said.borrow().as_deref() != Some(doing) {
        eprintln!("{doing}…");
        *said.borrow_mut() = Some(doing.to_owned());
    }
}

/// The person's answer to an elicitation, asked here: each field its schema asks for, or yes or no where it asks
/// for none.
fn answer(question: &Value) -> Result<Value> {
    eprintln!("{}", question["message"].as_str().unwrap_or("It asks:"));
    let fields = question["requestedSchema"]["properties"].as_object().cloned().unwrap_or_default();
    if fields.is_empty() {
        let yes = matches!(line("Yes or no? [y/N] ")?.to_lowercase().as_str(), "y" | "yes");
        return Ok(json!({"action": if yes { "accept" } else { "decline" }}));
    }
    let mut content = Map::new();
    for (name, field) in fields {
        let about = field["description"].as_str().or(field["title"].as_str()).map(|about| format!(" ({about})"));
        let given = line(&format!("{name}{}: ", about.unwrap_or_default()))?;
        let value = match field["type"].as_str() {
            Some("integer" | "number") => serde_json::from_str(&given).unwrap_or(json!(given)),
            Some("boolean") => json!(matches!(given.to_lowercase().as_str(), "y" | "yes" | "true")),
            _ => json!(given),
        };
        content.insert(name, value);
    }
    Ok(json!({"action": "accept", "content": content}))
}

fn line(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_owned())
}
