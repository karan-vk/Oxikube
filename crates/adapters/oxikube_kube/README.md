# oxikube_kube

**Layer:** `adapters`

kube-rs adapter: tolerant kubeconfig loading, ClientPool per context with exec/OIDC auth, aggregated discovery, typed + dynamic APIs, reflector feeds, hand-rolled Table API feed, SSA/dry-run/patch/delete, subresources, kubectl-equivalent algorithms (trigger cronjob, rollout undo, drain), logs with reconnect, exec/attach/port-forward over ws, metrics (k8s-metrics), events.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Sources

`sources` (E03-S02, E06-S05): `KubeconfigSources` implements `ClusterSourcePort`. The source list can be replaced at
run time (`set_user_sources`, the `kubeconfig.sources` setting), each source reports how its last read went
(`source_statuses`: found with N contexts, missing, blank, invalid; file content is never in a message) and pasted
text can be checked without storing it (`validate_kubeconfig`). Tests use temp files, no cluster.

## Service proxy

`service_proxy` (E13-S05): `KubeServiceProxy` implements `ServiceProxyPort`, a GET or form POST to a Service through the API server (`/api/v1/namespaces/<ns>/services/[https:]<name>:<port>/proxy/<path>`). The API server's own refusals (403, 404, 503) come back as answers with its message, a Service's own statuses as theirs, a body over the limit as `truncated`; the connection's client retries 429/503/504, so a Service with no ready endpoint ends as a timeout that says so. Kind test: `tests/service_proxy_kind.rs`.

## Integration tests

`tests/` is the kind integration suite (`--features integration`, `OXIKUBE_TEST_CONTEXT`); its
`README.md` maps every data-plane port method and E04 acceptance item to the test that covers it.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
