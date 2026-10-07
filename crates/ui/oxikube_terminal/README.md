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
- `mappings` + `input` (E09-S06): keystroke -> escape sequence mapping (`to_esc_str`), bracketed
  paste, mouse reporting, IME composition (`EntityInputHandler` for `TerminalState`), copy / paste
  / copy on select with the multi-line confirmation, and the `terminal::Copy` / `terminal::Paste`
  commands. Cost of the mapping: `cargo run --release -p oxikube_terminal --example input_bench`.
- `view` (E09-S07): `TerminalView`, the terminal as a workspace item (tab title from the process,
  dirty while it runs, dockable, saved as its `BackendDescriptor` only, restored as a fresh
  process), the `TerminalPanel` in a cluster tab's bottom dock, `TerminalViews` and the
  `terminal::New` (`ctrl-~`), `terminal::Split` (`cmd-d` / `ctrl-shift-d` in a terminal) and
  `terminal::Close` (`cmd-w` / `ctrl-shift-w` in a terminal) commands; `` ctrl-` `` toggles the panel.
  Screenshot: `terminal_tabs` in `tests/screenshot.rs`.

## Modules

- `grid` (E09-S04): `TermGrid` wraps `alacritty_terminal` (pinned `=0.26.0`; the only module that
  names its types): parse, snapshot, selection, search, scrollback, resize.
- `state` (E09-S04): `TerminalState`, the GPUI entity bridging a `TerminalBackend` and the grid
  (tokio pump and writer, frame-coalesced notify).
- `settings`: the `terminal` block of `settings.json` (`shell`, `shell_args`, `scrollback_lines`,
  `copy_on_select`, `option_as_meta`, `confirm_multiline_paste`).

## Manual input checks (IME, Linux, Windows)

The automated tests drive the input handler API and a real PTY on Linux and macOS; an input method
is only really exercised by hand. Run `cargo run -p oxikube_terminal --example terminal_preview --
/bin/sh` (or the app once the terminal tab exists) and check:

1. **IME composition.** Switch to a Japanese (or Chinese / Korean) input method, type `nihon`: the
   reading is underlined at the cursor (not at the window corner), the candidate window opens next
   to it, Enter/Space commit, and the committed text appears in the shell as UTF-8. Esc cancels the
   composition and nothing is sent. Repeat with the window moved and resized: the candidate window
   follows the cursor cell.
2. **Dead keys / Option.** On macOS with `option_as_meta` off, Option-e then e types `é`; with it
   on, Option-b / Option-f move by word in the shell. On Linux and Windows (German layout), AltGr
   combinations (`@`, `{`, `\`) type characters, Alt-b / Alt-f move by word.
3. **Arrows and friends.** In `vim` / `less` / `htop`: arrows, Home/End, PgUp/PgDn, F1-F12, Ctrl-arrows;
   `ctrl-c` interrupts `sleep 100`; `ctrl-d` ends `cat`.
4. **Mouse.** `htop` / `vim` (`:set mouse=a`): click, drag and wheel act in the program; Shift-drag selects text.
5. **Clipboard.** Copy a selection with cmd-c (ctrl-shift-c), paste with cmd-v (ctrl-shift-v); a
   two-line paste asks first; with `copy_on_select` on, releasing the mouse copies.
6. **HiDPI.** On a 2x display and with the UI zoom changed, the candidate window and cursor stay aligned.
7. **Wayland and X11 (Linux), Windows.** Repeat 1-3 under both Linux session types and on Windows; note
   IME candidate placement and any key that never arrives (the usual suspects: `ctrl-space`, `alt-tab`).

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
