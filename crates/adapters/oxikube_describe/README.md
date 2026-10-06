# oxikube_describe

**Layer:** `adapters`

DescribePort adapters: deskribe (native kubectl-describe port) with a kubectl-describe CLI fallback.

## What is here (E07-S06)

- `NativeDescribe`: deskribe over the connection's kube client (kind plurals from discovery).
- `KubectlDescribe`: `kubectl describe` as a killed-on-drop child process; names and paths only on
  its command line.
- `Describer`: the `DescribePort` the app gets; picks a backend per call from the shared
  `DescribePreference` (`auto`, `native`, `kubectl`), so a settings change needs no reconnect.

Tests: `cargo test -p oxikube_describe` (a recorded Pod through a fake API server, a stub `kubectl`
script, the backend choice) and, against kind,
`OXIKUBE_TEST_CONTEXT=kind-oxikube cargo test -p oxikube_describe --features integration --test kind_describe`.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
