//! Requests to a URL someone else gave, which may reach only some addresses. Each host is looked up once and
//! connected to at exactly the addresses that were checked, redirects too: a name that answers a public address to the
//! check and this machine's to the connection (DNS rebinding) reaches nothing. TLS and `Host` still use the name.

use std::fmt;
use std::net::{IpAddr, SocketAddr};

use ureq::config::ConfigBuilder;
use ureq::typestate::AgentScope;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// Says why an address may not be reached, or `None` when it may.
type Refuse = Box<dyn Fn(SocketAddr) -> Option<String> + Send + Sync>;

/// An agent from `config` whose requests go only to addresses `refuse` lets by. A proxy would be the address
/// checked, not the site's, so none is used.
pub fn agent(config: ConfigBuilder<AgentScope>, refuse: impl Fn(SocketAddr) -> Option<String> + Send + Sync + 'static) -> ureq::Agent {
    ureq::Agent::with_parts(config.proxy(None).build(), DefaultConnector::default(), Checked(Box::new(refuse)))
}

/// Only the internet: not this machine's addresses, a private network's, a tailnet's (100.64.0.0/10,
/// fd7a:115c:a1e0::/48) or a link's.
pub fn public(at: SocketAddr) -> Option<String> {
    (!on_the_internet(at.ip())).then(|| format!("{} isn't on the internet", at.ip()))
}

fn on_the_internet(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast() || v4.is_multicast() || a == 0 || (a == 100 && (64..128).contains(&b)))
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => on_the_internet(v4.into()),
            None => !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || v6.is_unique_local() || v6.is_unicast_link_local()),
        },
    }
}

/// Anything but `ports` on this machine: the node's own, which would let a request act as someone else (the local
/// API, workerd) or loop back here.
pub fn not_ports(ports: Vec<u16>) -> impl Fn(SocketAddr) -> Option<String> + Send + Sync + 'static {
    move |at| {
        let here = at.ip().is_loopback() || at.ip().is_unspecified() || matches!(at.ip(), IpAddr::V6(v6) if v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback() || v4.is_unspecified()));
        (here && ports.contains(&at.port())).then(|| "that's this device's own port".to_string())
    }
}

struct Checked(Refuse);

impl fmt::Debug for Checked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Checked")
    }
}

impl Resolver for Checked {
    fn resolve(&self, uri: &ureq::http::Uri, config: &ureq::config::Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let found = DefaultResolver::default().resolve(uri, config, timeout)?;
        if let Some(why) = found.iter().find_map(|at| (self.0)(*at)) {
            let host = uri.host().unwrap_or_default();
            return Err(ureq::Error::Other(format!("{host}: {why}").into()));
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};

    /// A name for this machine is refused by the address it looks up to, as an address is, and only those refused.
    #[test]
    fn refuses_by_the_address_connected_to() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut conn in listener.incoming().flatten() {
                let _ = conn.read(&mut [0; 1024]);
                let _ = conn.write_all(b"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n");
            }
        });
        let get = |refuse: Box<dyn Fn(std::net::SocketAddr) -> Option<String> + Send + Sync>, url: &str| crate::reach::agent(ureq::Agent::config_builder(), refuse).get(url).call().map(|r| r.status().as_u16());
        for url in [format!("http://127.0.0.1:{port}/"), format!("http://localhost:{port}/")] {
            assert!(get(Box::new(crate::reach::public), &url).is_err(), "{url}");
            assert!(get(Box::new(super::not_ports(vec![port])), &url).is_err(), "{url}");
            assert_eq!(get(Box::new(super::not_ports(vec![port + 1])), &url).unwrap(), 204, "{url}");
        }
    }
}

/// The most a request someone gave sends or is answered with.
pub const LIMIT: usize = 64 << 20;

/// Makes a request, filled in already, to anywhere but this machine's `ports` (its app runtime's own): → its status,
/// headers and body, of at most `LIMIT` bytes.
pub fn send(method: &str, url: &str, headers: Vec<(String, Vec<u8>)>, body: Vec<u8>, ports: Vec<u16>) -> anyhow::Result<(u16, Vec<(String, Vec<u8>)>, Vec<u8>)> {
    // Redirects go back to the app, whose next request comes here again: a secret never follows one to another host.
    let config = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).timeout_global(Some(std::time::Duration::from_secs(30)));
    let agent = agent(config, not_ports(ports));
    // Its secrets may be in `url`, so no error says it: only where the request went.
    let uri: ureq::http::Uri = anyhow::Context::context(url.parse(), "the address isn't a web address")?;
    let host = uri.host().unwrap_or_default().to_string();
    let mut builder = ureq::http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let mut resp = match body.is_empty() {
        true => agent.run(builder.body(())?),
        false => agent.run(builder.body(body)?),
    }
    .or_else(|err| match err {
        ureq::Error::Timeout(_) => anyhow::bail!("{host} took longer than 30s"),
        err => Err(err.into()),
    })?;
    let headers = resp.headers().iter().filter(|(name, _)| !matches!(name.as_str(), "content-length" | "transfer-encoding" | "connection")).map(|(name, value)| (name.as_str().to_string(), value.as_bytes().to_vec())).collect();
    let bytes = resp.body_mut().with_config().limit(LIMIT as u64).read_to_vec()?;
    Ok((resp.status().as_u16(), headers, bytes))
}
