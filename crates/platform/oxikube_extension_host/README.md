# oxikube_extension_host

**Layer:** `platform`

wasmtime component-model host: manifest parsing, capability grants, epoch interruption, compile cache, install from path/git, registry wiring.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_settings`
- `oxikube_runtime`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
