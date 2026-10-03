# kdash and the Rust Kubernetes ecosystem

Research date: 2026-10-03. kdash cloned at `c303673` (v2.1.1, 2026-08-27); kube-rs 4.2.0 source read from raw GitHub; crate versions from the crates.io API. Read-only analysis — nothing was executed.

**Licence note:** kdash is MIT (copyright 2021 Deepu K Sasidharan). Any code copied from it must keep the copyright notice and permission text (file header or `THIRD_PARTY_NOTICES.md` entry).

---

## PART 1/3: kdash architecture, licence, fetch methods, reusable-code map

### 1.1 Licence and dependencies
- kdash is MIT, copyright 2021 Deepu K Sasidharan. Reuse requires keeping the copyright notice and permission text with copied code (THIRD_PARTY_NOTICES entry or file header).
- kdash dependency kubectl-view-allocations 3.1.0 is CC0-1.0 (no attribution needed).

Key Cargo.toml deps:
- kube 4.2.0, default-features=false, features socks5, http-proxy, client, rustls-tls, oidc, oauth, ws
- k8s-openapi 0.28.0, feature "earliest"
- tokio 1.50 (macros, process, rt-multi-thread, io-util); ratatui 0.30; crossterm 0.29
- serde-saphyr 1.0.1 (YAML); chrono 0.4 (while k8s-openapi 0.28 and kube 4 use jiff)
- kubectl-view-allocations 3.0.2; notify 8 (kubeconfig file watcher); syntect 5.3 (YAML highlight)
- async-trait, anyhow, futures, strum, clap, regex

~29.5k lines of Rust under src/.

### 1.2 Module layout (src/)
- main.rs (888 lines): three tokio mpsc channels (IoEvent, IoStreamEvent, IoCmdEvent, each size 500). Network side runs on a separate OS thread with its own runtime (main.rs:163-197). start_ui runs the render loop; process_event handles tick/input.
- app/mod.rs (2665 lines): App and Data. Data holds one StatefulTable<KubeX> per resource kind plus kubeconfig, contexts, node_metrics, logs, describe_out, dynamic_kinds. Selected holds ns/pod/container/context/dynamic_kind. State is Arc<tokio::Mutex<App>>; the network side locks it to write results. on_tick (mod.rs:1587) POLLS every tick_until_poll ticks, dispatching an IoEvent for the active route only. There are NO watchers or reflectors anywhere.
- app/<kind>.rs: one file per kind. Each defines KubeX (display model) and XResource implementing AppResource (render + async get_resource(&Network)). The k8s object is kept inside KubeX for YAML output via the KubeResource<T> trait (models.rs:44).
- network/mod.rs (1606 lines): IoEvent enum, Network struct (client: Client, app: &Arc<Mutex<App>>), kubeconfig and client construction.
- network/stream.rs (1111 lines): log streaming (IoStreamEvent) and port-forward process management.
- cmd/: shells out to the kubectl binary for describe, edit, exec shell, port-forward. Also probes versions of kubectl, helm, docker (cmd/mod.rs:92).
- app/troubleshoot/: pure rule checks over pods, PVCs, ReplicaSets returning severity-sorted DisplayFindings.
- ui/: ratatui rendering (theme.rs, utils.rs, resource_tabs.rs, overview.rs). handlers/mod.rs (3830 lines) handles keys. config.rs: user config (serde_saphyr, keybindings, themes).

Takeaway: kdash is a poll-and-list TUI tightly coupled to its UI. Network reaches into App and renders in the same module as the model. The k8s layer is not separable as it stands. Reusable parts are individual functions, not the architecture.

