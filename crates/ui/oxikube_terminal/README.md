# oxikube_terminal

**Layer:** `ui`

alacritty_terminal grid + custom GPUI Element + TerminalBackend (local PTY, kube exec/attach, display-only).

## Status

- `backend::local` (E09-S02): `LocalPty`, the user's shell on a PTY with the cluster environment
  (`KUBECONFIG`, `KUBE_CONTEXT`, `OXIKUBE_NAMESPACE`). Bench: `cargo run --release -p oxikube_terminal --example local_pty_bench`.
- Settings: `terminal.shell`, `terminal.shell_args`.
- Grid, element and view: E09-S04..S07.

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
