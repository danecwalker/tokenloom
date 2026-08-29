//! Layer 1 — Transport & SSRF guard (PLAN.md §7, §11).
//!
//! Custom DNS resolution pins every hostname to verified public IPs before
//! any TCP connect; prohibited ranges (RFC1918, loopback, link-local incl.
//! `169.254.169.254`, CGNAT, multicast, reserved, IPv6 ULA/link-local) are
//! rejected. Applied to the initial request *and* every redirect hop
//! because `reqwest` re-resolves through this resolver per request.

use hickory_resolver::TokioAsyncResolver;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use tokenloom_core::TokenloomError;
use url::Url;

/// Ports in the WHATWG "bad port" blocklist (PLAN.md §6: "bad ports").
const BAD_PORTS: &[u16] = &[
    1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95, 101, 102,
    103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135, 137, 139, 143, 161, 179, 389, 427, 465,
    512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556, 563, 587, 601, 636, 989, 990, 993,
    995, 1719, 1720, 1723, 2049, 3659, 4045, 5060, 5061, 6000, 6566, 6665, 6666, 6667, 6668, 6669,
    6697, 10080,
];

/// True if the port is in the browser bad-port blocklist.
pub fn bad_port(port: u16) -> bool {
    BAD_PORTS.contains(&port)
}

/// True if the IP is in a prohibited range (PLAN.md §7 Layer 1).
///
/// Covers: `0.0.0.0/8`, loopback, RFC1918, link-local (incl. cloud metadata
/// `169.254.169.254`), CGNAT `100.64.0.0/10`, multicast/reserved, broadcast;
/// IPv6: `::1`, IPv4-mapped, ULA `fc00::/7`, link-local `fe80::/10`,
/// multicast, and other reserved ranges.
pub fn ip_is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4_is_blocked(v4),
        IpAddr::V6(v6) => v6_is_blocked(v6),
    }
}

fn v4_is_blocked(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    v4.is_loopback() // 127.0.0.0/8
        || v4.is_private() // 10/8, 172.16/12, 192.168/16
        || v4.is_link_local() // 169.254.0.0/16 (metadata endpoints)
        || v4.is_broadcast() // 255.255.255.255
        || v4.is_multicast() // 224.0.0.0/4
        || o[0] == 0 // 0.0.0.0/8 ("this network")
        || o[0] == 100 && (0x40..=0x7F).contains(&o[1]) // 100.64.0.0/10 CGNAT
        || o[0] == 192 && o[1] == 0 && o[2] == 0 // 192.0.0.0/24
        || o[0] == 192 && o[1] == 0 && o[2] == 2 // 192.0.2.0/24 TEST-NET-1
        || o[0] == 198 && (o[1] == 18 || o[1] == 19) // 198.18.0.0/15 benchmarking
        || o[0] == 198 && o[1] == 51 && o[2] == 100 // 198.51.100.0/24 TEST-NET-2
        || o[0] == 203 && o[1] == 0 && o[2] == 113 // 203.0.113.0/24 TEST-NET-3
        || o[0] >= 240 // 240.0.0.0/4 reserved (incl. broadcast)
}

fn v6_is_blocked(v6: Ipv6Addr) -> bool {
    let seg = v6.segments();
    // IPv4-mapped / IPv4-compatible (::ffff:0:0/96 and ::0.0.0.0/96)
    if (seg[0] == 0 && seg[1] == 0 && seg[2] == 0 && seg[3] == 0 && seg[4] == 0)
        && (seg[5] == 0 || seg[5] == 0xffff)
    {
        return true; // validate embedded IPv4 as blocked too (mapped forms loopback etc.)
    }
    v6.is_loopback() // ::1
        || v6.is_unspecified() // ::
        || (seg[0] & 0xfe00) == 0xfc00 // fc00::/7 ULA
        || (seg[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        || v6.is_multicast() // ff00::/8
        || (seg[0] == 0x2001 && seg[1] == 0xdb8) // documentation range
        || seg[0] == 0x100 && seg[1] == 0 && seg[2] == 0 && seg[3] == 0 // discard-only 100::/64
}

/// Validate a URL for fetching: only http/https, no bad ports, no literal
/// private IPs (PLAN.md §7 Layer 1, §11 SSRF row).
pub fn validate_url(url: &Url) -> Result<(), TokenloomError> {
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(TokenloomError::BadScheme {
                scheme: other.into(),
            })
        }
    }
    let port = url
        .port_or_known_default()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    if bad_port(port) {
        return Err(TokenloomError::BadPort { port });
    }
    if let Some(host) = url.host_str() {
        let host_l = host.to_lowercase();
        // Loopback hostnames are blocked outright (defense in depth beyond
        // the DNS-layer check — PLAN.md §7 P3).
        if host_l == "localhost" || host_l.ends_with(".localhost") {
            return Err(TokenloomError::SsrfBlocked {
                ip: IpAddr::V6(Ipv6Addr::LOCALHOST),
            });
        }
        // Literal IP addresses never hit the DNS resolver; check them here.
        if let Ok(ip) = host.parse::<IpAddr>() {
            if ip_is_blocked(ip) {
                return Err(TokenloomError::SsrfBlocked { ip });
            }
        }
    } else {
        return Err(TokenloomError::InvalidUrl(format!(
            "URL has no host: {url}"
        )));
    }
    Ok(())
}