### 1.3 Resources supported and how each is fetched (IoEvent, network/mod.rs:60)
- Typed via Api::<K>::list(&ListParams::default()): Nodes, Namespaces, Pods, Services, ConfigMaps, StatefulSets, ReplicaSets, Deployments, Jobs, DaemonSets, CronJobs, Secrets, ReplicationControllers, StorageClasses, Roles, RoleBindings, ClusterRoles, ClusterRoleBindings, Ingress, PVCs, PVs, ServiceAccounts, NetworkPolicies, Events (core/v1 Event, NOT events.k8s.io).
- Generic helpers get_namespaced_resources<K,T,F> and get_resources (mod.rs:629/654) pick Api::namespaced or Api::all based on selected namespace.
- Drill-down: get_pods_by_selector uses ListParams::labels(..); get_pods_by_node uses fields("spec.nodeName=..").
- Pagination: ListParams::default() everywhere, so NO limit/continue. Every list loads the full result set into memory.
- Dynamic resources (CRDs etc.): discover_dynamic_resources (mod.rs:912) calls client.list_api_groups(), then one discovery::pinned_group(client, &gv) per group (N+1 requests). Uses recommended_resources(), filters to the list verb, excludes ~24 typed kinds. Items come from Api<DynamicObject>::all_with / namespaced_with(&ApiResource) (get_dynamic_resources, mod.rs:733). Cells are only NAME, NAMESPACE, AGE. Server printer columns are not used.
- Mutations, all through Api<DynamicObject> via api_resource_for_block (app/dynamic.rs): delete_resource (mod.rs:760); patch_resource (mod.rs:797) with Patch::Merge bodies from the ResourcePatch enum (rollout-restart annotation, cordon, cronjob suspend, scale replicas); trigger_cronjob (mod.rs:844) clones jobTemplate into a Job with generateName and ownerReference.
- Metrics: app/metrics.rs:40-68 hand-defines NodeMetrics and Usage with manual k8s_openapi::Resource and Metadata impls for metrics.k8s.io/v1beta1, then Api::<NodeMetrics>::all().list() (nodes.rs:265). Utilisation view delegates entirely to kubectl_view_allocations::{collect_from_nodes, collect_from_pods, collect_from_metrics, make_qualifiers}. Missing metrics-server errors are swallowed (nodes.rs:279).
- Logs (stream.rs): stream_container_logs (line 126) uses Api<Pod>::log_stream(&name, &LogParams{container, follow:true, tail_lines, since_seconds, timestamps}). Reconnects in a loop with backoff, since_seconds=RECONNECT_OVERLAP_SECS, dedup HashSet over last N lines, batched (BATCH_SIZE, BATCH_FLUSH_MS), cancelled via a watch::channel. fetch_previous_logs (313) uses Api::logs with previous:true. stream_pod_all_container_logs (375) and stream_aggregate_logs (502) do multi-container and selector fan-in with short_pod_name prefixes (805).
- NOT through kube-rs, all via the kubectl binary: describe (cmd/mod.rs:231 get_describe), edit (cmd/edit.rs), exec shell (cmd/shell.rs), port-forward (stream.rs:665 spawns `kubectl port-forward` with piped stdout/stderr; cmd/port_forward.rs builds the command). The "Describe" view (ui/utils.rs:767 draw_describe_block) is a text view: YAML for most kinds, kubectl describe output for those dispatching IoCmdEvent::GetDescribe.

### 1.4 Reusable-code candidates for a hexagonal kube-adapter crate

