# Vendoring code from Zed and others

Oxikube is GPL-3.0-or-later, the same licence as Zed's application crates. That makes
copying Zed source legal; it does not make it free of obligations, and it does not make
Zed's crates good dependencies.

## Never depend, sometimes copy

- Never add Zed crates as git dependencies. `ui`, `theme`, `menu` are light (~18 internal
  crates) but conflict with gpui-component's `gpui-pre` pin; `settings` drags Zed's fs/git
  stack (36 crates); `workspace`, `picker`, `command_palette`, `editor`, `terminal_view`,
  `acp_thread`, `agent_ui`, `extension_host`, `settings_ui` drag 90-154 crates including
  `project`, `client`, `lsp`, `rpc`, `dap`.
- Copy **design** by default: trait shapes (Item/Panel/Dock/Pane, PickerDelegate,
  Settings/SettingsStore, keymap sections, extension manifest + WIT `since_vX` layout,
  acp_thread entry model), init order, registries.
- Copy **code** only when the logic is subtle and self-contained and rewriting would just
  reintroduce bugs. Approved candidates: `settings_json::update_value_in_json_text`
  (comment-preserving JSON edits) and its tests; keymap dispatch tests; theme JSON schema
  structs and `refine_theme` fallback logic; terminal keystroke-to-escape mapping tables;
  parts of `terminal_element` cell painting; picker keyboard handling.

## Required header for copied Zed code

Put this at the top of every file containing copied code, above the module doc:

```rust
// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/<crate>/src/<file>.rs @ zed <tag-or-sha>
```

Then add an entry to `THIRD_PARTY_NOTICES.md` under "Zed" listing the file and the
upstream path/revision. Keep the copied region recognisable (do not reformat it beyond
rustfmt) so future diffs against upstream are possible.

## Other sources

| Source | Licence | Header rule |
|---|---|---|
| kdash (`kdash-rs/kdash`) | MIT | keep the MIT copyright + permission text in the file header; list in notices |
| kubectl-view-allocations (`qty::Qty`) | CC0-1.0 | attribution optional; we still cite the source in a comment |
| deskribe | Apache-2.0 + Kubernetes NOTICE | preserve its NOTICE text if any code is vendored (we normally depend on the crate) |
| sofka / kubetui (Table API request shape) | MIT / MIT OR Apache-2.0 | treat as reference; if copied, MIT header |
| gpui-component | Apache-2.0 | dependency; if a component is forked into `oxikube_ui`, keep its Apache header |
| Zed bundled themes (One, Ayu, Gruvbox JSON) | GPL-3.0-or-later (Zed assets) | notices entry; themes from the extensions registry carry their own licences: check before bundling |

What not to copy from anywhere: kdash's polling architecture and pod-status derivation
(known inverted init-container branch), kdash's quantity parsing (incorrect), Lens's
injected edit annotations, any proprietary ACP adapter binaries (fetch at runtime only).

## Review rule

A PR that adds non-trivial code without a story-level mention of its provenance is sent
back. "Written from scratch after reading X" is fine and needs no header; say so.
