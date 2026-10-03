# Sample extensions

Sample Oxikube extensions (WASM components built against `oxikube_extension_api`) land in E23-S09:

- `theme-sample/` — a theme-only extension (no code)
- `hello-command/` — contributes a palette command
- `mcp-echo/` — contributes an MCP context server

Extensions cannot add UI; they contribute themes, icon themes, commands and MCP servers (see docs/adr/0007-plugins-wasm.md).
