# ADR 0001: GPL-3.0-or-later for the whole repository

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/

## Context

Zed licenses its app crates GPL-3.0-or-later and only GPUI/extension_api Apache-2.0. Oxikube wants to reuse Zed's designs and, where cleaner than rewriting, Zed's code (settings store, keymap, theme loader, picker, terminal element). Apache/MIT would forbid copying that code. gpui-component (Apache-2.0), kdash (MIT), deskribe (Apache-2.0) are all GPL-compatible.

## Decision

Every crate, including `oxikube_extension_api`, is GPL-3.0-or-later. Vendored Zed code carries the GPL header and an entry in THIRD_PARTY_NOTICES.md. cargo-deny allows GPL-3.0(-or-later), Apache-2.0, MIT, BSD, ISC, MPL-2.0, CC0, Unicode, Zlib, BSL-1.0.

## Consequences

Extensions must be GPL-compatible. Proprietary ACP adapters (claude-acp, antigravity-acp) are fetched at runtime, never bundled. We never git-depend on Zed crates (see ADR 0003/0004) even though the licence would allow it.
