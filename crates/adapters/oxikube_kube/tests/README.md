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
uploads each failed test's namespace events (saved by `TestNamespace` before it deletes the
namespace, into `OXIKUBE_TEST_DIAGNOSTICS_DIR`), the cluster-wide events, pods, nodes and the kind
logs. Locally, without that variable, a failed test prints its namespace's events to stderr.

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
| `ExecPort` | `exec`, `attach`, `create_debug_container`, `node_shell` | `exec_terminal` (TTY echo, resize, exit code, attach and kill, debug container, node shell removed on kill, `NotFound`) |
| `PortForwardPort` | `forward` | `portforward` (pod and service GET, named ports, errors, `Forbidden`), `portforward_restart` |
| `MetricsPort` | `node_metrics`, `pod_metrics` | `metrics_kind` (present, absent as `Unavailable(NotInstalled)`, `Forbidden`) |
| `KubeEvents` | `watch` | `events_feed` (merged core and events.k8s.io, per-object feed, capacity) |
| `WarningPort` | `subscribe` | `warnings_kind` (a PodSecurity `Warning:` header from the real API server reaches the port), plus unit tests of the header parser and the layer |
| `FeedSource` (watch budget) | `open` via `FeedRegistry` | `budget_kind`, `budget_tracing` |
| algorithms | `trigger_cronjob`, `rollout_history`, `rollout_undo`, `drain` | `algorithms_workloads`, `algorithms_drain` |

## Epic E04 acceptance items

| Item | Test |
|---|---|
| Table API on a CRD with `additionalPrinterColumns`, and on pods | `table_feed::crd_printer_columns_match_kubectl_get`, `table_feed::pods_match_kubectl_get_pods_o_wide` |
| SSA apply conflict returns `Conflict` naming the field managers | `mutations::server_side_apply_conflicts_name_the_other_manager_and_force_takes_over` |
| Dry run returns the server's would-be object | `mutations::dry_run_returns_the_would_be_object_and_changes_nothing`, `mutations_delete::a_dry_run_delete_leaves_the_object` |
| Logs survive a restart without duplicated or missing lines | `logs_restart::a_container_crash_mid_stream_leaves_no_duplicate_and_no_gap` |
| Exec round-trips stdin/stdout with resize | `exec_streams::a_tty_shell_echoes_and_reports_the_round_trip_latency`, `exec_streams::a_resize_is_observed_by_stty` |
| Port-forward GET | `portforward::forward_to_an_nginx_pod_and_get`, `portforward::forward_through_a_service_maps_the_service_port_to_the_named_target_port` |
| Eviction and drain with a PodDisruptionBudget | `subresources_pods::a_blocking_budget_refuses_the_eviction_as_a_retryable_error`, `algorithms_drain::a_drain_waits_out_a_budget_that_blocks_then_allows_and_evicts_everything` |
| Metrics present, and absence reported (not swallowed) | `metrics_kind::node_metrics_report_usage_and_utilisation_for_every_node`, `metrics_kind::a_cluster_without_the_metrics_group_is_unavailable_not_an_error` |
| 403 for an RBAC-restricted service account | `rbac`, `resources_special::a_restricted_account_gets_forbidden_not_a_crash`, `resources_port::list_metadata_without_rights_is_forbidden`, `metrics_kind::an_account_without_rights_on_metrics_gets_forbidden` |
