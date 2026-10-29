# oxikube_kube kind integration suite

Every data-plane port method of epic E04 runs here against a real API server (E04-S14). The
files are per story; this page maps each port method and each acceptance item of the epic to the
test that exercises it, so a gap is visible in review.

## Running

```
cargo xtask kind-up                      # cluster `oxikube`, metrics-server, Widget CRD, fixtures
OXIKUBE_TEST_CONTEXT=kind-oxikube cargo test -p oxikube_kube --features integration
OXIKUBE_TEST_CONTEXT=kind-oxikube cargo test -p oxikube_kube --features integration --test <file>
```

Without `OXIKUBE_TEST_CONTEXT` every test returns early, so `cargo test --workspace` stays green
without a cluster. CI runs the suite in `.github/workflows/integration.yml` on PRs that touch
adapters, ports, app, testing or xtask, and nightly twice in a row as a flake check; on failure it
uploads each failed test's namespace events and pod descriptions (saved by `TestNamespace`
before it deletes the namespace, into `OXIKUBE_TEST_DIAGNOSTICS_DIR`), the cluster-wide events,
pods, nodes and the kind logs. Locally, without that variable, a failed test prints its
namespace's events and pod descriptions to stderr.

## Rules every test follows

- Its own `oxi-test-<rand>` namespace (`TestNamespace`, deleted on drop, its events saved first
  when the test failed) and a random suffix on every cluster-scoped object (`TestCrd`,
  `FakeNode`), so suites run concurrently on one cluster.
- The real node is never cordoned, tainted or drained: drain and cordon target a `FakeNode` no
  kubelet backs. Shared fixtures (`oxikube-fixtures`, the `Widget` CRD) are only read.
- No sleeps as synchronisation: `common::wait_until`, or `common::wait_in` which dumps the
  namespace's events when it times out. Waiting for a pod goes through `common::pods`
  (`wait_started`, `wait_ready`): time spent pulling an image is charged to its own budget, not
  the start deadline, an unusable image fails at once, and every failure prints the pod's states,
  its scheduling and its events (`pod_waits.rs`).
- Images come from `oxikube_testkit::images` and the list `cargo xtask kind-up` pre-pulls
  (`fixtures/test-images.txt`); a test never relies on an image that only some clusters cache.
- Bulk pods that must stay `Pending` are `pending_pod`s: they name a scheduler nobody runs, so they
  cost the API server but not the real scheduler. A pod made unschedulable with a `nodeSelector`
  is recorded by the scheduler on every attempt through one throttled client, and a few thousand of
  them delay the scheduling of every other test's pod by half a minute (`unschedulable_pod` is for
  the one or two tests that assert the scheduler's verdict).
- Errors are asserted by `ErrorKind`, never by HTTP code; nothing prints a token or an object
  payload.

## Port methods

