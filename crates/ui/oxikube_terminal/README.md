# oxikube_terminal

**Layer:** `ui`

alacritty_terminal grid + custom GPUI Element + TerminalBackend (local PTY, kube exec/attach, display-only).

## Status

- `backend::local` (E09-S02): `LocalPty`, the user's shell on a PTY with the cluster environment
  (`KUBECONFIG`, `KUBE_CONTEXT`, `OXIKUBE_NAMESPACE`). Bench: `cargo run --release -p oxikube_terminal --example local_pty_bench`.
- `element` (E09-S05): `TerminalElement`, the custom GPUI element painting a `TerminalState`
  (theme terminal colours, attributes, wide glyphs, cursor styles, selection, OSC 8 / URL / path
  links with cmd/ctrl-click). `open_link`: the `terminal::OpenLink` handler.
  Bench: `cargo run --profile release-fast -p oxikube_terminal --example element_bench`; a live
  window: `cargo run -p oxikube_terminal --example terminal_preview [-- <program> [args...]]`;
  screenshots: `cargo test -p oxikube_terminal --features screenshot --test screenshot`.
- Keyboard and view: E09-S06, E09-S07.

## Modules

- `grid` (E09-S04): `TermGrid` wraps `alacritty_terminal` (pinned `=0.26.0`; the only module that
  names its types): parse, snapshot, selection, search, scrollback, resize.
- `state` (E09-S04): `TerminalState`, the GPUI entity bridging a `TerminalBackend` and the grid
  (tokio pump and writer, frame-coalesced notify).
- `settings`: the `terminal` block of `settings.json` (`shell`, `shell_args`, `scrollback_lines`).

Throughput benchmark (non-gating): `cargo run --release -p oxikube_terminal --example grid_bench`.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_workspace`
- `oxikube_runtime`
- `oxikube_theme`
- `oxikube_settings`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
