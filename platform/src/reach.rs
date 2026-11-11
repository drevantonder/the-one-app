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