/// A `reqwest` DNS resolver that validates every answer against the SSRF
/// blocklist. Installed via `ClientBuilder::dns_resolver`, it is invoked on
/// every request — including each redirect hop — which closes the DNS
/// rebinding window.
#[derive(Clone)]
pub struct SsrfGuardResolver {
    resolver: Arc<TokioAsyncResolver>,
}

impl SsrfGuardResolver {
    /// Build a resolver using the OS DNS configuration.
    pub fn new() -> Result<Self, TokenloomError> {
        let resolver = TokioAsyncResolver::tokio_from_system_conf()
            .map_err(|e| TokenloomError::Config(format!("failed to build DNS resolver: {e}")))?;
        Ok(Self {
            resolver: Arc::new(resolver),
        })
    }
}

impl reqwest::dns::Resolve for SsrfGuardResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let resolver = self.resolver.clone();
        Box::pin(async move {
            let host = name.as_str().to_string();
            let lookup = resolver.lookup_ip(host.clone()).await.map_err(
                |e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("DNS resolution failed for '{host}': {e}").into()
                },
            )?;

            let mut addrs: Vec<SocketAddr> = Vec::new();
            for ip in lookup.iter() {
                if ip_is_blocked(ip) {
                    return Err(Box::new(TokenloomError::SsrfBlocked { ip })
                        as Box<dyn std::error::Error + Send + Sync>);
                }
                addrs.push(SocketAddr::new(ip, 0));
            }
            if addrs.is_empty() {
                return Err(Box::new(TokenloomError::InvalidUrl(format!(
                    "no addresses resolved for '{host}'"
                )))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Hosts that must always resolve to a blocked range (used by `doctor` and
/// tests as a self-check of the guard).
pub const SSRF_SELF_TEST_HOSTS: &[&str] = &["localhost", "127.0.0.1", "169.254.169.254"];

/// Validate that a set of resolved IPs is entirely public (used by tests).
pub fn all_ips_public(ips: impl IntoIterator<Item = IpAddr>) -> bool {
    ips.into_iter().all(|ip| !ip_is_blocked(ip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn v4(s: &str) -> IpAddr {
        s.parse().unwrap()
    }
    fn v6(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn blocks_rfc1918_and_special_v4() {
        for ip in [
            "10.0.0.1",
            "10.255.255.255",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "127.0.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "169.254.169.254", // cloud metadata
            "100.64.0.1",      // CGNAT
            "224.0.0.1",
            "239.255.255.255", // multicast
            "240.0.0.1",
            "255.255.255.255", // reserved/broadcast
            "192.0.2.1",
            "198.51.100.7",
            "203.0.113.9", // TEST-NETs
            "198.18.0.1",  // benchmarking
        ] {
            assert!(ip_is_blocked(v4(ip)), "should block {ip}");
        }
    }

    #[test]
    fn allows_public_v4() {
        for ip in [
            "1.1.1.1",
            "8.8.8.8",
            "142.250.183.142",
            "93.184.216.34",
            "172.32.0.1",
        ] {
            assert!(!ip_is_blocked(v4(ip)), "should allow {ip}");
        }
    }

    #[test]
    fn blocks_v6_specials() {
        for ip in [
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1", // IPv4-mapped loopback
            "::ffff:10.0.0.1",
            "2001:db8::1",
        ] {
            assert!(ip_is_blocked(v6(ip)), "should block {ip}");
        }
    }

    #[test]
    fn allows_public_v6() {
        for ip in ["2606:4700::1111", "2001:4860:4860::8888", "2620:fe::fe"] {
            assert!(!ip_is_blocked(v6(ip)), "should allow {ip}");
        }
    }

    #[test]
    fn bad_ports_blocked() {
        for port in [22u16, 25, 53, 143, 993, 6000, 10080] {
            assert!(bad_port(port));
        }
        for port in [80u16, 443, 8080, 8443, 3000] {
            assert!(!bad_port(port));
        }
    }

    #[test]
    fn validate_url_rejects_metadata_and_scheme() {
        let blocked: Vec<Url> = [
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1:8080/",
            "http://localhost/",
            "file:///etc/passwd",
            "http://example.com:22/",
        ]
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
        for u in blocked {
            assert!(validate_url(&u).is_err(), "should reject {u}");
        }
        let ok: Url = "https://example.com/page?x=1".parse().unwrap();
        assert!(validate_url(&ok).is_ok());
    }

    #[test]
    fn all_ips_public_smoke() {
        assert!(all_ips_public([v4("1.1.1.1"), v6("2606:4700::1111")]));
        assert!(!all_ips_public([v4("1.1.1.1"), v4("10.0.0.1")]));
    }
}
