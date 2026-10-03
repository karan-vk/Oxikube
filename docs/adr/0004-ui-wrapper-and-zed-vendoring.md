# ADR 0004: gpui-component behind oxikube_ui; Zed code vendored, never depended on

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/

## Context

Building Zed-quality table/dock/editor/palette/dialog components in-house would take months. gpui-component (Apache-2.0, 15k stars) provides them on the same gpui-pre types but has frequent breaking releases. Zed's GPL crates are now licence-compatible but are tightly coupled to Zed's project model.

## Decision

All gpui-component usage lives in `oxikube_ui` (tokens, curated components, TableDelegate/Editor glue, `u(px)` zoom-safe sizes, Root layer rendering). Feature crates import only `oxikube_ui`; lint-deps bans gpui-component elsewhere. Zed is used as design reference (per-crate init(cx), AppState global, Item/Panel/Dock/Pane, PickerDelegate, SettingsStore layering, keymap JSON, theme-family JSON v0.2.0, extension manifest/WIT, acp_thread model) and selected modules are copied with GPL headers where copying is cleaner than rewriting.

## Consequences

A gpui-component upgrade or replacement touches one crate. Vendored Zed code must be tracked in THIRD_PARTY_NOTICES.md and periodically compared with upstream.
