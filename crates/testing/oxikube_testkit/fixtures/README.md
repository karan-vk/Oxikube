# Fixtures

Two independent sets live here:

- **JSON manifests** (`pods/`, `workloads/`, `nodes/`, `crds/`, `events/`, `helm/`, `core/`, `networking/`, `autoscaling/`, `config/`):
  embedded by `oxikube_testkit::fixtures` and loaded as domain `Resource`s in unit tests. They
  never reach a cluster. Every file must be listed in `src/fixtures.rs` (a test checks this);
  prefer the builders for variations instead of new files. Secret data is dummy only.
- **Recorded Table responses** (`tables/`): what the API server answered the Table feed for a kind of the
  kind fixtures (`tables/widgets.json`: `kubectl get widgets -n oxikube-fixtures`, v1.37), verbatim but for
  `managedFields` and annotations. Listed in `fixtures::tables` (a test checks the set and the shape).
- **Cluster fixtures** (`cluster/`, `metrics-server/`), described below.
- **Metrics stack** (`metrics-stack/`): the nightly's tier-2 metrics fixtures, described at the end.

## Cluster fixtures

Applied to the kind cluster (`oxikube`, context `kind-oxikube`) by `cargo xtask kind-up`
(E01-S09). The command is idempotent, only ever uses `--context kind-<name>`, and applies
things in this order:

0. every image in `test-images.txt` is pulled into each node (`crictl pull` inside the node, so it
   works for any node platform; images already present are skipped). The list covers what the
   suites and the workloads below run, plus the pinned metrics-server image.
1. `metrics-server/` (kustomization pinned to an explicit release; `kubectl apply -k`), then a
   wait for the `v1beta1.metrics.k8s.io` APIService to be `Available` (what `kubectl top` needs)
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

## Metrics stack (E13-S13)

`metrics-stack/charts.json` pins the Helm charts of the metrics suites' tier 2 and names a values
file next to it for each: kube-prometheus-stack (release `kube-prometheus-stack` in `monitoring`,
whose Service `kube-prometheus-stack-prometheus` is the detector catalogue's first entry; no
Alertmanager, Grafana or rules) and VictoriaMetrics single-node (`vmsingle-test` in `oxikube-vm`,
a namespace detection does not look in, served under `/vm`). `cargo xtask kind-metrics-stack`
installs them into a kind test cluster, waits for Prometheus and VictoriaMetrics to be ready and
prints the versions; `--down` removes them. The nightly `metrics-stack` job runs it; locally:

```
cargo xtask kind-up && cargo xtask kind-metrics-stack
OXIKUBE_TEST_CONTEXT=kind-oxikube OXIKUBE_TEST_PROMETHEUS_STACK=1 OXIKUBE_TEST_VICTORIA_METRICS=1 \
  cargo test -p oxikube --features integration --test kind_metrics_stack
```

It is a test fixture only: Oxikube never installs a metrics stack into a user's cluster. The tiers,
their switches and the helpers the suites share are `oxikube_testkit::integration::metrics`.
