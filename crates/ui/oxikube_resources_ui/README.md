# oxikube_resources_ui

**Layer:** `ui`

Generic resource table + detail drawer, per-kind panels and actions, create/bulk ops, CRD browsing, apply UI, file browser, audit viewer.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- platform crates (`oxikube_runtime`, `oxikube_keymap`, `oxikube_theme`, ...)
- `oxikube_workspace`
- `oxikube_palette`
- `oxikube_editor`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Modules

- `kinds` (E12-S01): the kind registry's UI half. Each kind registers a `KindSpec` (the app's
  `KindMeta`: gvk, columns, actions, sidebar slot, templates; plus a `DetailRenderer`) from its
  module's `init(cx)`; `KindSpecs::kind_for` falls back to the generic spec. The built-in kinds
  (`kinds::builtin`, one module per sidebar section) fill the cluster sidebar, where each cluster
  shows the kinds its discovery serves. A kind story adds `kinds/<kind>/` and registers its own spec.
- `kinds::network` (E12-S14): Ingress, IngressClass, NetworkPolicy and the Gateway API kinds
  (GatewayClass, Gateway, HTTPRoute, GRPCRoute, TLSRoute, listed only where discovery serves them): one
  `NetworkPanel` for all of them (rules, listeners, parents, conditions with reasons, links to Services,
  Gateways and GatewayClasses) and the IngressClass row action "Set as default" (`ingressclass::SetDefault`).
