# oxikube_app

**Layer:** `app`

Use-cases and services (no gpui, no kube): ClusterSessionManager, ResourceStore, CommandBus, MutationGuard + AuditService, LogService, ExecService, PortForwardManager, EventService, NotificationService, MetricsService, HelmService, DescribeService, SearchService, IntegrationRegistry, ToolRegistry, ContextRegistry, AgentSessionManager, ApplyService.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Integration tests

`tests/kind_smoke/` (`--features integration`, `OXIKUBE_TEST_CONTEXT`) is the E06 smoke test: the
session manager connects two kubeconfig contexts of the kind cluster through
`oxikube_kube::KubeConnector` (a dev-dependency, exempt from `lint-deps`; the production graph has
no adapter), and a namespace selection set through `NamespaceService` re-scopes the pod feeds of
each cluster's watch budget. A second test checks that a rejected token reaches `AuthRequired`.

```
OXIKUBE_TEST_CONTEXT=kind-oxikube cargo test -p oxikube_app --features integration --test kind_smoke -- --nocapture
```

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