| Port | Method | Test |
|---|---|---|
| `ResourceReader` | `list` | `resources_pagination` (2 000 pods, 8 pages), `resources_reads::label_and_field_selectors_return_the_expected_subsets`, `resources_special` (resourceVersion semantics, CRs) |
| | `list_metadata` | `resources_port::list_metadata_pages_selects_and_reads_custom_and_cluster_kinds`, `resources_port::list_metadata_without_rights_is_forbidden` |
| | `get`, `get_opt` | `resources_reads::get_get_opt_and_cluster_scoped_kinds`, `resources_reads::typed_and_dynamic_paths_return_identical_resources` |
| | `watch` | `resources_port::a_port_watch_folds_create_update_and_delete`, `resources_port::a_metadata_only_port_watch_carries_partial_objects`; the feeds below it in `feed_reflector`, `feed_metadata` |
| | `get_scale`, `get_subresource` | `subresources_scale` (Deployment and CRD scale, status), `subresources_pods` |
| `ResourceWriter` | `create`, `replace`, `patch` | `mutations::create_replace_and_the_three_patch_kinds_round_trip`, `mutations::replace_with_a_stale_resource_version_is_a_stale_version_conflict`, `mutations::custom_resources_accept_merge_and_apply_but_not_strategic_patches` |
| | `delete`, `delete_collection` | `mutations_delete` (each propagation policy, dry run, selectors) |
| | `scale` | `subresources_scale::scaling_a_deployment_goes_through_the_scale_subresource_and_reads_back` |
| | `evict` | `subresources_pods::evicting_a_pod_without_a_budget_deletes_it`, `subresources_pods::a_blocking_budget_refuses_the_eviction_as_a_retryable_error` |
| | `create_subresource` | `resources_port::create_subresource_posts_an_eviction_and_classifies_failures` |
| | `patch_subresource` | `subresources_pods` (ephemeral containers, resize), `subresources_scale`, `algorithms_drain` (pod status) |
| | `replace_subresource` | `subresources_scale::a_crd_with_scale_and_status_subresources_serves_both` |
| `TableFeedPort` | `list_table`, `table_feed` | `table_feed` (CRD printer columns and pods against `kubectl get`, live create/update/delete, paged lists) |
| `LogPort` | `stream_logs` | `logs_streams` (options, multi-container, selector fan-in, rate), `logs_restart` (restart without dupes or gaps) |
| `ExecStreamPort` | `exec_session`, `attach_session` | `exec_streams` (echo, large payload, resize, exit status, attach, error kinds), `exec_node_shell` |
| `ExecTransferExt` (on `ExecStreamPort`, E20-S06) | `tar_download`, `tar_upload`, `cat_download` | `exec_transfer` (100 MiB download against `sha256sum` in the pod with throughput, progress and memory printed; upload with content and mode; every byte value round-tripping; `cat`; a cancel that leaves no `tar` in the pod; `Unsupported` without `tar`) |
| `ExecPort` | `exec`, `attach`, `create_debug_container`, `node_shell` | `exec_terminal` (TTY echo, resize, exit code, attach and kill, debug container, node shell removed on kill, `NotFound`), `exec_scenarios` (attach to a long-running logger pod, a debug container in a pod with no shell) |
| `TerminalBackend` (`KubeStream`) | `write`, `resize`, `output_stream`, `kill`, `reconnect` | `exec_kube_stream` (`interactive_tty` echo, `stty size` after a resize, non-zero exit, missing container, `Forbidden` without `pods/exec`, reconnect to a fresh shell, 50 open/drop cycles leave no task, flood throughput) |
| `PortForwardPort` | `forward` | `portforward` (pod and service GET, named ports, errors, `Forbidden`), `portforward_restart` |
| `MetricsPort` | `node_metrics`, `pod_metrics` | `metrics_kind` (present, absent as `Unavailable(NotInstalled)`, `Forbidden` as `Unavailable(Forbidden)`, not an error) |
| `KubeEvents` | `watch`, `EventsPort::watch_events` | `events_feed` (one API by default, forced core, merged both, per-object feed, capacity, through the connection ports), `events_rbac` (namespace-scoped RBAC fallback and forbidden namespaces) |
| `WarningPort` | `subscribe` | `warnings_kind` (a PodSecurity `Warning:` header from the real API server reaches the port), plus unit tests of the header parser and the layer |
| `FeedSource` (watch budget) | `open` via `FeedRegistry` | `budget_kind`, `budget_tracing` |
| algorithms | `trigger_cronjob`, `rollout_history`, `rollout_undo`, `drain` | `algorithms_workloads`, `algorithms_drain` |

## Epic E04 acceptance items

