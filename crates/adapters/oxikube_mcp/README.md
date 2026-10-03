# oxikube_mcp

**Layer:** `adapters`

MCP server (rmcp) exposing the ToolRegistry to hosted agents over stdio and loopback streamable-HTTP.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
