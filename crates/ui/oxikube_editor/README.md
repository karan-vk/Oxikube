# oxikube_editor

**Layer:** `ui`

Manifest editor: YAML/JSON on gpui-component editor, spanned YAML (granit-parser), OpenAPI schema validation, hover/completion, diff vs live, dry-run + SSA apply, templates.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_workspace`
- platform crates: `oxikube_keymap` (key contexts), `oxikube_runtime` (`spawn_kube` for schema fetches)

## Modules

| Module | Story | What |
|---|---|---|
| `yaml` | E10-S02 | the spanned YAML model (granit-parser) |
| `validate` | E10-S03 | schema validation into diagnostics |
| `view` | E10-S04 | `ManifestEditor` (workspace item over `oxikube_ui::editor::CodeEditor`), debounced background validation against the cluster's `SchemaPort`, `editor::NewManifest` / `ToggleReadOnly` / `ToggleSoftWrap` |

Tests: `cargo test -p oxikube_editor` (unit, the view logic against a fake `EditorApi`, `#[gpui::test]`s in
`tests/view.rs`); screenshot: `cargo test -p oxikube_editor --features screenshot --test screenshot`;
typing bench: `cargo run -p oxikube_editor --profile release-fast --example typing_bench`.

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
