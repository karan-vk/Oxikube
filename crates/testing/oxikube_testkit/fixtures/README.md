# Fixtures

Two independent sets live here:

- **JSON manifests** (`pods/`, `workloads/`, `nodes/`, `crds/`, `events/`, `helm/`, `core/`):
  embedded by `oxikube_testkit::fixtures` and loaded as domain `Resource`s in unit tests. They
  never reach a cluster. Every file must be listed in `src/fixtures.rs` (a test checks this);
  prefer the builders for variations instead of new files. Secret data is dummy only.
- **Cluster fixtures** (`cluster/`, `metrics-server/`), described below.

## Cluster fixtures

Applied to the kind cluster (`oxikube`, context `kind-oxikube`) by `cargo xtask kind-up`
(E01-S09). The command is idempotent, only ever uses `--context kind-<name>`, and applies
things in this order:

0. every image in `test-images.txt` is pulled into each node (`crictl pull` inside the node, so it
   works for any node platform; images already present are skipped). The list covers what the
   suites and the workloads below run, plus the pinned metrics-server image.
1. `metrics-server/` (kustomization pinned to an explicit release; `kubectl apply -k`)
2. each subdirectory of `cluster/` in name order, with `kubectl apply -R -f`. A directory
   whose name ends in `crds` is followed by `kubectl wait --for condition=established`,
   so custom resources in later directories never race their definition.

| Directory | Contents |
|---|---|
| `cluster/00-crds/` | `widgets.test.oxikube.dev` (namespaced; printer columns Size, Replicas, Phase, Age, and Owner at priority 1 for `-o wide`) |
| `cluster/10-namespaces/` | `oxikube-fixtures` namespace |
| `cluster/20-workloads/` | three `Widget` CRs, Deployment `fixtures-web` (2 x pause), CronJob `fixtures-cron`, and pods in distinct states: `state-running`, `state-imagepullbackoff`, `state-pending`, Job `state-completed` |

Tests must not mutate these objects; they create their own `oxi-test-<rand>` namespace
(see `oxikube_testkit::integration::TestNamespace`).

Add new fixtures as a numbered subdirectory entry; keep images small and pinned.
