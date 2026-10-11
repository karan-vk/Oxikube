//! Turning a [`PromEndpoint`] into the URL of one API call.

use jiff::Timestamp;
use oxikube_ports::{PromEndpoint, PromqlError};
use url::Url;

/// The Prometheus HTTP API route to call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    /// `/api/v1/query`.
    Instant,
    /// `/api/v1/query_range`.
    Range,
}

impl Route {
    fn segment(self) -> &'static str {
        match self {
            Route::Instant => "query",
            Route::Range => "query_range",
        }
    }
}

/// `base_url` + `path_prefix` + `/api/v1/<route>`: slashes at the joins are normalised, an
/// existing query string on the base URL is kept (the call's parameters are appended to it), and
/// credentials or a fragment in the URL are refused or dropped.
pub(crate) fn api_url(endpoint: &PromEndpoint, route: Route) -> Result<Url, PromqlError> {
    let mut url = Url::parse(endpoint.base_url.trim()).map_err(|e| {
        PromqlError::InvalidEndpoint(format!("`{}` is not a valid URL: {e}", endpoint.base_url))
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(PromqlError::InvalidEndpoint(format!(
            "unsupported scheme `{}` (expected http or https)",
            url.scheme()
        )));
    }
    if url.host_str().is_none() {
        return Err(PromqlError::InvalidEndpoint("the URL has no host".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PromqlError::InvalidEndpoint(
            "credentials do not belong in the URL; configure the endpoint's auth instead".into(),
        ));
    }
    url.set_fragment(None);

    let prefix = endpoint.path_prefix.as_deref().unwrap_or("");
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|()| PromqlError::InvalidEndpoint("the URL cannot carry a path".into()))?;
        // A trailing slash on the base leaves an empty last segment; drop it before extending.
        segments.pop_if_empty();
        segments.extend(prefix.split('/').filter(|s| !s.is_empty()));
        segments.extend(["api", "v1", route.segment()]);
    }
    Ok(url)
}

/// A timestamp as Prometheus's `time`/`start`/`end` parameter: unix seconds with milliseconds.
pub(crate) fn unix_seconds(ts: Timestamp) -> String {
    let ms = ts.as_millisecond();
    // Sign and magnitude are formatted separately: floor division would turn -1.5 s into "-2.500".
    let sign = if ms < 0 { "-" } else { "" };
    let abs = ms.unsigned_abs();
    format!("{sign}{}.{:03}", abs / 1000, abs % 1000)
}

/// The URL for logs: scheme, host and path only, with the query string and credentials gone.
pub(crate) fn loggable(url: &Url) -> String {
    let mut shown = url.clone();
    shown.set_query(None);
    shown.set_fragment(None);
    let _ = shown.set_username("");
    let _ = shown.set_password(None);
    shown.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(base: &str, prefix: Option<&str>) -> Result<String, PromqlError> {
        let mut endpoint = PromEndpoint::new(base);
        endpoint.path_prefix = prefix.map(str::to_owned);
        api_url(&endpoint, Route::Instant).map(|u| u.to_string())
    }

    #[test]
    fn joins_base_prefix_and_route_with_single_slashes() {
        let want = "http://prom:9090/api/v1/query";
        assert_eq!(url("http://prom:9090", None).unwrap(), want);
        assert_eq!(url("http://prom:9090/", None).unwrap(), want);
        assert_eq!(url("http://prom:9090/", Some("")).unwrap(), want);
        assert_eq!(url("http://prom:9090", Some("/")).unwrap(), want);

        let want = "https://mimir.example/prometheus/api/v1/query";
        assert_eq!(
            url("https://mimir.example", Some("/prometheus")).unwrap(),
            want
        );
        assert_eq!(
            url("https://mimir.example/", Some("prometheus/")).unwrap(),
            want
        );
        assert_eq!(
            url("https://mimir.example//", Some("//prometheus//")).unwrap(),
            want
        );
    }

    #[test]
    fn nested_prefixes_and_base_paths_compose() {
        assert_eq!(
            url(
                "https://gw.example/grafana/",
                Some("/api/datasources/proxy/1")
            )
            .unwrap(),
            "https://gw.example/grafana/api/datasources/proxy/1/api/v1/query"
        );
        assert_eq!(
            url("https://thanos-querier.openshift-monitoring.svc:9091", None).unwrap(),
            "https://thanos-querier.openshift-monitoring.svc:9091/api/v1/query"
        );
    }

    #[test]
    fn an_existing_query_string_is_kept() {
        let mut endpoint = PromEndpoint::new("http://prom/?org=7");
        endpoint.path_prefix = Some("/p".into());
        let mut u = api_url(&endpoint, Route::Range).unwrap();
        u.query_pairs_mut().append_pair("query", "up");
        assert_eq!(
            u.as_str(),
            "http://prom/p/api/v1/query_range?org=7&query=up"
        );
    }

    #[test]
    fn bad_endpoints_are_invalid_not_panics() {
        for bad in [
            "",
            "not a url",
            "ftp://prom",
            "http://user:pw@prom",
            "mailto:a@b.c",
        ] {
            assert!(
                matches!(url(bad, None), Err(PromqlError::InvalidEndpoint(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn fragments_are_dropped_and_logs_show_no_query() {
        let mut endpoint = PromEndpoint::new("http://prom/x?a=b#frag");
        endpoint.path_prefix = None;
        let u = api_url(&endpoint, Route::Instant).unwrap();
        assert_eq!(u.fragment(), None);
        assert_eq!(loggable(&u), "http://prom/x/api/v1/query");
    }

    #[test]
    fn timestamps_render_as_unix_seconds_with_millis() {
        assert_eq!(
            unix_seconds(Timestamp::from_millisecond(1_700_000_000_123).unwrap()),
            "1700000000.123"
        );
        assert_eq!(
            unix_seconds(Timestamp::from_millisecond(5).unwrap()),
            "0.005"
        );
        assert_eq!(
            unix_seconds(Timestamp::from_millisecond(-1500).unwrap()),
            "-1.500"
        );
        assert_eq!(
            unix_seconds(Timestamp::from_millisecond(-2000).unwrap()),
            "-2.000"
        );
        assert_eq!(
            unix_seconds(Timestamp::from_millisecond(-5).unwrap()),
            "-0.005"
        );
    }
}
