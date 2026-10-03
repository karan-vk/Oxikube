# Kubernetes desktop/TUI client feature inventory, for a Rust + GPUI Lens alternative

**Method.** I read the source of Freelens (`main`, v2.0.0-0 in progress, last push 2026-10-02) and k9s (`master`, v0.50.18 line, last push 2026-09-30). I also used `gh api` issue mining, README and doc fetches, and a GitHub search for Rust/GPUI k8s clients.

**Not verified:**
- The Lens (Mirantis) source is retired. The default branch has only a README, and the code is on `master`, last pushed 2025-02-11. Lens Desktop's proprietary features come from its pricing page, not from code.
- Reddit and Hacker News searches returned almost nothing useful. Section D is built from GitHub issue reactions.

**Corrections to the brief:**
- **Seabird** is Go + GTK4/libadwaita (`getseabird/seabird`), not Rust.
- **k9s Popeye integration was removed** in v0.31.9. The README still documents it, but the code has no Popeye references.

**Your "no web tech" constraint:**
- Freelens is Electron + React + MobX, with xterm.js, Monaco, Chart.js and a JS extension runtime.
- Each of those needs a native Rust/GPUI replacement.
- Section F covers this.

---

## A. Resource catalog

### A1. Freelens resource views

Sidebar order: Cluster, Nodes, Workloads, Config, Network, Storage, Namespaces, Events, Helm, User Management, Custom Resources.

Every list view has:
- namespace filter and search
- sortable columns
- a column show/hide chooser. It is persisted as `hiddenTableColumns`; only Pods sets default-hidden columns (IP, Node, QoS).
- a per-row context menu (⋮) with Edit and Delete
- a right-hand detail drawer

**Detail drawers** show metadata (labels, annotations, owner refs, finalizers), conditions and an events list. Resources with metrics add Prometheus charts. The default per-row actions are Edit (opens a YAML tab in the dock) and Delete.