| Item | Test |
|---|---|
| Table API on a CRD with `additionalPrinterColumns`, and on pods | `table_feed::crd_printer_columns_match_kubectl_get`, `table_feed::pods_match_kubectl_get_pods_o_wide` |
| SSA apply conflict returns `Conflict` naming the field managers | `mutations::server_side_apply_conflicts_name_the_other_manager_and_force_takes_over` |
| SSA apply the kind's schema cannot read (the server's `500` "failed to create typed patch object") is a `Validation` error naming the fields (E10-S13) | `mutations::an_apply_the_schema_cannot_read_names_its_fields` |
| Dry run returns the server's would-be object | `mutations::dry_run_returns_the_would_be_object_and_changes_nothing`, `mutations_delete::a_dry_run_delete_leaves_the_object` |
| Logs survive a restart without duplicated or missing lines | `logs_restart::a_container_crash_mid_stream_leaves_no_duplicate_and_no_gap` |
| Exec round-trips stdin/stdout with resize | `exec_streams::a_tty_shell_echoes_and_reports_the_round_trip_latency`, `exec_streams::a_resize_is_observed_by_stty` |
| Port-forward GET | `portforward::forward_to_an_nginx_pod_and_get`, `portforward::forward_through_a_service_maps_the_service_port_to_the_named_target_port` |
| Eviction and drain with a PodDisruptionBudget | `subresources_pods::a_blocking_budget_refuses_the_eviction_as_a_retryable_error`, `algorithms_drain::a_drain_waits_out_a_budget_that_blocks_then_allows_and_evicts_everything` |
| Metrics present, and absence reported (not swallowed) | `metrics_kind::node_metrics_report_usage_and_utilisation_for_every_node`, `metrics_kind::a_cluster_without_the_metrics_group_is_unavailable_not_an_error` |
| 403 for an RBAC-restricted service account | `rbac`, `resources_special::a_restricted_account_gets_forbidden_not_a_crash`, `resources_port::list_metadata_without_rights_is_forbidden`, `metrics_kind::an_account_without_rights_on_metrics_gets_forbidden` |

## Epic E09 acceptance items (terminal and exec, E09-S13)

The terminal suite spans four places; each row names the test that proves the item.

| Item | Test |
|---|---|
| exec `echo` round trip | `exec_terminal::exec_gives_a_tty_with_resize_and_an_exit_code`, `exec_kube_stream::echo_round_trips_a_resize_reaches_stty_and_the_exit_code_propagates` |
| resize reaches the pod (`stty size`) | the same two; through the app: `bins/oxikube/tests/kind_terminal::a_full_screen_program_in_a_pod_renders_and_follows_a_resize` |
| non-zero exit code propagates | `exec_terminal` (`exit 3`), `exec_kube_stream`, `exec_streams::the_exit_status_and_separate_stderr_are_propagated` |
| attach to a long-running pod and receive its output | `exec_scenarios::attaching_to_a_long_running_pod_receives_what_it_keeps_writing` (a logger pod), `exec_terminal::attach_round_trips_and_kill_ends_the_stream` (`cat`) |
| node shell: `hostname` equals the node, the pod is cleaned up on close | `exec_terminal::a_node_shell_through_the_port_removes_its_pod_on_kill`, `exec_node_shell`; through the guard: `oxikube_app/tests/kind_smoke/node_shell` |
| ephemeral debug container: status appears, attach works | `exec_terminal::a_debug_container_is_created_and_attached_through_the_port`, `exec_node_shell::an_ephemeral_debug_container_is_attached_to`, `exec_scenarios::a_debug_container_reaches_a_pod_that_has_no_shell`; through the guard and the UI: `oxikube_app/tests/kind_smoke/debug`, `bins/oxikube/tests/kind_exec` |
| a read-only session blocks node shell and debug | `oxikube_app/tests/kind_smoke/node_shell::a_read_only_cluster_and_a_missing_permit_create_nothing`, `oxikube_app/tests/kind_smoke/debug::a_container_that_cannot_start_and_a_read_only_session_are_readable_errors` |
| a shell from the pod table, audited, output in the grid | `bins/oxikube/tests/kind_exec` |
| vi and top render, follow a resize, never scroll | `bins/oxikube/tests/kind_terminal::a_full_screen_program_in_a_pod_renders_and_follows_a_resize`; recorded vim, less, tmux: `oxikube_terminal/tests/captures.rs` |
| a flood stays within the scrollback | `bins/oxikube/tests/kind_terminal::a_flood_of_output_through_the_websocket_stays_within_the_scrollback` (prints `perf-jsonl`) |
| nothing typed or printed reaches the data dir | `bins/oxikube/tests/kind_terminal_data::nothing_typed_or_printed_in_a_terminal_reaches_the_data_dir`, `oxikube_terminal/tests/view/persist.rs` |
| golden screenshots of colours, bold and recorded programs (macOS nightly) | `oxikube_terminal/tests/screenshot.rs` (16 colours, 256 ramp, bold, italic, underline, strike, wide glyphs, cursors, selection), `tests/screenshot_programs.rs` (vim, less, tmux) |

