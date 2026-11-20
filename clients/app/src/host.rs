//! One app opened as its own, on a computer: this program started with `ONE_APP` (and `ONE_SUBJECT` for a group or the
//! network) by the system, from the app's own shortcut (`shortcuts`), which asks the One App, started if it isn't
//! running, to open the app's window (`hosts::open_window`), drawn natively, and ends.

use std::process::ExitCode;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// Who this window is: its app and who it's for, and where it was opened at.
struct Opened {
    entrypoint: String,
    subject: Option<String>,
    link: String,
}

/// Asks the One App to open `entrypoint`'s window, for `subject`, at `link`, and ends once it has, or says why it
/// couldn't, which the One App's window for it says too.
pub fn run(entrypoint: String, subject: Option<String>, link: String) -> ExitCode {
    let opened = Opened { entrypoint, subject, link };
    match ::client::home().and_then(|home| hand_over(&home, &opened)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{} didn't open: {error:#}", opened.entrypoint);
            ExitCode::FAILURE
        }
    }
}

/// Asks the One App, started if it isn't running, to open `opened`'s window, and waits for its answer.
fn hand_over(home: &std::path::Path, opened: &Opened) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
    runtime.block_on(async {
        let stream = connect(&crate::hosts::address(home)).await?;
        let (read, mut write) = tokio::io::split(stream);
        let ask = json!({"id": "open", "command": "open_window", "args": {"entrypoint": opened.entrypoint, "subject": opened.subject, "link": opened.link}});
        write.write_all(format!("{ask}\n").as_bytes()).await?;
        let mut lines = BufReader::new(read).lines();
        while let Some(line) = lines.next_line().await? {
            let answer: Value = serde_json::from_str(&line).unwrap_or_default();
            if answer["id"] == "open" {
                return match answer.get("error") {
                    Some(error) => Err(anyhow::anyhow!("{error}")),
                    None => Ok(()),
                };
            }
        }
        anyhow::bail!("the One App ended")
    })
}

/// The channel to the app's window. One that isn't running is started, with its Home Screen hidden, as only this app
/// was opened, and connected to once it's listening: a program starting is the outside world, so this asks again
/// every tenth of a second for up to half a minute.
pub(crate) async fn connect(address: &str) -> anyhow::Result<impl AsyncRead + AsyncWrite + Send + 'static> {
    if let Ok(stream) = open(address).await {
        return Ok(stream);
    }
    std::process::Command::new(cli::serving::program()?)
        .env_remove("ONE_APP")
        .env_remove("ONE_SUBJECT")
        .env_remove("ONE_OPEN")
        .env("ONE_HIDDEN", "1")
        .stdin(std::process::Stdio::null())
        .spawn()?;
    for _ in 0..300 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Ok(stream) = open(address).await {
            return Ok(stream);
        }
    }
    anyhow::bail!("the One App didn't start")
}

#[cfg(unix)]
async fn open(address: &str) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(address).await
}

#[cfg(windows)]
async fn open(address: &str) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    tokio::net::windows::named_pipe::ClientOptions::new().open(address)
}