| Candidate | file:function | What it does | Verdict |
|---|---|---|---|
| Multi-path KUBECONFIG load, tolerant of blank/missing files | network/mod.rs:243-333 is_blank_kubeconfig, load_kubeconfig_path, load_kubeconfig_from_paths, load_local_kubeconfig | Splits KUBECONFIG, skips blank/missing files, merges via Kubeconfig::merge | Good. kube's Kubeconfig::from_env errors on any bad path, so this tolerance has real value. Small, MIT. |
| Per-context client | network/mod.rs:335-404 load_client_config, load_client_config_from_kubeconfig, get_client | KubeConfigOptions{context}, Config::from_custom_kubeconfig, Config::incluster fallback, HTTPS_PROXY env fallback onto config.proxy_url, Client::try_from(config) | Good as template. Rewrite for multi-client (HashMap<ContextName, Client>); kdash holds one client. |
| Auth-retry heuristic | mod.rs:171-241 refresh_kube_config, should_retry_kubectl_refresh, run_kubectl_cluster_info | On Error::Auth/401/exec-plugin errors, shells `kubectl cluster-info` with timeout to refresh creds, then retries | Pragmatic. kube handles exec plugins itself; reuse only the error classification. |
| Kubeconfig live-reload | event/events.rs:135-210 start_kubeconfig_watcher (notify); mod.rs:587-626 get_kube_config | File watcher plus 60 s safety poll; detects external current-context change | Good idea; reimplement behind a port trait. |
| Contexts list | app/contexts.rs:25-60 KubeContext::from_api, get_contexts | NamedContext -> name/cluster/user/ns/active | Trivial. |
| Dynamic discovery | network/mod.rs:911-1007 discover_dynamic_resources, preferred_group_version | list_api_groups + pinned_group + verb filter | Do NOT copy. Use Discovery::new(client).run_aggregated() (one request). |
| Dynamic API construction | mod.rs:733-755; app/dynamic.rs KubeDynamicKind{kind, scope, api_resource} | Api::all_with / namespaced_with by Scope | Pattern good and tiny. |
| Patch builders | mod.rs:119-169 ResourcePatch::to_merge_patch | Rollout restart, cordon, suspend, scale | Good. For scale prefer the scale subresource (Api::patch_scale). |
| Cronjob trigger | mod.rs:844-909 trigger_cronjob | Mirrors `kubectl create job --from=cronjob/x` | Good, copy. |
| Pod status derivation | app/pods.rs:596-713 get_status, is_pod_init (714), get_container_state (579), KubeContainer::from_api (546), get_container_ports (719) | Port of kubectl's pod printer (Init:N/M, reason, NodeLost) | Valuable, but the init-container branch at line 621 looks buggy: it checks st.reason.is_empty() then unwraps st.reason in the same branch (condition appears inverted). Port the Go printer logic fresh rather than copying verbatim. |
| Age and format | app/utils.rs:12-72 to_age, to_age_secs, duration_to_age | Compact "5d3h" ages (chrono) | Rewrite on jiff; tests reusable. |
| Quantity parsing | utils.rs:74-122 mem_to_mi, cpu_to_milli, to_cpu_percent, to_mem_percent | String suffix matching | NAIVE: only handles Ki/Gi and m/n; fails for Mi, u, k, E, decimals. Use a real parser (kubectl-view-allocations qty::Qty, CC0, or deskribe's quantity.rs). |
| Metrics types | app/metrics.rs:29-96 | Hand-rolled NodeMetrics | Prefer the k8s-metrics crate (part 2). |
| Log streaming with reconnect + dedup | network/stream.rs:126-307 (+ stream_single_pod_for_aggregate 853, collect_pod_container_info 811) | Follow logs, reconnect with overlap, batch, dedup | Good logic. Extract as a LogStream port exposing async Stream<Item=LogLine>, no App coupling. |
| Troubleshoot rules | app/troubleshoot/{pod,pvc,rs}.rs, mod.rs evaluate_findings, types.rs | Pure fns: object -> Option<DisplayFinding>, Severity ordering | Reusable domain logic, but built on Kube* display models; re-key on k8s_openapi types. |
| Error cleaning | utils.rs:142-235 sanitize_error_message | Strips module paths from error chains | Optional. |
| sanitize_obj | utils.rs:5 | Clears managedFields before YAML | Trivial. |
| YAML | models.rs:44 resource_to_yaml via serde_saphyr::to_string | | kube 4 itself now uses serde-saphyr too. |

### 1.5 Limitations and known issues
- Polling only (no watch); each tick re-lists the full object set; no pagination.
- Single global client and global context. Switching context rebuilds everything (refresh_client resets the app).
- describe, edit, exec and port-forward depend on an installed kubectl. Port-forward is a child process rather than Api::portforward.
- Table API (server printer columns, CRD additionalPrinterColumns) unused; CRDs only show NAME/NAMESPACE/AGE.
- Metrics errors swallowed; quantity parsing naive; utilisation outsourced to kubectl-view-allocations.
- Events use core/v1 only. Mixed chrono and jiff after the k8s-openapi 0.28 migration.
- README known issue: clipboard on Linux/aarch64. Upstream has 4 open issues, all dependabot PRs; project is healthy (2.5k stars, last push 2026-09-24).
- handlers/mod.rs (3.8k lines) and app/mod.rs (2.7k lines) are monoliths, poor candidates for lifting wholesale.

---

## PART 2/3: Rust Kubernetes ecosystem capability matrix

(crates.io data as of 2026-10-03; kube 4.2.0 source read from raw GitHub tag 4.2.0)

### 2.1 Versions
- kube / kube-client / kube-core / kube-runtime / kube-derive: 4.2.0 (2026-07-22), Apache-2.0, MSRV 1.89, lockstep. Releases: 3.0.0 (2026-01-12, jiff replaces chrono), 3.1.0 (03-17), 4.0.0 (06-16, k8s 1.36 via k8s-openapi 0.28, serde-yaml->serde-saphyr, RetryPolicy on by default, Store::state_filtered, Api<PartialObjectMeta> auto metadata calls), 4.2.0 (07-22, https proxy support). 3.0 also added Discovery::run_aggregated and merged ErrorResponse into Status.
- k8s-openapi 0.28.0 (2026-06-15), Apache-2.0. Features: v1_32, v1_33, v1_34, v1_35, v1_36, earliest, latest, schemars, schemars08, std. kube 4.2 pins ^0.28.0. Uses jiff for times.
- k8s-metrics 0.28.0 (2026-06-17), Apache-2.0, 113k downloads, repo rkubectl/k8s-metrics-rs (8 stars). Tracks k8s-openapi versions (0.26/0.27/0.28 releases). Deps: k8s-openapi ^0.28, serde, thiserror, constcat, go-parse-duration; kube ^4.0 dev-only.
- kube-cel 0.8.0 (2026-06-19): local CEL validation, optional.
- deskribe 0.1.2 (2026-09-15): kubectl describe in Rust (see 2.4).
- serde-saphyr 1.3.0 (2026-09-16): YAML, used by kube 4 (pins ^0.0.29 internally) and kdash.
- serde_yaml 0.9.34+deprecated (2024-03). serde_yaml_ng 0.10.0 (2024-05, stale). serde_yml 0.0.13: RUSTSEC-2025-0068 "unsound and unmaintained" - AVOID.
- json-patch 4.2.0; similar 3.2.0 (2026-08); imara-diff 0.2.0 (2025-06).
- tokio 1.53.1; tower 0.5.3; tower-http 0.7.1 (kube pins ^0.6.4); tracing 0.1.44; secrecy 0.10.3 (kube ^0.10.2); backon 1.6.0 (kube-runtime backoff).
- jiff 0.2.37 (matches k8s-openapi/kube; use this, not chrono 0.4.45); humantime 2.4.0.
- ratatui 0.30.2; tui-term 0.3.4 (2026-04); vt100 0.16.2; alacritty_terminal 0.26.0 (2026-04); portable-pty 0.9.0 (STALE, 2025-02; fork portable-pty-psmux 0.9.7 updated 2026-08).
- wasmtime 49.0.2 (2026-10-02), Apache-2.0 WITH LLVM-exception, MSRV 1.96; wasmtime-wasi / wasmtime-wasi-http 49.0.2; wit-bindgen 0.62.0; extism 1.30.0.
- agent-client-protocol 2.2.0 (2026-09-18), Apache-2.0, MSRV 1.88. 2.0.0 landed 2026-07-23 (1.0.0 only 2026-06-24). oci-client 0.18.0 (2026-09).
- gtmpl 0.7.1 (2021, unmaintained); gotpl 0.2.6 (FFI wrapper over real Go templates).

### 2.2 kube 4.2.0 feature flags (from crates.io metadata)
- default = client, rustls-tls, ring.
- TLS: rustls-tls, openssl-tls, aws-lc-rs, ring, webpki-roots. Pitfall: aws-lc-rs needs a C toolchain; choose ONE provider (mixed providers can panic at runtime). Use default-features=false and pick explicitly (kdash: rustls-tls; sofka and deskribe example: rustls-tls + aws-lc-rs).
- Auth/network: oauth (GCP via tame-oauth), oidc, ws (exec/attach/port-forward via tokio-tungstenite 0.29), gzip, http-proxy, socks5.
- Other: config, client, runtime, derive, jsonpatch, admission, cel, unstable-client, unstable-runtime, kubelet-debug, hyper-util-tracing.

### 2.3 Capability matrix (verified in 4.2.0 source)
- Kubeconfig: Kubeconfig::read() (KUBECONFIG else default path), read_from, from_env() (splits KUBECONFIG, merges, but errors on any bad path), merge, from_yaml. Config::from_kubeconfig(&KubeConfigOptions{context, cluster, user}), from_custom_kubeconfig, infer(), incluster(), incluster_env/dns. [kube-client config/mod.rs:212-361, file_config.rs:441-554]
- Auth: exec credential plugins (ExecConfig, ExecInteractiveMode, ExecAuthCluster / provideClusterInfo), token file re-read ~1/min, static token/basic, client certs, GCP auth_provider (feature oauth), OIDC (feature oidc). Exec runs on the blocking pool during refresh. [client/auth/mod.rs]
- Client Config knobs: connect_timeout, read_timeout (default None to protect long-lived exec/attach/port-forward; watcher has its own idle timeout), write_timeout, accept_invalid_certs, disable_compression, proxy_url, tls_server_name, headers, default_retry. RetryPolicy::server_retry() retries 429/503/504, on by default as of 4.0. gzip via tower-http decompression when feature gzip. root_cert_file reloaded ~60s. [config/mod.rs:128-175, builder.rs:228-251]
- Discovery: kube::discovery::Discovery::new(client).filter/exclude(..).run() or .run_aggregated() (k8s >=1.30 aggregated discovery); groups(), groups_alphabetical, get, has_group, resolve_gvk -> (ApiResource, ApiCapabilities); ApiGroup; pinned_group; raw Client::list_api_groups, list_api_group_resources, list_core_api_*, list_api_groups_aggregated, apiserver_version. [discovery/mod.rs, client/mod.rs:414-525]
- Dynamic types: DynamicObject, ApiResource, GroupVersionKind, GroupVersionResource, Api::all_with / namespaced_with / default_namespaced_with.
- Core CRUD: get, get_opt, get_with(GetParams), list(ListParams{label_selector, field_selector, timeout, limit, continue_token, version_match, resource_version}), create, replace, delete (Either<K,Status>), delete_collection, patch, watch; metadata variants get_metadata*, list_metadata, patch_metadata, watch_metadata return PartialObjectMeta via Accept application/json;as=PartialObjectMetadata. [api/core_methods.rs]
- Patch: Patch::Apply (server-side apply, PatchParams::apply("manager")), Merge, Strategic, Json (feature jsonpatch). [kube-core params.rs:605]
- Subresources (api/subresource.rs): get_scale/patch_scale/replace_scale; get/patch/replace_status; evict; ephemeral containers get/patch/replace_ephemeral_containers; pod resize get/patch/replace_resize; logs; log_stream (AsyncBufRead); attach; exec; portforward; generic get/create/patch/replace_subresource.
- Logs: LogParams{container, follow, limit_bytes, pretty, previous, since_seconds, since_time (jiff Timestamp), tail_lines, timestamps} [kube-core subresource.rs:17-43].
- Exec/attach: AttachParams -> AttachedProcess (stdin/stdout/stderr streams, join, abort, take_status, terminal-size sender via TerminalSize{width,height} when tty; needs feature ws) [remote_command.rs]. Port-forward: Portforwarder::take_stream(port) -> AsyncRead+AsyncWrite, take_error(port), abort, join [portforward.rs:128-148]. Client::connect gives a raw websocket Connection.
- Raw escape hatch: Client::request / request_text / request_stream / request_status / send / connect(http::Request<Vec<u8>>); kube::core::Request::new(path).list/watch(..) builds the http::Request and you can replace the Accept header. [client/mod.rs:217-340]
- Watcher/reflector (kube-runtime): watcher(api, watcher::Config), metadata_watcher, watch_object. Config{label_selector, field_selector, timeout (default 290s, max 295), list_semantic, initial_list_strategy, page_size, ...}. InitialListStrategy::{ListWatch (default, paginated list then watch), StreamingList} via Config::streaming_lists() (needs WatchList feature gate, k8s 1.27 opt-in). reflector(writer, stream), Store<K> (state(), state_filtered since 4.0), backoff via backon. kube-rs issue #2010 (open): make StreamingList default in 6.0 for k8s >=1.34. [watcher.rs:196-420, 787-879]
- Events: typed via k8s-openapi core::v1::Event and events::v1::Event as normal Api<K> (both available); kube-runtime has an events recorder module.
- Table API: NOT supported in kube. grep of kube-client shows only PartialObjectMetadata(List) content types [client/mod.rs:679-769, kube-core metadata.rs]. No Table type anywhere in kube/k8s-openapi.
- Metrics types: not in kube or k8s-openapi (see 2.3b).
- CEL: kube-cel 0.8 with #[kube(cel)] derive.
- Large clusters: ListParams::limit/continue_token, watcher page_size, gzip feature, default retry on 429/503/504, StreamingList, metadata-only watchers (Api<PartialObjectMeta> auto-uses metadata API in 4.0).

#### 2.3a Table API workaround (proven prior art)
- sarub0b0/kubetui (MIT) src/kube/client.rs:9 sets Accept "application/json;as=Table;v=v1;g=meta.k8s.io,application/json;as=Table;v=v1beta1;g=meta.k8s.io,application/json" and deserialises its own Table / TableRow / TableColumnDefinition (src/kube/apis/v1_table.rs, src/kube/table.rs; columnDefinitions have name/type/format/description/priority).
- nklmilojevic/sofka (MIT OR Apache-2.0) src/k8s/table.rs: builds a kube::core::Request, appends "&includeObject=Metadata", sets Accept "application/json;as=Table;g=meta.k8s.io;v=v1,application/json", sends via the Client, does a Table list+watch with a 30 s refresh interval, 15 s request timeout and 5 s retry; gives server-side columns incl. CRD additionalPrinterColumns. Also src/server_table.rs.
- Other users: databricks/click src/k8s_table.rs (Apache-2.0), Ramilito/kubectl.nvim kubectl-client/src/cmd/get.rs (Apache-2.0).
- Verdict: implement ourselves on Client::request_text/request (~150 lines). No crate exists. Fall back to plain JSON when Accept is not honoured (aggregated APIs).

#### 2.3b Metrics types
k8s-metrics 0.28.0 provides k8s_metrics::v1beta1::{NodeMetrics, PodMetrics,...} implementing k8s_openapi::Resource + Metadata; README example: Api::<metricsv1::PodMetrics>::namespaced(client.clone(), ns).list(&lp). README says "portions copied from kdash". kubetui and kdash hand-roll instead (kdash metrics.rs:29-96; kubetui src/kube/apis/metrics.rs). Recommend k8s-metrics, with a thin internal fallback type (single maintainer, 8 stars).

### 2.4 kubectl describe in Rust
deskribe 0.1.2 (Apache-2.0, nklmilojevic, 2 stars but used by sofka): "Kubernetes resource descriptions in Rust, without starting kubectl". API: gather(client: kube::Client, &ApiResource, &DynamicObject) -> Description; Description::render(&RenderOptions) (no network; options for now/timezone); fetch(..) = gather + render returning (DynamicObject, String). Fetches fresh object (checks UID), related resources and events; follows paginated lists; overlaps independent requests; HPA v2->v1 and ServiceCIDR/IPAddress v1->v1beta1 fallbacks. 36 specialised kinds plus generic format for CRs. Ported from kubectl pkg/describe v0.35.1 (+v0.37.0 behaviour), apimachinery quantity/duration, component-helpers resource accounting; upstream.json + CI (upstream.yaml) track drift. Deps: kube ^4.2 (client only), k8s-openapi ^0.28, x509-parser 0.18, futures-util, base64, serde_json, tokio. Caveats: "Experimental", output not guaranteed byte-identical to kubectl, v0.1.x single maintainer. NOTICE retains Kubernetes Authors' Apache-2.0 copyright - must be preserved if code is vendored. It is the only credible alternative to shelling out to kubectl describe; put it behind a Describer port with a kubectl-describe fallback adapter.

### 2.5 Helm from Rust (honest assessment)
- No maintained Rust crate does Helm install/upgrade/template. Looked at: gtmpl 0.7.1 (2021, partial Go-template semantics; Helm needs Sprig plus include/tpl/lookup/toYaml), gotpl 0.2.6 (FFI to real Go, needs Go toolchain), fluvio-helm 0.4.3 (2021, CLI wrapper), deislabs/pilothouse (abandoned 2019 experiment), helm-sdk 0.11 (unrelated AI tool-calling). GitHub repo searches for "helm template rust" / "helm chart renderer" returned nothing active.
- What Rust tools do: (1) native read-only inspection + helm binary for mutations - sofka src/helm.rs decodes release Secrets of type helm.sh/release.v1 (Secret data base64 -> base64 -> gzip -> JSON; same as Helm), lists latest revision per release, history, values, rendered manifest, NOTES.txt; rollback and uninstall shell out to `helm`. (2) shell out to helm (kdash only probes `helm version --short`). (3) Nobody reimplements chart rendering.
- Recommendation: HelmPort with two adapters: native secret-store reader (list/history/values/manifest/notes; also consider ConfigMap and SQL storage drivers) and a HelmCli adapter for template/upgrade/rollback/uninstall. oci-client 0.18 exists if native OCI chart pull is wanted later.

### 2.6 Other pieces
- YAML: serde-saphyr 1.3.0. kdash notes a behaviour difference vs old serde_yaml: saphyr resolves bare n/y/t/f differently (config.rs:100-105 sets options to avoid it). kubetui and sofka still pin serde_yaml 0.9.
- Diff: similar 3.2 (sofka :diff), json-patch 4.2 for RFC6902.
- Embedded terminal: kube's AttachedProcess already gives byte streams, so you need an emulator widget, not a PTY: vt100 0.16 + tui-term 0.3.4, or alacritty_terminal 0.26. portable-pty only for local shells.
- Plugins: wasmtime 49.0.2 (+wasmtime-wasi, wasmtime-wasi-http, wit-bindgen 0.62) for component-model plugins (heavy: MSRV 1.96). sofka uses process plugins instead: plugin.toml + adapter executables exchanging a JSON snapshot over stdin and JSON report on stdout, catalog-installed (sofka-plugins repo), with per-pipe 1 MiB capture limits, process-group cancel, sanitised stderr; docs state plainly there is no sandbox.
- ACP (agent-client-protocol 2.2.0; repo agentclientprotocol/rust-sdk, 212 stars, Apache-2.0): roles Client/Agent/Proxy/Conductor. Client shape: Client.builder().on_receive_notification(async |n: SessionNotification, _cx| ..., on_receive_notification!()).on_receive_request(async |r: RequestPermissionRequest, responder, _conn| responder.respond(RequestPermissionResponse::new(RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id)))), on_receive_request!()).connect_with(AcpAgent::from_str("cmd"), |conn: ConnectionTo<Agent>| async { conn.send_request(InitializeRequest::new(ProtocolVersion::V1)).block_task().await?; NewSessionRequest::new(cwd); PromptRequest::new(session_id, vec![ContentBlock::Text(TextContent::new(..))]) ... }). Types under agent_client_protocol::schema::v1::*. Transport AcpAgent = subprocess over stdio; extra crates agent-client-protocol-http (HTTP/SSE/WS), -rmcp, -conductor, -cookbook, -test. Features: unstable_protocol_v2 (draft v2), unstable_mcp_over_acp, unstable_session_fork, etc. Pitfall: major bump 1.x->2.0 within a month; pin exactly.

