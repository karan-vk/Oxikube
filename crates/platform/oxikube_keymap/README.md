# oxikube_keymap

**Layer:** `platform`

keymap.json layering (per-OS defaults, user, vim), key contexts, action registry, validator (vendored Zed design).

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_settings`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
