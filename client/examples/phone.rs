//! Acts as a phone from a terminal: `cargo run -p one-client --example phone -- <dir> join <pairing code>`,
//! then `... <dir> GET /v1/state` or `... <dir> POST /v1/call '{"app":"board","op":"read","input":{}}'`.

use serde_json::Value;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let client = one_client::Client::start(args[0].clone().into(), "Test phone").await?;
    let reply = match args[1].as_str() {
        "join" => client.join(&args[2]).await?,
        method => {
            let body = args.get(3).map(|b| serde_json::from_str(b)).transpose()?.unwrap_or(Value::Null);
            client.request(method, &args[2], body).await?
        }
    };
    println!("{}", serde_json::to_string_pretty(&reply)?);
    Ok(())
}
