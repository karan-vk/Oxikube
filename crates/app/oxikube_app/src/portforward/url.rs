//! The local URL of a forward: what "copy URL" puts on the clipboard and "open in browser" hands
//! to the system browser.

use std::net::{IpAddr, SocketAddr};

use oxikube_domain::portforward::ForwardPort;

use super::ForwardState;
use super::model::ForwardInfo;

/// Ports that speak TLS by convention (the same list as a Service port's browser scheme).
const TLS_PORTS: [u16; 2] = [443, 8443];

impl ForwardInfo {
    /// `http://127.0.0.1:8080`: where the forward listens while it is [`Active`](ForwardState::Active);
    /// `None` when it is starting, restarting, stopped or failed, since nothing is listening then. An unspecified bind
    /// (`0.0.0.0`, `::`) is reached through loopback; IPv6 hosts are bracketed. The scheme is
    /// `https` for a remote port named `https` / `tls` or one of the usual TLS ports, else `http`.
    pub fn url(&self) -> Option<String> {
        if !matches!(self.state, ForwardState::Active { .. }) {
            return None;
        }
        let addr = self.local_addr?;
        Some(format!("{}://{}", self.scheme(), host_port(addr)))
    }

    fn scheme(&self) -> &'static str {
        let secure = match &self.spec.remote_port {
            ForwardPort::Number(port) => TLS_PORTS.contains(port),
            ForwardPort::Named(name) => {
                name.eq_ignore_ascii_case("https") || name.eq_ignore_ascii_case("tls")
            }
        };
        if secure { "https" } else { "http" }
    }
}

/// `host:port` of a listener as a client on this machine reaches it.
fn host_port(addr: SocketAddr) -> String {
    let ip = if addr.ip().is_unspecified() {
        match addr.ip() {
            IpAddr::V4(_) => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        }
    } else {
        addr.ip()
    };
    SocketAddr::new(ip, addr.port()).to_string()
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
    use oxikube_domain::portforward::ForwardSpec;
    use oxikube_ports::ForwardStatsSnapshot;

    use super::*;
    use crate::portforward::{ForwardId, ForwardState};

    fn info(remote: ForwardPort, local_addr: Option<&str>) -> ForwardInfo {
        let target = ResourceRef::new(
            ClusterId::new("kubeconfig", &ContextName::from("ctx")),
            Gvk::new("", "v1", "Pod"),
            Some("default".into()),
            "web",
        );
        ForwardInfo {
            id: ForwardId(1),
            spec: ForwardSpec::new(target, remote),
            state: match local_addr {
                Some(a) => ForwardState::Active {
                    pod: "web-0".into(),
                    local_addr: a.parse().unwrap(),
                },
                None => ForwardState::Starting,
            },
            local_addr: local_addr.map(|a| a.parse().unwrap()),
            pod: None,
            last_error: None,
            stats: ForwardStatsSnapshot::default(),
            started_at: jiff::Timestamp::UNIX_EPOCH,
        }
    }

    #[test]
    fn ipv4_loopback_is_http_by_default() {
        let i = info(ForwardPort::Number(80), Some("127.0.0.1:18080"));
        assert_eq!(i.url().as_deref(), Some("http://127.0.0.1:18080"));
    }

    #[test]
    fn ipv6_hosts_are_bracketed() {
        let i = info(ForwardPort::Number(80), Some("[::1]:9000"));
        assert_eq!(i.url().as_deref(), Some("http://[::1]:9000"));
    }

    #[test]
    fn an_unspecified_bind_is_reached_through_loopback() {
        let v4 = info(ForwardPort::Number(80), Some("0.0.0.0:9000"));
        assert_eq!(v4.url().as_deref(), Some("http://127.0.0.1:9000"));
        let v6 = info(ForwardPort::Number(80), Some("[::]:9000"));
        assert_eq!(v6.url().as_deref(), Some("http://[::1]:9000"));
    }

    #[test]
    fn a_non_default_bind_address_is_kept() {
        let i = info(ForwardPort::Number(80), Some("192.168.1.5:9000"));
        assert_eq!(i.url().as_deref(), Some("http://192.168.1.5:9000"));
    }

    #[test]
    fn tls_ports_and_names_are_https() {
        for remote in [
            ForwardPort::Number(443),
            ForwardPort::Number(8443),
            ForwardPort::Named("https".into()),
            ForwardPort::Named("TLS".into()),
        ] {
            let i = info(remote, Some("127.0.0.1:9000"));
            assert_eq!(i.url().as_deref(), Some("https://127.0.0.1:9000"));
        }
    }

    #[test]
    fn no_url_once_the_forward_is_not_active() {
        for state in [
            ForwardState::Starting,
            ForwardState::Stopped,
            ForwardState::Failed(crate::portforward::ForwardFailure::message("boom")),
        ] {
            let mut i = info(ForwardPort::Number(80), Some("127.0.0.1:18080"));
            i.state = state;
            assert_eq!(i.url(), None);
        }
    }

    #[test]
    fn no_url_before_the_listener_is_bound() {
        assert_eq!(info(ForwardPort::Number(80), None).url(), None);
    }
}
