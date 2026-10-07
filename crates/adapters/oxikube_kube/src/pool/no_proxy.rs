//! `NO_PROXY` matching for the environment proxy fallback (see [`super::proxy`]).
//!
//! The value is a comma-separated list; surrounding whitespace and empty items are
//! ignored, matching is case-insensitive, and an item that cannot be understood is
//! skipped rather than failing the build. Supported forms, as in curl and Go's
//! `httpproxy`:
//!
//! * `*` bypasses the proxy for every host;
//! * a host name or domain, with or without a leading dot (`example.com`,
//!   `.example.com`, `*.example.com`), matches that domain and every subdomain;
//! * an IP address (`10.0.0.1`, `::1`, `[::1]`);
//! * a CIDR range (`10.0.0.0/8`, `fd00::/8`), which applies to IP-literal hosts only
//!   (no DNS lookup is made, as in Go);
//! * any host or IP form with a port (`example.com:8443`, `10.0.0.1:6443`,
//!   `[::1]:6443`), which then matches only that port. The cluster's port is the
//!   `server` URL's port, or the scheme default.

use std::net::IpAddr;

/// A parsed `NO_PROXY` list. The empty list (the default) matches nothing.
#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct NoProxy {
    entries: Vec<Entry>,
}

#[derive(Clone, PartialEq, Eq)]
enum Entry {
    All,
    Domain { name: String, port: Option<u16> },
    Ip { ip: IpAddr, port: Option<u16> },
    Cidr { network: IpAddr, prefix: u8 },
}

/// The host of the cluster `server` URL, as `NO_PROXY` sees it.
pub(super) enum Target {
    Name(String),
    Ip(IpAddr),
}

impl Target {
    /// The matchable host of `url`, or `None` when it has none.
    pub(super) fn from_url(url: &url::Url) -> Option<Self> {
        Some(match url.host()? {
            url::Host::Domain(d) => Self::Name(d.trim_end_matches('.').to_ascii_lowercase()),
            url::Host::Ipv4(ip) => Self::Ip(ip.into()),
            url::Host::Ipv6(ip) => Self::Ip(ip.into()),
        })
    }
}

