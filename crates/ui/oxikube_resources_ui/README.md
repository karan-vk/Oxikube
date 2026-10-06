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

- `table` (E07-S03): `ResourceTable`, the generic virtualised table of one kind (store
  subscription, sort through the store, column layout per kind, multi-select, keyboard, context
  menu, status colours from the theme's `oxikube` block).
- `table::states` (E07-S10): `TableState` (the pure derivation), the state views, the stale badge, Retry and the API server warnings.
- `filter` (E07-S04): `FilterBar`, the `/` filter of a table (`foo`, `!foo`, `-l k=v`, `-f fuzzy`):
  parse errors, the `123 of 4,812` count, debounce, `table::FocusFilter`, and the saved filter
  (`resource_table.persist_filter`).
- `views` (E07-S03): `ResourceViews`, which opens tables in cluster tabs from the sidebar and
  `resource::OpenList`, and runs `resource::Open`, `CopyName`, `SelectAll` and `RetryFeed` (E07-S10).
- `detail` (E07-S05): `DetailView`, the generic detail of one object (header, labels and
  annotations with copy, owner links, finalizers, conditions, `status` summary, Events, and the YAML
  and Describe tabs of E07-S06), as the right-dock `DetailDrawer` of a cluster tab or,
  after `resource::PinDetail`, a workspace tab. Secrets show key names, never values.
- `detail::yaml` / `detail::describe` (E07-S06): the YAML tab (read-only tree-sitter highlighted editor from
  `oxikube_ui::editor`, managedFields toggle, secrets masked, copy and save as commands) and the Describe tab
  (the connection's `DescribePort`, spinner, error with Retry, refresh).
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

Bench: `cargo run -p oxikube_resources_ui --profile release-fast --example table_bench`.
Screenshots: `cargo test -p oxikube_resources_ui --features screenshot --test screenshot` (status tones, the detail drawer's Overview, YAML and Describe tabs) and `--test states_screenshot` (the table states, E07-S10).

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
