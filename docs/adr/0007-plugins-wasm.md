# ADR 0007: Zed-identical extension system on wasmtime + WIT, no UI hooks

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/

## Context

Lens extensions run with full Node privileges and are a trust/EDR problem; Zed extensions are sandboxed WASM components contributing themes, languages, slash commands and MCP servers but cannot add UI.

## Decision

Extensions are WASM components built against `oxikube_extension_api` (WIT worlds versioned since_vX), declared by extension.toml with capability grants (process:exec, download_file, kube:read patterns), hosted by wasmtime 49 (component model, epoch interruption, bounded compile cache). They contribute themes, icon themes, commands and MCP servers. They do not add UI. Install from local path or git URL in v1; a curated registry is a backlog epic.

## Consequences

Plugin authors write Rust (or any WIT-capable language). wasmtime adds build time and MSRV 1.96 (isolated to one crate). Lens-style custom views are explicitly not supported; integrations (ADR 0009) are the path for first-class UI.