Run it all: `cargo xtask kind-up`, then `cargo it` (this crate, the app's kind smoke, the testkit) and
`OXIKUBE_TEST_CONTEXT=kind-oxikube cargo test -p oxikube --features integration --test kind_exec
--test kind_terminal --test kind_terminal_data`. Pods come from `oxikube_testkit::integration::pods` (`sleeper`, `cat`,
`logger`, `shell_less`).

## Epic E13 acceptance items (metrics provider detection, E13-S05)

| Item | Test |
|---|---|
| the service proxy reaches a Service by port name and number, GET with parameters and form POST, the Service's own 404/405 told from the API server's | `service_proxy_kind::the_proxy_reaches_a_service_and_tells_its_answers_from_the_api_servers` |
| a Service that does not exist (404 with the API server's message), one with no ready endpoint (503, retried by the client until the deadline: a timeout that says so), a body over the limit (`truncated`) | the same |
| an RBAC-restricted account gets the API server's 403 as an answer, not an error | `service_proxy_kind::an_account_that_may_not_use_the_proxy_gets_a_403_that_says_so` |
| an account that may not list Services: detection ends in `Failed(Forbidden)` with the way out, every probe `Skipped` | `service_proxy_kind::detection_as_an_account_that_may_not_list_services_fails_as_forbidden_not_as_absent` |
| a path that could change the route never reaches the cluster | `service_proxy_kind::a_path_that_could_change_the_route_never_reaches_the_cluster` |
| detection of a real Prometheus Service through the proxy; the app checks a configured Service, queries it and shows it; automatic detection ends in a visible verdict | `bins/oxikube/tests/kind_metrics` |

## Epic E14 acceptance items (events and notifications, E14-S10)

The chain from the kubelet to the user: a pod whose image cannot be pulled
(`oxikube_testkit::integration::events::unpullable_pod`, image under `.invalid`) makes `Failed`
Warnings; `EventArrivals` (a `kubectl get events --watch` on the test's namespace) timestamps
them, so a latency is the app's own and not the kubelet's.

| Item | Test |
|---|---|
| the adapter: one API by default, forced `core/v1`, both merged, per-object feeds, RBAC fallback | `events_feed`, `events_rbac` (E14-S01) |
| a Warning reaches the `EventService` within 1 s of the watch seeing it, on `events.k8s.io/v1` and on `core/v1` | `oxikube_app/tests/kind_smoke/events_chain/fallback.rs` |
| `events.k8s.io` unavailable (the API server's own 404, discovery leaves the group out): the feed reads `core/v1` and the normalised events equal the `events.k8s.io/v1` ones | the same test |
| a Deployment's burst: one row per pod up to the group threshold, one group row for the rest, each raised once; mute holds new warnings back and is saved; unmute resumes | `oxikube_app/tests/kind_smoke/events_chain/notify.rs` |
| the running app: `:ev <ns>` and `ctrl-z` show the Warning within 1 s, the pods table's badge, the drawer's Events tab (`resource::ViewEvents`), the toast; muted, a new failing pod is in the view and not in the centre; unmuted, the next one notifies | `bins/oxikube/tests/kind_events.rs` |
| the notification centre in the app: the toast, the bell, the panel key, mute saved in the state db | `bins/oxikube/tests/kind_notifications.rs` (E14-S05) |

## Epic E15 acceptance items (port-forward manager, E15-S09)

The `PortForwardManager` over the real session (`KubeConnector`, so `KubePortForward`), the
shared `ResourceStores` and `SessionBackend`; nginx fixtures and the HTTP assertions are
`oxikube_testkit::integration::nginx`. Timings print as `perf-jsonl` lines; a reconnect is timed
from the replacement pod becoming Ready (polled on the API server) to the manager's `Active`.

| Item | Test |
|---|---|
| the adapter: `start`, traffic counters, `PodGone`, `BindFailed`, `PortClosed`, a Service through its named target port, `Forbidden` | `portforward_port`, `portforward`, `portforward_restart` (E15-S01, E04-S10) |
| a pod forward answers HTTP on an Auto port; its pod is recreated (StatefulSet) and the forward goes `Reconnecting` and back to `Active` on the same port, drop and recovery reported | `oxikube_app/tests/kind_smoke/portforward/pod.rs` |
| a Service forward goes to the EndpointSlice's ready pod; the pod is replaced (within 3 s of Ready); scale to zero waits, scale back recovers | `oxikube_app/tests/kind_smoke/portforward/service.rs` |
| an auto-start favourite comes back after a restart over the same SQLite file once the session is Ready | `oxikube_app/tests/kind_smoke/portforward/favourites.rs` |
| a closed remote port, a missing pod, a busy local port (with its suggestion) through the manager | `oxikube_app/tests/kind_smoke/portforward/errors.rs` |
| 50 concurrent forwards to one Service, each answering | `oxikube_app/tests/kind_smoke/portforward/concurrent.rs` |

## Epic E17 acceptance items (RBAC and access tooling, E17-S10)

The scenario is `oxikube_testkit::fixtures::rbac::RbacScenario`, applied per test with
`oxikube_testkit::integration::rbac::AppliedScenario`: two `oxi-test-<rand>-home` / `-other`
namespaces and, in `home`, the ServiceAccount `limited`, which may read pods there and nothing else
of its own. `kubectl auth can-i --list` rows are read back with
`oxikube_testkit::fixtures::rbac::printed_rules`; a mismatch saves both sides with
`integration::rbac::save_diagnostic` (rules and kubectl output only). The same objects drive the
offline `oxikube_app` test `access::tests::scenario`, so a fixture that drifts fails without kind.

| "Done when" | Test |
|---|---|
| (1) the sidebar hides what the user may not list, with a "restricted" indicator | `oxikube_app/tests/kind_smoke/rbac/gating.rs` (the sidebar's review and `AccessVisibility` over the account's generated kubeconfig), `bins/oxikube/tests/kind_rbac.rs` (the running app's sidebar: Pods shown, Config / Nodes hidden, "Limited access" line) |
| (2) the can-i matrix matches `kubectl auth can-i --list` | `rbac::rules_review_lists_what_kubectl_auth_can_i_list_prints` (the adapter's rules review), `oxikube_app/tests/kind_smoke/rbac/matrix.rs` (the subject index and its per-verb matrix, before and after a binding) |
| (3) the create-binding dialog produces valid objects | `bins/oxikube/tests/kind_rbac.rs` (`BindingDraft` saved through `binding::Create`), `oxikube_app/tests/kind_smoke/rbac/matrix.rs` (the aggregated `view` then allowed by the account's own review), `oxikube_app/tests/kind_smoke/bindings.rs` (E17-S06) |
| (4) the ServiceAccount kubeconfig works with kubectl | `oxikube_app/tests/kind_smoke/rbac/token.rs` (`kubectl --kubeconfig` as the account; the expiry the server reports), `oxikube_app/tests/kind_smoke/sa_kubeconfig.rs` (E17-S09), `token_request` |
| (5) the accessible-namespaces fallback when listing namespaces is forbidden | `oxikube_app/tests/kind_smoke/rbac/gating.rs` (`NamespaceService::catalog`: `Forbidden`, only `home` offered) |
