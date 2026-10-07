//! `NO_PROXY` through [`build_config`]: the environment fallback is skipped for a listed
//! host, an explicit kubeconfig `proxy-url` never is. Matching forms are unit-tested in
//! `no_proxy.rs`; the environment is passed in explicitly, never read from the process.

use kube::config::Kubeconfig;
use oxikube_domain::ids::ContextName;

use super::*;

const ENV_PROXY: &str = "http://env-proxy:8080";

/// The proxy `build_config` settles on for a cluster at `server`, with an optional
/// kubeconfig `proxy-url` and the given environment.
fn proxy_for(
    server: &str,
    proxy_url: Option<&str>,
    https_proxy: Option<&str>,
    no_proxy: Option<&str>,
) -> Option<String> {
    let proxy_line = proxy_url
        .map(|u| format!("    proxy-url: {u}\n"))
        .unwrap_or_default();
    let yaml = format!(
        "clusters:\n- name: c\n  cluster:\n    server: {server}\n    \
         insecure-skip-tls-verify: true\n{proxy_line}\
         users:\n- name: u\n  user:\n    token: t\n\
         contexts:\n- name: x\n  context: {{cluster: c, user: u}}\n"
    );
    let kubeconfig = Kubeconfig::from_yaml(&yaml).expect("kubeconfig");
    let def = ContextDefinition::from_kubeconfig(&kubeconfig, &ContextName::from("x")).expect("x");
    let env = ProxyEnv::with_https_proxy(https_proxy.map(str::to_owned))
        .with_no_proxy(no_proxy.map(str::to_owned));
    build_config(&def, &PoolConfig::default(), &env)
        .expect("config")
        .proxy_url
        .map(|u| u.to_string())
}

fn env_proxy_for(server: &str, no_proxy: Option<&str>) -> Option<String> {
    proxy_for(server, None, Some(ENV_PROXY), no_proxy)
}

#[test]
fn env_proxy_applies_when_no_proxy_is_unset_or_does_not_match() {
    let expected = Some("http://env-proxy:8080/".to_owned());
    assert_eq!(env_proxy_for("https://api.corp:6443", None), expected);
    assert_eq!(
        env_proxy_for("https://api.corp:6443", Some("other.corp,10.0.0.0/8")),
        expected
    );
}

#[test]
fn a_matching_host_gets_no_proxy_from_the_environment() {
    for (server, list) in [
        ("https://api.corp:6443", "api.corp"),
        ("https://api.corp:6443", ".corp"),
        ("https://api.corp:6443", "*.corp"),
        ("https://10.1.2.3:6443", "10.0.0.0/8"),
        ("https://10.1.2.3:6443", "10.1.2.3"),
        ("https://[fd00::1]:6443", "fd00::/8"),
        ("https://api.corp:6443", "*"),
        ("https://api.corp:6443", "api.corp:6443"),
        ("https://api.corp", "other, API.CORP"),
    ] {
        assert_eq!(env_proxy_for(server, Some(list)), None, "{server} / {list}");
    }
}

#[test]
fn a_port_in_the_list_must_match_the_servers_port() {
    let expected = Some("http://env-proxy:8080/".to_owned());
    assert_eq!(
        env_proxy_for("https://api.corp:6443", Some("api.corp:8443")),
        expected
    );
    // The default port of the scheme counts.
    assert_eq!(
        env_proxy_for("https://api.corp", Some("api.corp:443")),
        None
    );
}

#[test]
fn kubeconfig_proxy_url_is_used_even_when_the_host_is_in_no_proxy() {
    let proxy = proxy_for(
        "https://api.corp:6443",
        Some("http://from-kubeconfig:3128"),
        Some(ENV_PROXY),
        Some("api.corp,*"),
    );
    assert_eq!(proxy.as_deref(), Some("http://from-kubeconfig:3128/"));
    // ...also without any HTTPS_PROXY in the environment.
    let proxy = proxy_for(
        "https://api.corp:6443",
        Some("http://from-kubeconfig:3128"),
        None,
        Some("api.corp"),
    );
    assert_eq!(proxy.as_deref(), Some("http://from-kubeconfig:3128/"));
}

#[test]
fn a_bypassed_host_does_not_validate_the_environment_proxy() {
    // `ftp://` is unsupported, but is never used for a host that bypasses the proxy.
    let proxy = proxy_for(
        "https://api.corp:6443",
        None,
        Some("ftp://env-proxy:1"),
        Some("api.corp"),
    );
    assert_eq!(proxy, None);
}

#[test]
fn no_proxy_without_an_environment_proxy_changes_nothing() {
    assert_eq!(
        proxy_for("https://api.corp", None, None, Some("api.corp")),
        None
    );
}

#[test]
fn debug_reports_only_whether_no_proxy_is_set() {
    let env = ProxyEnv::default().with_no_proxy(Some("secret-internal.corp".into()));
    let text = format!("{env:?}");
    assert!(text.contains("no_proxy_set: true"), "{text}");
    assert!(!text.contains("secret-internal"), "{text}");
    assert!(format!("{:?}", ProxyEnv::default()).contains("no_proxy_set: false"));
}

#[test]
fn empty_no_proxy_counts_as_unset() {
    assert_eq!(
        ProxyEnv::default().with_no_proxy(Some(" , ".into())),
        ProxyEnv::default()
    );
    assert_eq!(
        ProxyEnv::default().with_no_proxy(Some(String::new())),
        ProxyEnv::default()
    );
}
