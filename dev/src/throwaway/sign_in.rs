//! Signing in to a throwaway network as its first user would in a browser, with no passkey: only a throwaway network's
//! auth lets anyone in this way (`auth/worker.js`'s `throwawayNetwork`).

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

fn said(out: &Value, otherwise: &str) -> String {
    out["error_description"].as_str().or(out["error"].as_str()).or(out["message"].as_str()).unwrap_or(otherwise).to_string()
}

/// Approves a sign-in as a throwaway network's first user would in a browser, with no passkey: an app's authorization,
/// which then comes back to it. Only a throwaway network's auth signs anyone in this way
/// (`auth/worker.js`'s `throwawayNetwork`). `user`, someone who joined it, signs in as them instead.
pub fn approve_on_throwaway(address: &str, url: &str, user: Option<&str>) -> Result<()> {
    let back = approved_on_throwaway(address, url, user)?;
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).timeout_global(Some(Duration::from_secs(30))).build().into();
    agent.get(back).call()?;
    Ok(())
}

/// `approve_on_throwaway`, but for an app on another machine, where it comes back to (its loopback redirect, which
/// only that machine reaches): → that URL, for it to open there.
pub fn approved_on_throwaway(address: &str, url: &str, user: Option<&str>) -> Result<String> {
    let address = address.trim_end_matches('/');
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).timeout_global(Some(Duration::from_secs(30))).build().into();
    let went = |resp: &ureq::http::Response<ureq::Body>| resp.headers().get("location").and_then(|l| l.to_str().ok()).map(str::to_string);
    let to = |l: String| if l.starts_with('/') { format!("{address}{l}") } else { l };
    // With nobody signed in, an authorization goes on to the sign-in page with it.
    let page = went(&agent.get(url).call()?).context("the sign-in went nowhere")?;
    anyhow::ensure!(page.starts_with("/sign-in?") || page.starts_with(&format!("{address}/sign-in?")), "the sign-in went on to {page}, not its sign-in page");
    let oauth_query = page.split_once('?').map(|(_, q)| q.to_string()).context("its sign-in page had no authorization")?;
    // For the device its authorization names, as a throwaway network's sign-in page signs in for it.
    let join = oauth_query.split('&').find_map(|p| p.strip_prefix("join="));
    let who = match user {
        Some(user) => serde_json::json!({ "user": user }),
        None => serde_json::json!({ "join": join }),
    };
    let mut resp = agent.post(format!("{address}/api/auth/throwaway/sign-in")).header("origin", address).send_json(who)?;
    let cookie = resp.headers().get_all("set-cookie").iter().filter_map(|c| c.to_str().ok()?.split(';').next().map(str::to_string)).collect::<Vec<_>>().join("; ");
    if resp.status() != 200 || cookie.is_empty() {
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        bail!("only a throwaway network signs in with no passkey: {}", said(&out, "this one didn't"));
    }
    let mut resp = agent.post(format!("{address}/api/auth/oauth2/consent")).header("origin", address).header("cookie", &cookie).send_json(serde_json::json!({ "accept": true, "oauth_query": oauth_query }))?;
    let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
    let done = out["url"].as_str().with_context(|| said(&out, "signing in didn't work"))?;
    Ok(to(done.to_string()))
}
