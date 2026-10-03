# oxikube_acp

**Layer:** `adapters`

AgentPort adapter on agent-client-protocol (=2.2.0): ACP client, registry fetch, launchers (npx/uvx/binary), presets, fs/terminal virtualisation hooks.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