- `kinds::workloads` (E12-S04): Deployments, StatefulSets, ReplicaSets and ReplicationControllers,
  one spec file each, with the shared detail `WorkloadPanel` (replicas, strategy, selector, pod
  template, owned ReplicaSets and Pods as links) and the Scale dialog (`workload::Scale`: context
  menu, palette, `s` on those kinds, the panel's button).
  Rollout history and rollback (E12-S06): `kinds::workloads::rollback`, the Rollback dialog
  (`workload::Rollback`: the revisions with change causes, what the chosen one changes, the
  confirmation tier; context menu, palette, `ctrl-l` on Deployments, StatefulSets, DaemonSets and
  ReplicaSets, the panel's button).
- `kinds::workloads::rollout` (E12-S05): Restart and Set image for Deployments, StatefulSets and
  DaemonSets (`workload::Restart`, `workload::SetImage`): context menu, palette, `r` and `i` in the
  table for those kinds, the detail header's buttons, and the set-image dialog (a field per
  container, the old-to-new diff).
- `kinds::pods` (E12-S02): the Pod spec: `PodColumns` (Lens and k9s columns, IP / Node / QoS hidden,
  usage `–` until E13's metrics) and `ContainersPanel`, the containers drill-down at the top of a pod's detail.
- `kinds::service`, `kinds::endpoints` (E12-S13): Services, Endpoints and EndpointSlices. The
  Service's detail panel lists its ports (a named `targetPort` resolved against a backing pod),
  its EndpointSlices with ready counts and its backing pods, through two server-side
  label-selected feeds that live as long as the detail is open. "Show Pods" (`u`, `service::ShowPods`)
  opens the Pods table filtered by the selector. "Open in Browser" (`o`, `service::OpenInBrowser`)
  appears once the port-forward manager installs its opener (`ServiceBrowser::enable` plus
  `register_open_in_browser`); a Service without a selector says why it has no pods.
- `kinds::storage` (E12-S15): the Storage section. Claims (Status, Storage Class, Size, Pods used by),
  volumes, storage classes (Default badge) and, where the CSI snapshot CRDs are served, volume snapshots
  and their classes; a detail panel per kind (a claim lists its bound volume and the Pods that mount it);
  "Resize…" (`pvc::Resize`, `shift-r`) and "Set as Default" (`storageclass::SetDefault`, `shift-s`) as row
  actions with their dialogs, through the guard.
- `kinds::node` (E12-S09): the Nodes spec. The table's columns (Roles, Status, Version, OS, Kernel,
  Internal IP, Pods, CPU%, Mem%, Taints, Schedulable, Age; instance type and zone as wide columns),
  the detail panel (cordon badge and button, conditions, allocation bars from the pods on the node,
  taints, pods, capacity, images) and the "Cordon" / "Uncordon" actions (keys `c` / `u` on Nodes).
- `kinds::configmap`, `kinds::secret` (E12-S11): the ConfigMaps panel (uniform key rows, a language-aware
  preview created when a key is expanded, capped at 64 KiB) and the Secrets panel (masked keys, the type's
  facts, reveal and "Edit decoded"); `detail::reveal` holds a revealed Secret in the detail's memory only
  (per tab, `REVEAL_TIMEOUT`), `x` / `shift-e` are bound to `kind == Secret`.
- `kinds::pods::panel`, `kinds::pods::actions` (E12-S03): the pod detail panel (an action bar, the containers table and the
  collapsible Owner chain, Node, Environment, Mounts, Probes, Volumes, Tolerations and Affinity sections; ConfigMap and
  Secret references resolved on demand with Secret values masked, in memory only) and the pod row actions Evict, Delete,
  Force delete, Sanitize, Jump to owner and Show node, with the keys `ctrl-k`, `z`, `o` and `shift-j` (`table::pod_keys`).
- `table` (E07-S03): `ResourceTable`, the generic virtualised table of one kind (store
  subscription, sort through the store, column layout per kind, multi-select, keyboard, context
  menu, status colours from the theme's `oxikube` block).
- `table::states` (E07-S10): `TableState` (the pure derivation), the state views, the stale badge, Retry and the API server warnings.
- `filter` (E07-S04): `FilterBar`, the `/` filter of a table (`foo`, `!foo`, `-l k=v`, `-f fuzzy`):
  parse errors, the `123 of 4,812` count, debounce, `table::FocusFilter`, and the saved filter
  (`resource_table.persist_filter`, on by default, per cluster and kind; clearing removes it), the
  removable chip of the active filter (E11-S06).
- `detail::find` (E11-S06): `/`, `n` / `N` and `escape` over the YAML and Describe text, through
  `oxikube_app::search::find`.
- `views` (E07-S03): `ResourceViews`, which opens tables in cluster tabs from the sidebar and
  `resource::OpenList`, and runs `resource::Open`, `CopyName`, `SelectAll` and `RetryFeed` (E07-S10).
- `detail` (E07-S05): `DetailView`, the generic detail of one object (header, labels and
  annotations with copy, owner links, finalizers, conditions, `status` summary, Events, and the YAML
  and Describe tabs of E07-S06), as the right-dock `DetailDrawer` of a cluster tab or,
  after `resource::PinDetail`, a workspace tab. Secrets show key names, never values.
- `detail::yaml` / `detail::describe` (E07-S06): the YAML tab (read-only tree-sitter highlighted editor from
  `oxikube_ui::editor`, managedFields toggle, secrets masked, copy and save as commands) and the Describe tab
  (the connection's `DescribePort`, spinner, error with Retry, refresh).
- `table::render` toolbar "New" button (E10-S09): dispatches `resource::Create` for the table's cluster (the manifest editor's "Create resource" picker).
- `describe_settings` (E07-S06): the `describe` setting (`backend`: `auto`, `native`, `kubectl`; `kubectl_path`).
- `views` (E07-S03): `ResourceViews`, which opens tables in cluster tabs from the sidebar and
  `resource::OpenList`, and runs `resource::Open` (which opens the detail drawer), `CopyName`,
  `SelectAll`, `PinDetail`, `CopyLabel`, `CopyYaml`, `SaveYaml`, `ToggleManagedFields` and `RefreshDescribe`.
- `crds` (E07-S07): CRD browsing. `CrdInfo` (a CRD read for browsing, the version a table opens),
  `served_versions` (the table's version switcher), `SchemaTree` (a CRD's `openAPIV3Schema` as a lazy,
  bounded, collapsible tree: the Schema tab of the CRD's detail) and the CRD list's row actions.
  A CRD row opens its custom resources (`crd::OpenResources`, Enter or the row menu); the sidebar's
  "Definitions" entry opens the CRD list (`crd::OpenList`); a kind with several served versions gets a
  switcher in its table; a table whose API ignored the Table `Accept` header says it shows basic columns.

Performance (E07-S09): cells go to the table as `TextCell`s from a per-frame `CellCache`; the
stores' feed tasks report applied events to `oxikube --perf`. Measure with
`oxikube --perf --perf-table <context>` and `cargo xtask perf scroll-10k` (docs/PERFORMANCE.md
"Resource table").

Bench: `cargo run -p oxikube_resources_ui --profile release-fast --example table_bench`
(`OXIKUBE_BENCH_FRAMES`, `OXIKUBE_BENCH_CHURN`, `OXIKUBE_BENCH_STEP` override frames, churn and
scroll step).
Screenshots: `cargo test -p oxikube_resources_ui --features screenshot --test screenshot` (status tones, the detail drawer's Overview, YAML and Describe tabs) and `--test states_screenshot` (the table states, E07-S10).

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
