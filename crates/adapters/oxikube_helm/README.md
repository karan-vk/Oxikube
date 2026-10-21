# oxikube_helm

**Layer:** `adapters`

HelmPort adapters: native release Secret/ConfigMap decoder (read-only) and the helm CLI runner for repos/charts/install/upgrade/rollback/uninstall.

## Modules

- `cli` — `HelmCli`, the `HelmPort` over the user's `helm` binary (E16-S04): binary detection with the
  `helm.path` override, the >= 3.10 gate, cluster flags from a `TargetResolver`, `-o json` parsers,
  streamed install/upgrade output, deadlines, kill on drop, error mapping and redaction. Recorded outputs
  of Helm 3.10.3 and 4.3.1 live in `tests/fixtures/helm-<version>/`.

## Tests

```
cargo test -p oxikube_helm     # parsers over recorded output, argv, errors, a fake `helm` script
OXIKUBE_TEST_CONTEXT=kind-oxikube cargo test -p oxikube_helm --features integration --test kind_helm_cli
# OXIKUBE_TEST_HELM=/path/to/helm picks another binary; skipped without a context or helm >= 3.10
```

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