| Group / Kind | List columns | Extra detail / actions |
|---|---|---|
| (Cluster overview) | Metrics charts (memory, CPU, pods, disk), pie charts, cluster issues list | Time-range and master/worker switcher |
| core/v1 **Node** | Name, CPU, Memory, Disk, Taints, Roles, Pods, Version, Internal IP, Conditions, Instance Type, Node Group, Capacity Type, Schedulable, Age | **Shell** (node shell), **Cordon/Uncordon**, **Drain** (`--delete-emptydir-data --ignore-daemonsets --force`). Cordon and drain are sent as kubectl commands into the terminal. Metrics charts. |
| core/v1 **Pod** | Name, Namespace, Containers (status icons), Restarts, Controlled By, Node, QoS, IP, CPU, Memory, Status, Age, Warning icon, Logs button | Container list with env, ports, mounts, probes, secrets, tolerations, affinities, volumes. Per-container and pod metrics. Menu: **Shell, Attach, Logs**. There is no Evict and no ephemeral-container debug (open request #962). |
| apps/v1 **Deployment** | Name, Namespace, Pods (ready/desired), Replicas, Age, Conditions | **Scale** dialog, **Restart** (rollout restart), owned ReplicaSets, metrics. There is no rollout history or undo (request #418). |
| apps/v1 **DaemonSet** | Name, Namespace, Pods, Node Selector, Age | Restart, metrics |
| apps/v1 **StatefulSet** | Name, Namespace, Pods, Age | Scale, Restart, metrics |
| apps/v1 **ReplicaSet** | Name, Namespace, Desired, Current, Ready, Age | Scale |
| core/v1 **ReplicationController** | Name, Namespace, Replicas, Selector, Age | Scale |
| batch/v1 **Job** | Name, Namespace, Completions, Conditions, Age | **Suspend/Resume**, metrics |
| batch/v1 **CronJob** | Name, Namespace, Schedule, Timezone, Suspend, Active, Last Schedule, Age | **Trigger** (create a Job), **Suspend/Resume** |
| Workloads Overview | Status tiles and bars for Pods, Deployments, DaemonSets, StatefulSets, ReplicaSets, Jobs, CronJobs, plus a recent-events panel | Extensions can add widgets (`kubeWorkloadsOverviewItems`) |
| core/v1 **ConfigMap** | Name, Namespace, Keys, Age | Detail shows key/values |
| core/v1 **Secret** | Name, Namespace, Labels, Keys, Type, Age | Reveal/decode values. **Create-secret dialog** (`add-dialog`). |
| core/v1 **ResourceQuota** | Name, Namespace, Age | Per-resource used/hard bars |
| core/v1 **LimitRange** | Name, Namespace, Age | |
| autoscaling **HPA** | Name, Namespace, Metrics, Min Pods, Max Pods, Replicas, Age, Status | v1 and v2 metric parsers |
| autoscaling.k8s.io **VPA** | Name, Namespace, Mode, Age, Conditions | |
| policy/v1 **PodDisruptionBudget** | Name, Namespace, Min Available, Max Unavailable, Current Healthy, Desired Healthy, Age | |
| scheduling.k8s.io **PriorityClass** | Name, Value, Global Default, Age | |
| node.k8s.io **RuntimeClass** | Name, Handler, Age | |
| coordination.k8s.io **Lease** | Name, Namespace, Holder, Age | |
| admissionregistration **MutatingWebhookConfiguration** | Name, Webhooks, Age | |
| admissionregistration **ValidatingWebhookConfiguration** | Name, Webhooks, Age | |
| admissionregistration **ValidatingAdmissionPolicy** | Name, Validations, Age | |
| admissionregistration **ValidatingAdmissionPolicyBinding** | Name, Policy, Actions, Age | |
| core/v1 **Service** | Name, Namespace, Type, Cluster IP, Ports, External IP, Selector, Status, Age | "Open in browser" link per port. Detail lists endpoint slices. |
| core/v1 **Endpoints** | Name, Namespace, Endpoints, Age | |
| discovery.k8s.io **EndpointSlice** | Name, Namespace, Address Type, Ports, Endpoints, Age | |
| networking.k8s.io **Ingress** | Name, Namespace, LoadBalancers, Rules, Age | Ingress traffic metrics |
| networking.k8s.io **IngressClass** | Name, Controller, API Group, Scope, Kind, Age | **Set as default** menu item |
| networking.k8s.io **NetworkPolicy** | Name, Namespace, Policy Types, Age | |
| **Port Forwarding** (app-level) | Name, Namespace, Kind, Pod Port, Local Port, Protocol, Address, Status | Start, stop, open in browser |
| core/v1 **PersistentVolumeClaim** | Name, Namespace, Storage class, Size, Pods, Age, Status | PVC disk metrics |
| core/v1 **PersistentVolume** | Name, Storage Class, Capacity, Claim, Age, Status | |
| storage.k8s.io **StorageClass** | Name, Provisioner, Reclaim Policy, Default, Age | |
| core/v1 **Namespace** | Name, Labels, Status, Age | Create dialog. Delete also handles hierarchical sub-namespaces. Namespace metrics. |
| core/v1 **Event** | Type, Message, Namespace, Involved Object, Source, Count, Age, Last Seen | Sortable. Per-object warning icons. |
| helm **Releases** (Helm 3 via the helm binary) | Name, Namespace, Chart, Revision, Version, App Version, Status, Updated | Detail with values, notes, resources. Dock tabs for **Upgrade**, **Rollback**, **Delete**. |
| helm **Charts** (from configured repos) | Icon, Name, Description, Version, App Version, Repository | Detail page with README and versions. **Install** opens a dock tab with a values editor. |
| rbac **ServiceAccount** | Name, Namespace, Age | Generate kubeconfig. Detail shows secrets and image pull secrets. |
| rbac **Role / ClusterRole** | Name, Namespace, Age | Rule table. Create dialogs. |
| rbac **RoleBinding / ClusterRoleBinding** | Name, Namespace, Binding targets (Role Reference, Bindings, Types), Age | Create/edit dialog for subjects and role ref |
| policy **PodSecurityPolicy** | Name, Privileged, Volumes, Age | Removed upstream in k8s 1.25, but the view is still present |
| apiextensions **CustomResourceDefinition** | Resource, Group, Version, Scope, Short Names, Age | Detail: schema, versions, printer columns |
| **Custom Resources** (sidebar group per API group) | `additionalPrinterColumns` from the CRD, plus Age | Generic YAML-style detail, edit, delete. Extensions can add columns (`additionalCategoryColumns`). |

**Gaps in Freelens:**
- Gateway API.
- Pod Evict.
- Rollout history/undo.
- Ephemeral debug containers.
- Pod file copy.
- Deployment-level logs (request #687, +11).
- Metrics from metrics-server alone, without Prometheus (#466, #627, #1670).
- VictoriaMetrics (#524).
- Custom columns (#2244).
- Grouping Custom Resources (#696).

### A2. k9s resource views

**Architecture.** k9s is dynamic. At startup it loads the server's preferred API resources, plus all CRDs, into `MetaAccess`. Any resource can be opened as a generic table with `:<name|singular|shortname|Kind>`. It has custom viewers and extenders for the kinds below, plus synthetic non-API views.

**Custom viewers (`internal/view`, `internal/render`):**

| View (alias examples) | Columns beyond NAME/NAMESPACE/AGE | Actions |
|---|---|---|
| `po` Pods | READY, STATUS, RESTARTS, LAST RESTART, CPU, MEM, %CPU/R, %CPU/L, %MEM/R, %MEM/L, GPU/RL, IP, NODE, NOMINATED NODE, READINESS GATES, QOS, SERVICE-ACCOUNT, PF, LABELS | Enter→containers, `l`/`p` logs, `s` shell, `a` attach, `t` **Transfer** (file copy to/from pod), `z` **Sanitize** (delete Completed/Failed pods), `o` show node, `ctrl-k` kill, `f`/`shift-f` port-forward, `shift-j` jump to owner |
| containers (drill-down from pod) | IDX, NAME, PF, IMAGE, READY, STATE, RESTARTS, PROBES(L:R:S), CPU/RL, %CPU/R, %CPU/L, MEM/RL, %MEM, GPU/RL, PORTS | logs, shell, attach, port-forward, `i` set image |
| `no` Nodes | ROLE, STATUS, VERSION, OS-IMAGE, KERNEL, INTERNAL-IP, EXTERNAL-IP, PODS, CPU/A, %CPU, MEM/A, %MEM, GPU/A, GPU/C, ARCH, TAINTS | `c` cordon, `u` uncordon, `r` drain (dialog), `s` node shell (needs the `nodeShell` feature gate; launches a privileged shell pod) |
| `dp` Deployments | READY, UP-TO-DATE, AVAILABLE | `s` scale, `r` restart, `i` set image, `z` ReplicaSets, `ctrl-l` rollback on RS, `shift-j` owner |
| `rs` ReplicaSets | DESIRED, CURRENT, READY | scale, **rollback** (`ctrl-l`) |
| `sts` / `ds` | READY (and UP-TO-DATE/AVAILABLE for ds), SERVICE | scale (sts), restart, set image, logs |
| `job` / `cj` | COMPLETIONS, DURATION / SCHEDULE, SUSPEND, ACTIVE, LAST_SCHEDULE | cj: `t` trigger, `s` suspend/resume (Job suspend ships as a plugin) |
| `svc` Services | TYPE, CLUSTER-IP, EXTERNAL-IP, SELECTOR, PORTS | `b` benchmark, port-forward, logs of backing pods |
| `ep`, `eps` | ENDPOINTS (+ ADDRESSTYPE) | |
| `ing`, `np` | np: POD-SELECTOR, ING/EGR SELECTOR/PORTS/BLOCK | |
| `pdb`, `hpa` | pdb: MIN-AVAILABLE, MAX-UNAVAILABLE, ALLOWED-DISRUPTIONS, EXPECTED | **Scale HPA targets** (recent) |
| `pv`, `pvc`, `sc` | pv: CAPACITY, ACCESS MODES, RECLAIM, CLAIM, STORAGECLASS, REASON, VOLUMEMODE. sc: PROVISIONER, RECLAIMPOLICY, VOLUMEBINDINGMODE, ALLOWVOLUMEEXPANSION | pvc/sa/cm/secret/priorityclass: `u` **UsedBy** |
| `cm`, `secret`, `sa` | DATA/SECRET counts | secret: `x` **decode**, UsedBy. sa: Enter→RBAC rules. |
| `ns` Namespaces | STATUS | `u` use namespace. Favorites as number keys. |
| `ev` Events | TYPE, REASON, SOURCE, COUNT, MESSAGE | `ctrl-z` faults-only toggle |
| `crd` | GROUP, VERSIONS, SCOPE, ALIASES | Enter→CR list. Custom jumps (`jumps.yaml`). |
| `cr/crb/ro/rob` | CLUSTERROLE, SUBJECT-KIND, SUBJECTS, ROLE | Enter→RBAC rules |
| `helm` / helm-history | NAMESPACE, NAME, REVISION, STATUS, CHART, APP VERSION, VALID, AGE. History: REVISION, STATUS, CHART, APP VERSION, DESCRIPTION | `r` releases/history, `r` **rollback to…**, `v` values, `v` toggle all values (uses `helm.sh/helm/v3`) |

**Synthetic and meta views:**

| View | What it is |
|---|---|
| `ctx` / contexts | Contexts from kubeconfig: CLUSTER, AUTHINFO, NAMESPACE. Enter switches. `r` rename, `ctrl-d` delete. |
| `pf` portforwards | Active port-forwards: CONTAINER, PORTS, URL, C, N, AGE. `b` benchmark, `ctrl-d` stop. |
| `be` benchmarks | `hey` result files: TIME, REQ/S, 2XX, 4XX/5XX, REPORT |
| `sd` screendumps | Saved screen dumps (`ctrl-s`) |
| `dir` | Browse a local directory of manifests: `a` apply, `d` delete, `e` edit |
| `wk` workloads | Aggregated view across Deployment, StatefulSet, DaemonSet, CronJob. Columns KIND, READY. |
| `rbac`, `usr`, `grp`, `policy` | Subject-centric RBAC: users and groups from bindings, and a per-verb policy matrix (GET/LIST/WATCH/CREATE/PATCH/UPDATE/DELETE/DEL-LIST/EXTRAS) per API group |
| `scans` | Image vulnerability view: SEVERITY, VULNERABILITY, IMAGE, LIBRARY, FIXED-IN. Built on Anchore Syft and Grype libraries. Configured via `imageScans`. |
| `pu` pulses | Dashboard: gauges and sparkline charts for Nodes, Namespaces, Services, Events, Pods, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs, PVs, PVCs, HPAs, Ingresses, NetworkPolicies, ServiceAccounts, plus Cluster CPU/Mem. Tab/hjkl to move, Enter drills in. |
| `xray <res> [ns]` | Tree/dependency view. Roots: po, svc, dp, rs, sts, ds, ns, sa. Expands owners, containers, service→pods, SA→secrets. Supports y/d/l/p/s/e/ctrl-d on nodes. |
| `ali` aliases (`ctrl-a`), `ref` references | Alias browser and reference/UsedBy results |

**Cluster header in k9s:** context, cluster, user, k9s rev, k8s rev, CPU%, MEM%.

---

## B. Cross-cutting features

### Auth and kubeconfig

**Freelens**
- Catalog entities are `KubernetesCluster`s, synced from kubeconfig files and folders. Preferences → Kubernetes → "Kubeconfig syncs" holds the watched list.
- Clusters can be added by pasting a kubeconfig (Add Cluster dialog), which is stored in app storage. Clusters can also be removed, and a cluster can be disconnected.
- The connect lifecycle is a Cluster Status page: connecting, then activating, with an error and retry state. Behind it, main runs a `kube-auth-proxy` (a bundled Go binary, `freelens-k8s-proxy`) that handles exec-plugin credential refresh, client certs and the proxy URL.
- A detector chain records distribution, id, last-seen, nodes count and version.
- Per-cluster settings:
  - Name.
  - Kubeconfig path.
  - HTTP proxy.
  - Accessible namespaces, for RBAC-restricted users.
  - Namespace auth check.
  - Prometheus provider/path/direct URL/bearer token/request method.
  - Show metrics.
  - Terminal working dir and default namespace.
  - Node shell image and pull secret.
- Global proxy and "allow untrusted CAs" are in Preferences.
- Freelens also uses the system proxy.
- It reads kubeconfig fields such as `proxy-url` (issue #1200: `proxy-url` breaks exec in the terminal).

**k9s**
- Reads `KUBECONFIG` and the default path.
- `--context`, `--kubeconfig`, `-n`, `--readonly`, `--token` and similar flags.
- `:ctx` switches in-app.
- Per-context config directories under `…/k9s/clusters/<cluster>/<context>/`.
- `@ctx` and `ctx name` commands.
- Optional start without a valid context (recent fix).

### Multi-cluster

**Freelens**
- One cluster is active per window, with each cluster in its own frame.
- A cluster **hotbar** of icons on the left, in multiple named hotbars with switch/add/rename/remove commands.
- No multi-window (#1852, +3), no tabs (#1938), and hotbar limits (#848, #527, #485).

**k9s**
- Switch with `:ctx`. `:pod @ctx1` jumps to a view in another context.
- One context at a time.

### Namespaces and search/filter

**Freelens**
- A multi-select namespace filter at the top of every namespaced list, with "All namespaces" and favorites.
- Search box per list (name, labels, annotations, plus view-specific fields).
- A catalog search command.
- Persisted search is an option (`persistentSearch`).
- No saved searches (#939) and no advanced search (#1536).

**k9s**
- `/regex` filters rows, `/!regex` inverts, `/-l selector` filters by label, and `/-f text` does a fuzzy find.
- Command-line filters: `:pod /fred`, `:pod app=x,env=y`, `:pod ns-x`.
- `0-9` keys jump to favorite namespaces.
- Namespace favorites are stored per context and can be locked.

### Logs

**Freelens** (dock tab)
- Single-pod and workload logs, with an owner/pod/container selector, including init containers.
- **Previous terminated container**, **timestamps** and **word wrap** are checkboxes.
- Search inside the logs with next/previous.
- Download visible or all logs.
- Auto-scroll to bottom.
- Open requests: log wrap persistence, JSON prettify, multi-pod aggregation (#687), selection reset on scroll (#1170).

**k9s**
- Keys:
  - `0` tail
  - `1` head
  - `2`-`6` since 1m/5m/15m/30m/1h
  - `s` autoscroll
  - `w` wrap
  - `t` timestamps
  - `f` fullscreen
  - `m` mark
  - `shift-c` clear
  - `ctrl-s` save
  - `c` copy
  - `a` all containers
  - `shift-l` column lock
  - `/` filter (regex, with inverse and fuzzy)
- Logger config: `tail`, `buffer`, `sinceSeconds`, `textWrap`, `disableAutoscroll`, `columnLock`, `showTime`, `logBufferSize`.
- Known issue: "Stream closed EOF" (#1399, +19).
- Plugins for stern, jq, bunyan, loki and lnav.

### Exec, terminal, attach

**Freelens**
- A bottom dock with tabs:
  - Terminal
  - Logs
  - Edit resource (Monaco YAML)
  - Create resource, with templates and user templates
  - Install chart
  - Upgrade chart
- Terminal is node-pty + xterm.js.
- Per-cluster terminal sessions run with kubectl env set.
- A standalone terminal (not tied to a cluster) is also available.
- Shell, font, theme, copy-on-select and scrollback preferences.
- Pod Shell and Attach, node shell.

**k9s**
- Hands the TTY to `kubectl exec`/`attach`, so the terminal is the host terminal.
- Node shell uses a shell pod.

### Port-forward

**Freelens**
- Per-port forward button on Pod and Service, a Port Forwarding list, and open-in-browser.
- Forwards run in-process in the app.

**k9s**
- `shift-f` dialog (container port → local port, address).
- `f` lists forwards.
- Annotation fast-forwards: `k9scli.io/auto-port-forwards` and `k9scli.io/port-forwards`.
- Websocket transport is on by default.
- Benchmarks with `hey`, configured per cluster/context.

### Metrics

**Freelens**
- Prometheus providers: `lens` (bundled stack), `helm`, `helm-14`, `operator`, `stacklight`, `openshift`, plus auto-detect.
- Queries exist for cluster, nodes, pods, PVCs and ingresses.
- CPU/memory columns in Pods/Nodes come from the metrics.k8s.io API.
- The "enable metrics" installer feature for the `lens` stack is part of the feature code.
- Gaps: metrics-server-only charts, VictoriaMetrics/Mimir, a switch to disable metrics fetching (#1703), init-container metrics (#1110).

**k9s**
- Metrics-server only (CPU, MEM, %CPU/R, %CPU/L, %MEM/R, %MEM/L, node %CPU/%MEM).
- Pulses, with sparkline history inside the session.
- GPU resources are supported through configurable vendors.
- Warn/critical thresholds are configurable (`thresholds.cpu/memory`).

### Helm

**Freelens**
- Releases list, detail, Upgrade (values editor), Rollback to revision, Delete.
- Charts catalog with Install.
- Repo management in Preferences (add from list or custom, remove).
- Server-side apply toggle and custom helm binary path.

**k9s**
- `:helm` list, history, `r` rollback, `v` values.
- No chart install.

### CRDs

**Freelens**
- A CRD list, plus Custom Resources grouped in the sidebar by API group.
- Lists use printer columns.
- Extension-registered extra columns and detail items per kind (`kubeObjectDetailItems`).

**k9s**
- Fully generic.
- Custom columns via `views.yaml`, including json-path expressions, attributes (`T N W S H L R`) and namespace regex scoping.
- `jumps.yaml` for CRD→related-resource navigation, using Go templates for label/field selectors and namespaces.

### Events

- **Freelens:** Events page, with Warning icons next to Pods and objects, and an events section in every detail drawer.
- **k9s:** `:ev` with `ctrl-z` faults toggle. Events are also in Pulses.

### RBAC

- **Freelens:** list and detail of Roles, ClusterRoles, Bindings and ServiceAccounts, with create dialogs. It uses `SelfSubjectRulesReview` to hide sidebar items the user can't list (`visibility-of-sidebar-items`) and to compute accessible namespaces.
- **k9s:** user/group/rbac/policy views (what a subject can do) and can-i-like matrices.

### Extensions

**Freelens**
- v2 extension API, "no compatibility promised for v1".
- Extensions are ESM, with the host's React 19, MobX 7 and Monaco injected as globals, and `main` and `renderer` entry points.
- Registrations on `LensRendererExtension`: `globalPages`, `clusterPages`, `clusterPageMenus`, `clusterFrameComponents`, `appPreferences`, `appPreferenceTabs`, `entitySettings`, `statusBarItems`, `kubeObjectDetailItems`, `kubeObjectMenuItems`, `kubeWorkloadsOverviewItems`, `commands`, `welcomeMenus`, `catalogEntityDetailItems`, `topBarItems`, `additionalCategoryColumns`, `customCategoryViews`, `kubeObjectHandlers`.
- `LensMainExtension`: `addCatalogSource`/`removeCatalogSource`, IPC, protocol handlers, `terminalShellEnvModifier`.
- Common API: catalog, cluster types, events, k8s-api, proxy, stores, app.
- Extension install from a URL or npm-like registry (`extensionRegistryUrl`), and from a local folder.
- No marketplace (#730).
- Extensions run with full privileges.

**k9s**
- Plugins are YAML only (below).

### k9s plugins, hotkeys, aliases

- **Plugins** are YAML. They are loaded from `plugins.yaml` and the `plugins/` directories, with symlinks and recursive scanning. Fields:
  - `shortCut` (`[a-z]`, `Shift-X`, `Ctrl-X`)
  - `override`
  - `confirm`
  - `description`
  - `scopes` (resource names or `all`)
  - `command`
  - `args`
  - `background`
  - `overwriteOutput`
  - `dangerous` (disabled in read-only mode)
  - `inputs`
- Plugin inputs are up to 5 of type `string`, `number`, `bool` or `dropdown`, referenced as `$INPUT_<NAME>`.
- Environment variables:
  - `$RESOURCE_GROUP`
  - `$RESOURCE_VERSION`
  - `$RESOURCE_NAME`
  - `$NAMESPACE`
  - `$NAME`
  - `$CONTAINER`
  - `$FILTER`
  - `$KUBECONFIG`
  - `$CLUSTER`
  - `$CONTEXT`
  - `$USER`
  - `$GROUPS`
  - `$POD`
  - `$COL-<COLUMN>`
- The bundled catalog has about 50 plugins: argocd, flux, cert-manager, crossplane, karpenter, keda, cnpg, helm-diff, helm-values, debug-container, dive, trace-dns, pvc-resize, log-stern/jq/lnav/loki, eks-node-viewer, remove-finalizers, blame, node-root-shell and others.
- **Hotkeys** (`hotkeys.yaml`, global and per-context): `shortCut`, `description`, `command` (can use `$RESOURCE_NAME $NAMESPACE`), `override`, `keepHistory`. Hot-reloaded.
- **Aliases** (`aliases.yaml`) can map to a GVR or to another command with args (e.g. `fred: pod fred app=blee`).

### Settings

**Freelens Preferences tabs:**
- **Application:** theme (light/dark/system), extension registry URL, start-up (open at login, tray icon), timezone, hotbar auto-hide, and menu-bar and sidebar options.
- **Proxy:** HTTP proxy and "allow untrusted CAs".
- **Kubernetes:** kubectl binary download, path, mirror and directory; helm binary path and server-side-apply toggle; kubeconfig sync list; helm repositories.
- **Editor:** minimap, line numbers, tab size, font family/size.
- **Terminal:** shell path, font family/size, terminal theme, copy-on-select.
- **Extensions:** install, enable/disable, uninstall, per-extension preference blocks.
- Other persisted settings: hidden table columns, cluster page menu order.

**k9s config:** see section C. Format is YAML. JSON schemas ship for `k9s`, `skin`, `aliases`, `hotkeys`, `plugin`, `jumps`, `views` and `context`.

### Themes

- **Freelens:** Light and Dark, plus terminal themes. A custom theme and accent colour is a heavily commented request (#1280, 51 comments; #550).
- **k9s:** about 22 skins in the repo (dracula, gruvbox variants, everforest, kanagawa, modus, in-the-navy, black-and-wtf and others). Skin schema sections: `body`, `prompt`, `info`, `help`, `dialog`, `frame`, `views` (table, xray, charts, yaml, logs), with per-section fg/bg colors. Options `invert` (dark↔light via Oklch) and per-context skin.

### Keybindings

- **Freelens:** mostly mouse-driven. Command palette (`⌘/Ctrl+Shift+P`) and a few shortcuts.
- **k9s:** keyboard-first and fully documented (section C). Custom hotkeys and plugin shortcuts are supported. Rebinding the default keys is the #2 most-requested open item (#625, +35).

### Command palette and welcome

- **Freelens:** command palette from `commands` registrations. Built-ins: navigate to cluster/hotbar/catalog, add/remove/rename/switch hotbar, preferences, extensions, and a cluster-scoped search command.
- **Freelens welcome page:** menu items from extensions (`welcomeMenus`), a "new version" notification, and catalog browse.
- **Catalog:** cluster/entity browser with categories, labels, a drawer, hotbar toggle, and `catalog-entity-drawer-menu`. Extensions add entity detail items and category columns.
- **Tray icon, application menu, status bar** (extension items), and **top bar** items.
- **Weblinks:** a catalog category of user-defined links.

### Features present in Lens Desktop, removed or absent in OpenLens/Freelens

- **OpenLens 6.3+** removed pod logs and terminal/shell (moved to the closed Lens Desktop). Freelens restored both.
- **Lens ID / login, Lens Spaces/teams, built-in "skip login" confusion** (lensapp/lens issue #5444, +923).
- **Lens K8S IDE, per its pricing page:**
  - Free "Personal" tier: multi-cluster management with metrics, real-time logs and "Smart Terminal", Helm, port-forward, resource editor.
  - Plus tier: Ask AI, GitOps (Flux/Argo CD), EKS/AKS integration, Security Center (CVE reporting) and the cluster **hotbar**.
  - Pro tier: team subscriptions and a built-in MCP server.
  - Enterprise tier: SSO/SCIM and air-gapped mode.
- Freelens has a community AI extension (`freelens-ai-extension`), but its feature set is not verified.

---

## C. k9s command and keybinding reference (condensed)

**Command mode (`:`)**
- Resource navigation:
  - `:pods|po`, `:dp`, `:svc`, any `:<singular|plural|short|Kind>`
  - `:pod ns-x`
  - `:pod /re`
  - `:pod k=v,k2=v2`
  - `:pod @ctx`
- Meta views:
  - `:ctx [name]`
  - `:ns`
  - `:pu|pulses`
  - `:xray <po|svc|dp|rs|sts|ds|ns|sa> [ns]`
  - `:rbac|usr|grp|policy`
  - `:helm`
  - `:pf`
  - `:be`
  - `:sd|screendump`
  - `:dir <path>`
  - `:scans`
  - `:wk|workloads`
  - `:alias`
  - `:help`
  - `:quit|q|q!`
- `ctrl-a` shows all aliases.
- History: `-` (last command), `[` / `]`.

**Global/table keys**
- Navigation:
  - `?` help
  - `esc` back
  - `enter` drill in
  - `q` back (in detail views)
  - `0`-`9` namespace favorites
- Filtering and views:
  - `/` filter (`/!` inverse, `/-l` label selector, `/-f` fuzzy)
  - `ctrl-w` wide
  - `ctrl-z` faults
  - `ctrl-e` header
  - `ctrl-g` crumbs
  - `ctrl-r` refresh
- Marking, copying, saving:
  - `space` mark
  - `ctrl-space` range mark
  - `ctrl-\` clear marks
  - `ctrl-s` save to file
  - `c` copy name (multi-select supported)
  - `n` copy namespace
- Sorting:
  - `shift-n/a/s/p/o` sort by Name/Age/Status/Namespace/selected column
  - `shift-←/→` move column
- Common actions:
  - `y` YAML
  - `d` describe
  - `e` edit
  - `ctrl-d` delete (TAB+ENTER to confirm)
  - `ctrl-k` kill
  - `w` warp to namespace
  - `shift-j` jump to owner

**Resource-specific keys**
- `l`/`p` logs/previous
- `s` shell, or scale on Deployments/StatefulSets/ReplicaSets
- `a` attach
- `t` transfer files (Pods) or trigger (CronJobs)
- `z` sanitize (Pods) or show ReplicaSets (Deployments)
- `o` show node
- `r` restart (Deployments/DaemonSets/StatefulSets) or drain (Nodes)
- `c`/`u` cordon/uncordon (Nodes)
- `ctrl-l` rollback (ReplicaSets)
- `i` set image
- `u` UsedBy (SA/PVC/Secret/CM/PriorityClass) or use-namespace (Namespaces)
- `x` decode secret
- `v` helm values
- `b` benchmark
- `f`/`shift-f` show/start port-forward
- CronJob `s` suspend

**Log view:** `0` tail, `1` head, `2`-`6` since 1m/5m/15m/30m/1h, `s` autoscroll, `w` wrap, `t` timestamps, `f` fullscreen, `m` mark, `shift-c` clear, `ctrl-s` save, `c` copy, `a` toggle all containers, `shift-l` column lock, `/` filter, `n`/`shift-n` next/prev match.

**YAML/describe views:** `f` fullscreen, `c` copy, `n`/`shift-n` match navigation, `ctrl-s` save, `r` toggle auto-refresh, `m` toggle managedFields, `x` toggle encoded/decoded.

**Config files (XDG, or `K9S_CONFIG_DIR`):** `config.yaml`, `aliases.yaml`, `hotkeys.yaml`, `plugins.yaml` and `plugins/`, `views.yaml`, `jumps.yaml`, `skins/*.yaml`, and `clusters/<cluster>/<context>/{config,hotkeys,benchmarks}.yaml` (namespace favorites, active namespace/view, feature gates).

**`config.yaml` keys**
- Refresh and connection:
  - `refreshRate`
  - `apiServerTimeout`
  - `maxConnRetry`
  - `readOnly`
  - `defaultView`
  - `noExitOnCtrlC`
  - `liveViewAutoRefresh`
  - `skipLatestRevCheck`
  - `disablePodCounting`
  - `portForwardAddress`
- Misc:
  - `screenDumpDir`
  - `gpuVendors`
- UI:
  - `ui.enableMouse`
  - `ui.headless`
  - `ui.logoless`
  - `ui.crumbsless`
  - `ui.splashless`
  - `ui.noIcons`
  - `ui.reactive`
  - `ui.skin`
  - `ui.invert`
  - `ui.defaultsToFullScreen`
  - `ui.useFullGVRTitle`
- `shellPod`: image, command, args, namespace, limits, labels, tty, imagePullPolicy, imagePullSecrets, hostPathVolume.
- `imageScans`: enable, namespace, exclusions.
- `logger`: tail, buffer, sinceSeconds, textWrap, disableAutoscroll, columnLock, showTime, logBufferSize.
- `thresholds`: cpu/memory warn and critical.
- Env vars: `K9S_CLIPBOARD` (auto/native/osc52), `K9S_DEFAULT_PF_ADDRESS`, `K9S_FEATURE_GATE_NODE_SHELL`, `K9S_SKIN`, `K9S_LOGS_DIR`.

---

## D. Gaps and opportunities

All counts below are GitHub reactions on open issues.

### Lens
Lensapp/lens has 1,168 open issues, the repo is frozen since Feb 2025, and OpenLens is unmaintained and ships old Electron.
- **Login/account wall:** #5444 "Unable to skip login page" (+923, by far the largest).
- **UI regression:** #8101 "The new UI is terrible" (+84).
- **Feature removals:**
  - #6857 missing pod attach/shell/log buttons (+34).
  - #1690 "Where is pod logs and terminal button".
- **Networking leak:** #6063 "Internet connection stops working when using Lens" (+55, 91 comments).
- **Reliability:**
  - #8163 logs stop after redeploy (+49).
  - #8186 ECONNRESET main-process crash (+32).
  - #7835 cannot delete pod (+30).
- **Performance:**
  - #3777 high CPU.
  - #7227 GPU helper high memory.
  - #8313 freezing.
  - #1952 CPU spike on screen sleep.
- **Most-requested features:**
  - Graphical `kubectl cp` (+98).
  - MFA prompt for aws-iam-authenticator (+89).
  - Wrap logs (+73).
  - Metrics provider extension API (+46).
  - JSON log prettify and keyword colouring (+44).
  - Remove the injected `k8slens-edit-resource-version` label (+40).
  - Multi-window (+39).
  - Per-workload logs (+38).
  - External Prometheus (+35).
  - Run rollouts on multiple resources (+34).
  - Pod resource request/usage/limit view (+32).
  - EndpointSlice UI (+32).
  - Force-delete pod (+28).
  - Copy files from node (+27).
  - WSL2 terminal (+26).
  - Rancher monitoring (+25).
  - Disable auto-update (+25).
  - VictoriaMetrics (+20).
  - GPU details (+22).
  - Terminal scrollback option (+23).

### Freelens
214 open issues; actively maintained, with v2 in progress.
- **Metrics:** metrics-server-only (#466 +40, #627 +21, #1670 +9), VictoriaMetrics (#524 +19), and disabling metrics fetch (#1703). Issue #1555 reports Nodes showing about 2x CPU/memory with VictoriaMetrics.
- **Logs:** deployment/statefulset logs (#687 +11).
- **Hotbar and UX:** more clusters on the hotbar (#848 +10) and hotbar folders (#485).
- **Theming:** custom theme and accent colour (#1280, 51 comments; #550 +10).
- **Distribution and security:**
  - Autoupdate (#552).
  - Marketplace (#730).
  - AV and EDR false positives: #1668 (SentinelOne flags code injection and browser-memory access) and #2376 (a macOS EDR classed Freelens 1.10.3 as a possible reverse shell). Both are likely false positives, but they show the trust cost of shipping Electron plus a bundled `kubectl`/`helm`/Go proxy and a full-privilege extension system. Issue #2399 asks what renderer sandboxing would cost.
- **Core gaps:**
  - Rollout support (#418).
  - Ephemeral debug containers (#962).
  - Group Custom Resources (#696).
  - Custom columns like k9s (#2244).
  - Advanced and saved searches (#1536, #939).
  - Tabs (#1938) and multi-window (#1852).
  - Context auto-switch for the terminal (#1032).
  - macOS terminal pollutes shell history with a probe command (#1696).
  - Edit view adds an annotation (#1541).
  - Clipboard paste broken in the edit search bar (#721 +10).
  - Replacing crypto-js and upgrading Chart.js (#2172, #2177).
  - Webpack to Vite (#1718).
- **Electron cost:** a large bundled runtime, `--no-sandbox` AppImage flags on Linux, gray-window bugs on some Linux setups (#940), and extensions that run with full Node privileges in the renderer.
- Both Lens and Freelens run per-cluster watchers in a JS renderer. Large clusters hurt. #113 asks for an API load calculation, and #2250 reports redundant per-namespace RBAC checks.

### k9s
91 open issues. Actively maintained, but the maintainer's own changelog notes "Sponsorship cancellations since the last release: 17", so sustainability is a risk.
- **Secrets:** edit secrets without base64 handling (#1017, +45), and decode when viewing as YAML (#4080).
- **Keybindings:** default keys can't be altered (#625, +35), and vi-like paging is missing (#4231).
- **Sorting:** sort by CPU/MEM broken in the pod view (#3793, +24). Column sorting was reworked in 0.50.18.
- **Logs:** "Stream closed EOF" (#1399, +19, 44 comments).
- **Auth errors:** an expired-auth error shows "Ruroh? 'V1/pods' command not found" instead of an auth error (#3730).
- **State clarity:**
  - Unclear whether a view is empty or still loading (#4121).
  - No hanging/working indicator (#4170).
  - API-server warning headers not shown (#4106).
- **Stability and rendering:**
  - Terminal freeze, with CTRL+C useless (#4240).
  - Flicker on resize and wrapping artifacts (#4107, #4123).
  - CPU at 100% with 50+ endpoints (#2681).
- **Missing features:**
  - Select-all (#4247).
  - Tabs for concurrent resource views (#4217).
  - Manual namespace favorites (#4019).
  - Filter strictly by name (#4199).
  - Gateway API in Pulses (#4271).
  - Per-context access differences (#4273).
  - Plugins in empty namespaces (#3992).
- **Structural limits:**
  - One context and one view at a time.
  - Tied to the host terminal: no inline charts, images or rich diff.
  - No chart install, no topology/graph UI, and Popeye was dropped.

### Opportunities for a native GPUI client

1. **Memory and idle CPU.** No Chromium, no per-cluster Node process. Use shared informer-style caches (kube-rs reflectors) with demand-driven watches.
2. **Large clusters.** Virtualised tables, with a watch budget per kind and lazy per-namespace watches. Periscope claims 10k pods at 60 fps; kubyl claims 5k+ pods. Both are young and unverified.
3. **Unify the k9s keyboard model with Lens's discoverability.** Command palette, k9s-style `:` aliases, rebindable keys (#625), saved filters and a custom-columns file, and multi-tab/multi-window (#431 +39, #1852, #1938).
4. **Treat metrics as pluggable.** Support metrics-server-only, Prometheus, VictoriaMetrics and Mimir (the most repeated Freelens and Lens complaint).
5. **Close the feature gaps the incumbents leave open:**
   - rollout history and undo
   - evict and force-delete
   - ephemeral debug containers
   - file transfer in a GUI (`kubectl cp`, +98)
   - aggregated multi-pod and workload logs, with JSON prettify (+44)
   - Gateway API
   - requests/limits/usage per pod
   - GPU info
   - Secrets editing with transparent base64
6. **Security and trust story.**
   - Extension permissions that are scoped, not full-privilege.
   - A read-only context mode (Periscope's double gate).
   - An audit log.
   - Credentials in the OS keychain.
   - No telemetry, no account.
   - No bundled opaque binaries, which answers the EDR false positives.
7. **Do not copy:** the injected edit annotations/labels (Freelens #1541, Lens #7886), login walls, and the probe command that pollutes shell history.

---

## D2. Notable features of other clients (not in Freelens or k9s)

- **Headlamp** (Kubernetes SIG UI, https://github.com/kubernetes-sigs/headlamp)
  - Runs both in-cluster (as a web app) and as a desktop app.
  - RBAC-aware UI: controls reflect the user's permissions.
  - Cancellable create/update/delete operations.
  - Plugin system with official plugins and Artifact Hub listings.
  - OpenSSF Best Practices certified.
- **Aptakube** (Tauri, https://aptakube.com/)
  - **Resource Diff** across clusters or namespaces.
  - A **human-friendly resource view** as an alternative to YAML.
  - A workload overview that surfaces failing pods, undersized containers and restart history.
  - Aggregated multi-pod/container log viewer, filterable and downloadable.
  - Metrics via metrics-server and Prometheus.
  - Follows the EKS/AKS/GKE release calendars (last five Kubernetes versions).
  - Pricing was not on the page I fetched.
- **Seabird** (Go + GTK4/libadwaita, https://github.com/getseabird/seabird)
  - A resource editor that puts the API reference next to the YAML.
  - Terminal, logs and metrics.
  - GNOME-native (Flatpak recommended). Last pushed 2025-08.
- **kubetui** (Rust + ratatui, https://github.com/sarub0b0/kubetui)
  - **Log queries** with regex, label/field selectors and resource targeting.
  - jq and JMESPath filtering of JSON logs, and a JSON pretty-print toggle.
  - Multi-pod and multi-container log retrieval, with an adjustable buffer.
  - Customisable columns through a dialog, CLI flags and presets.
  - Column-aware include/exclude regex filter plus server-side label selectors.
  - Multi-namespace selection, and a vertical/horizontal split layout.
  - Arbitrary resource watching (list/YAML) for any kind.
  - Clipboard backends (system/OSC52/auto), mouse support, incremental search.
  - Gateway API is in its network views.
- **Other Rust TUIs** (`kdash`, `b4n`, `sofka`) are in section F. `sofka`'s "why is this broken" incident view, native Flux/Argo patches and bulk-mark actions are things neither Freelens nor k9s has.

---

## F. GPUI / no-web-tech implications

### Prior art in Rust + GPUI (all very young, 0 stars, treat as references only)
- **`abdulk1/periscope`** (created 2026-08-18)
  - Generic watch-driven table for every served kind, with printer columns.
  - Warm-cluster cache and two-cluster side-by-side.
  - Merged multi-pod logs with per-pod colours.
  - Cross-cluster ⌘K palette.
  - Write confirmation and read-only contexts, with an audit log.
  - Its `LIMITATIONS.md` says plainly that GPUI has no mature accessibility layer.
  - Launch-tested only on macOS.
- **`craigeous/baeus`**
  - 45+ kinds in 11 categories, 8-crate workspace.
  - kube-rs, alacritty_terminal + portable-pty, ropey + tree-sitter editor, and a Helm release decoder.
  - A `libloading` plugin sandbox, ArgoCD built in, and a topology view.
- **`BirknerAlex/kubyl`**
  - Cmd-K palette with a k9s keymap option, and live tables (5,000+ pods).
  - YAML editor with OpenAPI schema validation plus diff/apply.
  - Pod file browser with drag-drop, and a port-forward web view.
  - Argo CD, OLM, Helm, alerts and Hubble flows; kubeconfig editor with OIDC and exec plugins; OS keychain.
- **`nklmilojevic/sofka`** (kube-rs + ratatui TUI, 1.6k stars)
  - Decodes Helm release secrets natively (no `helm` binary).
  - Native Flux/Argo suspend/sync via API patches, with bulk actions.
  - A deterministic "why is this broken" incident view, and guardrails.
- **Others:** `kdash`, `kubetui`, `b4n`. `kftray` (Tauri) is a port-forward manager with GUI and TUI. `AnuragAmbuj/kubespark` (GPUI) has not been pushed since 2025-12.

### Building blocks
| Need | Option | Status |
|---|---|---|
| UI framework | `gpui` 0.2.2 on crates.io, Apache-2.0, from Zed | "Still pre-1.0. There will often be breaking changes" (README). Zed repo is 91k stars. |
| Component library | `longbridge/gpui-kit` (includes `gpui-component`), 15.7k stars | 75+ components, virtual data tables (hundreds of thousands of rows), virtual lists, a code editor (Tree-sitter, LSP, 200K lines), dock layout with draggable tabs and nested splits (serialisable), Markdown/HTML rendering, built-in charts, AccessKit, headless UI integration tests. Claims shipped in a commercial app. |
| K8s client | `kube` 4.2 + `k8s-openapi` 0.28 (CNCF Sandbox) | Features: `ws` (exec/attach/port-forward), `oidc`, `oauth` (GCP), `socks5`, `http-proxy`, `gzip`, `jsonpatch`, `kubelet-debug`. `runtime` provides `watcher` with auto-relist and `reflector` stores. |
| Terminal | `alacritty_terminal` + `portable-pty` (used by baeus and many GPUI terminals), or Zed's `crates/terminal` | Zed's terminal crate is GPL-licensed (Zed's licensing is mixed), so check before reuse. Pure Rust, no xterm.js. |
| Editor / YAML | `gpui-component` code editor, or `ropey` + `tree-sitter` | Replaces Monaco. Needs YAML schema validation, diff and apply (kubyl shows it's feasible). |
| Charts | `gpui-component` built-in charts | Replaces Chart.js. |
| Helm | **Reading** releases: decode `sh.helm.release.v1` secrets (base64 → gzip → JSON), as sofka and baeus do. | **Install, upgrade and template** require Go template + Sprig rendering. There is no Rust Helm engine, so decide between shelling out to a pinned `helm` binary (the Freelens and Lens approach) and not offering install. |
| Extensions | Zed's `extension_api` crate uses WebAssembly, and `gpui-kit` has `gpui-shell` (a JS runtime, i.e. web tech: avoid). | Candidates: WASM components (wasmtime), or `libloading` (baeus, no sandbox). Declarative registrations map one-to-one from Freelens' list: pages, sidebar menus, kube-object detail items and menu items, status bar, commands, preferences, catalog sources, columns. |

### Constraints to plan for
- **Accessibility.** Periscope reports that GPUI has no mature accessibility layer, while `gpui-kit` ships AccessKit. This is unresolved, so verify it before committing.
- **Platform maturity.** Periscope is macOS-tested only. Linux (X11/Wayland) and Windows are CI-built but unverified, so GPUI portability must be proven early.
- **No browser engine means no embedded web views.** kubyl solves "open a Service in the app" with a port-forward plus a web view. A pure GPUI client would open the system browser instead.
- **Markdown, Helm chart READMEs and icons** need native renderers (`gpui-component` has Markdown/HTML rendering).
- **Packaging and signing** are on you. Periscope documents that signing and notarisation need Developer ID secrets in CI.
- **Bundled binaries.** Freelens ships `kubectl`, `helm` and a Go `freelens-k8s-proxy`. A pure-Rust client can replace `kubectl`/kube-auth-proxy with kube-rs (exec plugin credentials, proxy, OIDC are covered). `kubectl drain`, `kubectl cp`, `rollout undo` and `kubectl apply -f -` (resource applier) need native reimplementations.
  - Drain is cordon plus eviction with PDB handling.
  - Rollout undo patches a Deployment's pod template from a ReplicaSet revision.
  - `cp` is tar over exec.

---

## E. Sources

**Source code read (shallow clones, `/private/tmp/claude-1000895165/-Users-karan-vijayakumar-code-0misc-Oxikube/a85e422d-0922-46e9-9b7f-7728836e9348/scratchpad/research/`):**
- Freelens: https://github.com/freelensapp/freelens
  - Component tree: `packages/core/src/renderer/components/`
  - Extension API contract: `docs/extensions/api.md`, `docs/extensions/migrating-from-v1.md`
  - Renderer extension class: `packages/core/src/extensions/lens-renderer-extension.ts`
  - Prometheus providers: `packages/technical-features/prometheus/`
  - Preferences: `packages/core/src/features/preferences`, `.../user-preferences`
  - kube-object types: `packages/kube-object/src/specifics`
- k9s: https://github.com/derailed/k9s
  - Views: `internal/view`
  - Columns: `internal/render`
  - GVRs: `internal/client/gvrs.go`
  - Config schemas: `internal/config/json/schemas`
  - README, `plugins/`, `skins/`, `change_logs/`
- Lens: https://github.com/lensapp/lens (README only on the default branch; "History of this Repository" confirms the open-source version is retired)

**Web:**
- Lens plans: https://lenshq.io/pricing
- Freelens site: https://freelensapp.github.io/
- Freelens vs OpenLens vs Lens: https://alexandre-vazquez.com/openlens-vs-lens/ (thin on feature detail)
- Search results referencing the OpenLens logs/terminal removal: https://dev.to/abhinavd26/openlens-deprecated-logs-shell-k91 and https://komodor.com/learn/kubernetes-lens/
- Headlamp: https://github.com/kubernetes-sigs/headlamp
- Aptakube (Tauri): https://aptakube.com/
- Seabird (Go + GTK4): https://github.com/getseabird/seabird
- kubetui: https://github.com/sarub0b0/kubetui
- kube-rs: https://github.com/kube-rs/kube and https://kube.rs/features/
- GPUI: https://github.com/zed-industries/zed/tree/main/crates/gpui
- gpui-kit / gpui-component: https://github.com/longbridge/gpui-kit
- Prior art:
  - https://github.com/abdulk1/periscope (includes `docs/LIMITATIONS.md`)
  - https://github.com/craigeous/baeus
  - https://github.com/BirknerAlex/kubyl
  - https://github.com/nklmilojevic/sofka
  - https://github.com/kdash-rs/kdash
  - https://github.com/hcavarsan/kftray

**Issues cited** (via `gh api search/issues`, sorted by reactions):
- freelensapp/freelens: #466, #627, #524, #687, #848, #1280, #730, #1668, #2376, #2399, #1555, #1703, #418, #962, #2244, #1938, #1852
- lensapp/lens: #5444, #8101, #6063, #8163, #8186, #1369, #208, #1077, #1865, #3045, #7886, #431, #272, #909, #5017, #6857, #4154, #7857, #7835, #515, #5254, #4224, #6420, #546, #5837, #3777, #7227
- derailed/k9s: #1017, #625, #3793, #1399, #3730, #4055, #4121, #4170, #4240, #4247, #4217, #4271, #4080

**Limits of this research:**
- **Reddit/HN:** nothing usable. Section D rests on GitHub reactions, which skew toward feature requests over performance complaints.
- **Lens Desktop:** its proprietary feature set comes from the pricing page only.
- **Counts:** star counts and dates are from GitHub's API on 2026-10-03.
