# oxikube_prometheus

**Layer:** `adapters`

PromqlPort adapter: HTTP client, provider auto-detection (kube-prometheus, Lens stack, VictoriaMetrics, Mimir, OpenShift) and per-provider query catalogues.

## What is here

- `PromqlClient`: the HTTP `PromqlEndpointPort` (instant and range queries, none/bearer/basic auth, path prefix, TLS or insecure toggle, timeout, response size cap, typed `PromqlError`s).
- `PrometheusPort`: the per-cluster `PromqlPort` over it (an endpoint table filled by settings and detection).
- Tests: recorded Prometheus, VictoriaMetrics and Mimir responses (`tests/fixtures/`), wiremock for statuses, auth and URLs, a TLS server for certificate failures, and a kind test (`--features integration`) against a Prometheus pod behind a port-forward.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