### 2.7 Other Rust Kubernetes UIs (licences)
- sofka (nklmilojevic/sofka): 1.6k stars, MIT OR Apache-2.0, v0.29.8, edition 2024, pushed 2026-10-02. k9s-style TUI on kube 4.2 + ratatui 0.30 + deskribe. Module layout (docs/architecture.md): main.rs (async select! loop), app.rs + app/*.rs (mode state machine), k8s.rs + src/k8s/{discovery,kubeconfig,proxy,table,completion}.rs, store.rs (generation-tagged Msg over mpsc), columns.rs, explain.rs (deterministic "why unhealthy"), adjacent.rs, timeline.rs, redact.rs, helm.rs, plugins*.rs. Features: watch tasks, Table API feed, native Helm inspector, Flux/Argo controls via API patches, bulk actions, background port-forwards, read-only guardrails, fleet multi-cluster, skins, process plugin catalog. kube features used: runtime, client, derive, ws, http-proxy, socks5, gzip, oidc, rustls-tls, aws-lc-rs; also similar, secrecy, zstd, flate2, nucleo-matcher. Closest prior art; read src/k8s/*.rs, store.rs, helm.rs, redact.rs first (I only read table.rs, Cargo.toml and docs).
- kubetui (sarub0b0): 397 stars, MIT, v1.14.0, kube 3.0 + k8s-openapi 0.27 (one version behind). Threads: UserInput, Tick, KubeWorker (tokio pollers), Render over crossbeam; own Table, metrics, Gateway API types; jaq/jmespath filtering.
- click (databricks): 1.5k stars, Apache-2.0, last push 2026-03; REPL k8s CLI.
- kdash: MIT. k7s 0.7.0 and kuberift 0.1.2 are small. kubectl-view-allocations (davidB): 806 stars, CC0-1.0, 3.1.0 - allocation/utilisation maths (Qty, Resource, make_qualifiers).
- Seabird (getseabird/seabird): Go + GTK4 (NOT Rust), 1.4k stars, MPL-2.0, last push 2025-08; UX reference only.

---

## PART 3/3: risks and gaps, unverified items, sources

### 3. Risks and gaps
1. Table API. Not in kube, so we build it. Prior art: kubetui and sofka (see part 2, 2.3a). Plan for: includeObject=Metadata (or Object); watch/refresh semantics differ from the typed watcher (sofka re-fetches every 30 s); fall back to plain application/json for aggregated APIs that ignore the Table Accept header (both projects include the plain-JSON fallback in the Accept list).
2. Describe. Two options: deskribe (native, experimental, 36 kinds, no byte-parity guarantee) or the kubectl binary (kdash). Make it a Describer port with both adapters. If we ever vendor deskribe code, preserve its NOTICE (Kubernetes Authors' Apache-2.0 copyright).
3. Helm. No Rust library. Native Secret decoding for read-only ops (helm.sh/release.v1: base64 -> base64 -> gzip -> JSON) plus the helm binary for mutations. A native chart renderer (Go templates + Sprig) is high-risk and out of scope.
4. Metrics types. k8s-metrics 0.28 exists but is single-maintainer and low-star: keep a thin internal fallback type. Metrics may be absent (no metrics-server): surface that state explicitly instead of swallowing it as kdash does (nodes.rs:279).
5. Quantity parsing. kdash's string helpers (utils.rs:74-122) are wrong for many inputs. Use a real parser (kubectl-view-allocations qty::Qty, CC0) or implement against the Kubernetes Quantity spec.
6. kdash reuse is limited. It is polling-based, single-client, with state and UI entangled. Copy only the small pure functions in the part 1 table (kubeconfig loader, cronjob trigger, patch builders, log reconnect logic, troubleshoot rules). Do not copy its network/app architecture. Re-derive get_status (pods.rs:596) because of the likely inverted init-container condition at line 621.
7. Multi-cluster. kube clients are per-Config. Build a ClientPool keyed by context with lazy auth. Exec-plugin auth can block or prompt: kube supports ExecInteractiveMode, but an interactive plugin inside a raw-mode TUI needs deliberate handling (suspend the TUI or forbid interactive mode).
8. Large clusters. Prefer metadata_watcher for list views, page_size on the watcher, Table + StreamingList (k8s >= 1.27 gate; default-on only in later k8s), full-object fetches on demand. kdash's full relist per tick does not scale.
9. Version churn. kube went 1.0 (2025-05) to 4.2 (2026-07) in about 14 months; jiff in 3.0, serde-saphyr in 4.0. k8s-openapi must match kube's pinned minor (0.28 line). Pin and bump together. MSRVs: kube 1.89, ACP 1.88, wasmtime 49 needs 1.96.
10. TLS provider. Choose once (ring vs aws-lc-rs); mixing causes runtime panics. Depend on kube with default-features=false and select explicitly; aws-lc-rs needs a C toolchain.
11. Plugin systems. wasmtime gives sandboxing but is heavy (MSRV 1.96, big compile). Process plugins (sofka style) are cheap but unsandboxed. Product decision.
12. ACP. v2 is behind unstable_protocol_v2; major version moved 1.x -> 2.x quickly. Isolate behind an AgentPort and pin exactly.
13. Licences. Compatible permissive: kdash (MIT), kubetui (MIT), sofka (MIT OR Apache-2.0), click (Apache-2.0), deskribe (Apache-2.0 + Kubernetes NOTICE), k8s-metrics (Apache-2.0), kubectl-view-allocations (CC0), kube / k8s-openapi / ACP / wasmtime (Apache-2.0, wasmtime with LLVM exception). Reference only: Seabird (MPL-2.0, Go).

### Not verified
- kdash was not run or built; findings come from reading source and crates.io/GitHub metadata.
- docs.rs pages were not read item by item; kube API shapes come from the 4.2.0 source tree.
- wasmtime component-model API and ACP v2 details beyond the example client and feature list are unchecked.
- sofka internals: only src/k8s/table.rs, Cargo.toml, README and docs/{architecture,features,plugins}.md were read; src/helm.rs, store.rs, redact.rs were not opened (behaviour taken from its docs).
- A WebFetch summary of the kube-rs releases page gave wrong years; the dates above come from `gh api repos/kube-rs/kube/releases`.

### 4. Sources
- https://github.com/kdash-rs/kdash (cloned; Cargo.toml, LICENSE, README.md, src/network/{mod,stream}.rs, src/app/{mod,pods,metrics,nodes,dynamic,utils,contexts,models}.rs, src/cmd/*, src/event/events.rs, src/main.rs)
- crates.io API (https://crates.io/api/v1/crates/<name>[/<version>[/dependencies]]) for kube, kube-client, kube-core, kube-runtime, kube-derive, k8s-openapi, k8s-metrics, kube-cel, deskribe, serde-saphyr, serde_yaml, serde_yaml_ng, serde_yml, json-patch, similar, imara-diff, portable-pty, alacritty_terminal, secrecy, tower, tower-http, humantime, jiff, chrono, tracing, tokio, wasmtime, wasmtime-wasi, wasm-component-ld, extism, wit-bindgen, agent-client-protocol, ratatui, tui-term, vt100, backon, oci-client, gtmpl, gotpl, kubectl-view-allocations, kubetui
- kube-rs 4.2.0 source: https://github.com/kube-rs/kube (kube-runtime/src/watcher.rs; kube-client/src/{api/core_methods,api/subresource,api/portforward,api/remote_command,client/mod,client/builder,client/auth/mod,config/mod,config/file_config,discovery/mod}.rs; kube-core/src/{params,subresource,metadata}.rs); releases via `gh api repos/kube-rs/kube/releases`; issue #2010
- https://github.com/rkubectl/k8s-metrics-rs (README)
- https://github.com/nklmilojevic/deskribe (README, NOTICE, Cargo.toml)
- https://github.com/nklmilojevic/sofka (README, Cargo.toml, docs/architecture.md, docs/features.md, docs/plugins.md, src/k8s/table.rs)
- https://github.com/sarub0b0/kubetui (Cargo.toml, CLAUDE.md, src/kube/client.rs, src/kube/apis/v1_table.rs)
- https://github.com/databricks/click, https://github.com/Ramilito/kubectl.nvim, https://github.com/getseabird/seabird, https://github.com/davidB/kubectl-view-allocations
- https://github.com/agentclientprotocol/rust-sdk (README, src/agent-client-protocol/examples/yolo_one_shot_client.rs)
- https://rustsec.org/advisories/RUSTSEC-2025-0068.html (serde_yml)
