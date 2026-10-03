# oxikube_settings

**Layer:** `platform`

SettingsStore: default.json -> user settings.json (JSONC) -> per-cluster overrides, Settings trait + inventory registration, comment-preserving edits, schemars schema, notify hot reload (vendored Zed design).

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
