# oxikube_editor

**Layer:** `ui`

Manifest editor: YAML/JSON on gpui-component editor, spanned YAML (granit-parser), OpenAPI schema validation, hover/completion, diff vs live, dry-run + SSA apply, templates.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_workspace`
- platform crates: `oxikube_keymap` (key contexts), `oxikube_runtime` (`spawn_kube` for schema fetches), `oxikube_settings` (the `editor` settings)

## Modules

| Module | Story | What |
|---|---|---|
| `yaml` | E10-S02 | the spanned YAML model (granit-parser) |
| `validate` | E10-S03 | schema validation into diagnostics |
| `settings` | E10-S12 | the `editor` settings (`tab_size`, `font_family`, `font_size`, `soft_wrap`, `show_managed_fields`, `mask_secrets`, `validation`, `field_manager`), hot reload into open editors, the notice for a rejected value |
| `open` | E10-S06 | an editor opened from a live object: `render_resource` (kubectl key order, managedFields hidden, Secret values masked, `status` dimmed), the `resource::Edit` / `resource::ViewYamlInEditor` row actions, `editor::ToggleManagedFields` / `ToggleSecretMask` |
| `view` | E10-S04 | `ManifestEditor` (workspace item over `oxikube_ui::editor::CodeEditor`), debounced background validation against the cluster's `SchemaPort`, `editor::NewManifest` / `ToggleReadOnly` / `ToggleSoftWrap`; E10-S10: the language chip, `editor::ConvertToJson` / `ConvertToYaml` / `UseJson` / `UseYaml`, the notice bar |
| `json` | E10-S10 | JSON mode (pure, no gpui): `Language` detection, the strict JSON check, YAML <-> JSON `convert` with `Warnings`, the two writers |

Tests: `cargo test -p oxikube_editor` (unit, the view logic against a fake `EditorApi`, `#[gpui::test]`s in
`tests/view.rs`, `tests/open.rs`); screenshot: `cargo test -p oxikube_editor --features screenshot --test screenshot`;
typing bench: `cargo run -p oxikube_editor --profile release-fast --example typing_bench`; conversion bench:
`cargo bench -p oxikube_editor --bench convert`; JSON mode: `cargo test -p oxikube_editor --test json_mode --test json_props
--test json_view` (properties: `PROPTEST_CASES=20000` for a longer run).

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
