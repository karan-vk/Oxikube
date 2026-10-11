# oxikube_prometheus

**Layer:** `adapters`

PromqlPort adapter: HTTP client, provider auto-detection (kube-prometheus, Lens stack, VictoriaMetrics, Mimir, OpenShift) and per-provider query catalogues.

## What is here

- `PromqlClient`: the HTTP `PromqlEndpointPort` (instant and range queries, none/bearer/basic auth, path prefix, TLS or insecure toggle, timeout, response size cap, typed `PromqlError`s).
- `PrometheusPort`: the per-cluster `PromqlPort` over it (an endpoint table filled by settings and detection); `ProxyPromqlClient` sends an endpoint on a Service through the cluster's API server service proxy.
- `ProviderDetector` (E13-S05): finds a cluster's Prometheus-compatible service from the data-driven catalogue (`src/detect/catalogue.json`), confirms it with a probe query and classifies the stack.
- Tests: recorded Prometheus, VictoriaMetrics and Mimir responses (`tests/fixtures/`), wiremock for statuses, auth and URLs, a TLS server for certificate failures, and a kind test (`--features integration`) against a Prometheus pod behind a port-forward. Detection is tested against fake Service lists and a fake service proxy (every stack's layout, forbidden, probe failures, ordering, a catalogue added as data); the real service proxy and a real detection are in `oxikube_kube/tests/service_proxy_kind.rs` and `bins/oxikube/tests/kind_metrics.rs`.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
