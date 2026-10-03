# oxikube_prometheus

**Layer:** `adapters`

PromqlPort adapter: HTTP client, provider auto-detection (kube-prometheus, Lens stack, VictoriaMetrics, Mimir, OpenShift) and per-provider query catalogues.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