impl NoProxy {
    /// Parses `raw`; never fails.
    pub(super) fn parse(raw: &str) -> Self {
        Self {
            entries: raw.split(',').filter_map(Entry::parse).collect(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether `host` (reached on `port`) must bypass the proxy.
    pub(super) fn matches(&self, host: &Target, port: u16) -> bool {
        self.entries.iter().any(|e| e.matches(host, port))
    }
}

impl Entry {
    fn parse(item: &str) -> Option<Self> {
        let item = item.trim().to_ascii_lowercase();
        if item.is_empty() {
            return None;
        }
        if item == "*" {
            return Some(Self::All);
        }
        if let Some((net, prefix)) = item.split_once('/') {
            let network: IpAddr = net.trim_matches(['[', ']']).parse().ok()?;
            let prefix: u8 = prefix.parse().ok()?;
            let max = if network.is_ipv4() { 32 } else { 128 };
            return (prefix <= max).then_some(Self::Cidr { network, prefix });
        }
        let (host, port) = split_port(&item)?;
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Some(Self::Ip { ip, port });
        }
        let name = host.trim_start_matches("*.").trim_start_matches('.');
        let name = name.trim_end_matches('.');
        if name.is_empty() || name.contains(['*', '[', ']', ':']) {
            return None;
        }
        Some(Self::Domain {
            name: name.to_owned(),
            port,
        })
    }

    fn matches(&self, host: &Target, port: u16) -> bool {
        match (self, host) {
            (Self::All, _) => true,
            (Self::Domain { name, port: p }, Target::Name(h)) => {
                p.is_none_or(|p| p == port)
                    && (h == name
                        || h.strip_suffix(name.as_str())
                            .is_some_and(|rest| rest.ends_with('.')))
            }
            (Self::Ip { ip, port: p }, Target::Ip(h)) => p.is_none_or(|p| p == port) && ip == h,
            (Self::Cidr { network, prefix }, Target::Ip(h)) => in_cidr(*network, *prefix, *h),
            _ => false,
        }
    }
}

/// Splits an optional trailing `:port`. A bare IPv6 literal (two or more colons, no
/// brackets) has no port. `None` means the item is malformed.
fn split_port(item: &str) -> Option<(&str, Option<u16>)> {
    if let Some(rest) = item.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        return match after {
            "" => Some((host, None)),
            _ => Some((host, Some(after.strip_prefix(':')?.parse().ok()?))),
        };
    }
    match item.matches(':').count() {
        0 | 2.. => Some((item, None)),
        _ => {
            let (host, port) = item.split_once(':')?;
            Some((host, Some(port.parse().ok()?)))
        }
    }
}

fn in_cidr(network: IpAddr, prefix: u8, host: IpAddr) -> bool {
    match (network, host) {
        (IpAddr::V4(n), IpAddr::V4(h)) => {
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            u32::from(n) & mask == u32::from(h) & mask
        }
        (IpAddr::V6(n), IpAddr::V6(h)) => {
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            u128::from(n) & mask == u128::from(h) & mask
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bypass(list: &str, server: &str) -> bool {
        let url = url::Url::parse(server).expect("server url");
        let port = url.port_or_known_default().expect("port");
        NoProxy::parse(list).matches(&Target::from_url(&url).expect("host"), port)
    }

    #[test]
    fn exact_host_and_case_insensitivity() {
        assert!(bypass("api.corp", "https://api.corp:6443"));
        assert!(bypass("API.Corp", "https://api.CORP:6443"));
        assert!(!bypass("api.corp", "https://other.corp:6443"));
        assert!(!bypass("", "https://api.corp"));
    }

    #[test]
    fn domain_suffix_with_and_without_a_leading_dot() {
        for list in ["corp.example", ".corp.example", "*.corp.example"] {
            assert!(bypass(list, "https://k8s.corp.example"), "{list}");
            assert!(bypass(list, "https://a.b.corp.example"), "{list}");
            assert!(bypass(list, "https://corp.example"), "{list}");
            assert!(!bypass(list, "https://notcorp.example"), "{list}");
            assert!(!bypass(list, "https://corp.example.evil.com"), "{list}");
        }
    }

    #[test]
    fn trailing_dot_on_the_server_host_is_ignored() {
        assert!(bypass("corp.example", "https://k8s.corp.example.:6443"));
    }

    #[test]
    fn ip_addresses_v4_and_v6() {
        assert!(bypass("10.0.0.5", "https://10.0.0.5:6443"));
        assert!(!bypass("10.0.0.5", "https://10.0.0.6:6443"));
        assert!(bypass("::1", "https://[::1]:6443"));
        assert!(bypass("[::1]", "https://[::1]:6443"));
        assert!(!bypass("10.0.0.5", "https://api.corp"));
    }

    #[test]
    fn cidr_ranges_v4_and_v6() {
        assert!(bypass("10.0.0.0/8", "https://10.200.3.4"));
        assert!(bypass("10.0.0.0/8", "https://10.0.0.0"));
        assert!(!bypass("10.0.0.0/8", "https://11.0.0.1"));
        assert!(bypass("192.168.1.128/25", "https://192.168.1.200"));
        assert!(!bypass("192.168.1.128/25", "https://192.168.1.100"));
        assert!(bypass("0.0.0.0/0", "https://8.8.8.8"));
        assert!(bypass("10.1.2.3/32", "https://10.1.2.3"));
        assert!(bypass("fd00::/8", "https://[fd12::1]:6443"));
        assert!(!bypass("fd00::/8", "https://[fe80::1]:6443"));
        assert!(!bypass("10.0.0.0/8", "https://[::1]"));
        // A CIDR never matches a DNS name: no lookup is made.
        assert!(!bypass("10.0.0.0/8", "https://api.corp"));
    }

    #[test]
    fn wildcard_matches_everything() {
        assert!(bypass("*", "https://api.corp"));
        assert!(bypass("foo, *", "https://10.0.0.1"));
    }

    #[test]
    fn ports_restrict_the_match() {
        assert!(bypass("api.corp:6443", "https://api.corp:6443"));
        assert!(!bypass("api.corp:6443", "https://api.corp:8443"));
        assert!(bypass("api.corp:443", "https://api.corp"));
        assert!(bypass(".corp:6443", "https://api.corp:6443"));
        assert!(bypass("10.0.0.1:6443", "https://10.0.0.1:6443"));
        assert!(!bypass("10.0.0.1:6443", "https://10.0.0.1:443"));
        assert!(bypass("[::1]:6443", "https://[::1]:6443"));
        assert!(!bypass("[::1]:6443", "https://[::1]:1"));
    }

    #[test]
    fn list_separators_whitespace_and_junk() {
        let list = " foo.example , ,10.0.0.0/8,, bad/99, api.corp:notaport ,a*b ";
        assert!(bypass(list, "https://foo.example"));
        assert!(bypass(list, "https://10.1.1.1"));
        assert!(!bypass(list, "https://api.corp"));
        assert!(!bypass(list, "https://other.example"));
        assert!(NoProxy::parse(" , ,").is_empty());
    }
}
