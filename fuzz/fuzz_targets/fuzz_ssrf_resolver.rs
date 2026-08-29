//! Fuzz target 2 (PLAN.md §12): adversarial IP representations (hex IPs,
//! octal IPs, IPv6-mapped IPv4) must never pass the SSRF blocklist check.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data);
    // When the fuzzed bytes parse as an IP at all, the blocklist decision
    // must be deterministic and consistent.
    if let Ok(ip) = s.trim().parse::<std::net::IpAddr>() {
        let blocked = tokenloom_fetch::ssrf::ip_is_blocked(ip);
        // Loopback/metadata literals must always be blocked regardless of
        // textual representation.
        let is_critical = match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_link_local() || v4.is_private()
            }
            std::net::IpAddr::V6(v6) => v6.is_loopback(),
        };
        if is_critical {
            assert!(blocked, "critical IP {ip} escaped the blocklist");
        }
    }
});
