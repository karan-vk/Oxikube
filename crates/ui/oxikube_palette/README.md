# oxikube_palette

**Layer:** `ui`

Command palette, ':' jump bar, pickers (vendored Zed PickerDelegate design), help overlay.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_workspace`
- `oxikube_keymap`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Modules

- `jump` (E11-S05): the `:` jump bar. `JumpHost` per window, `JumpBar` (a modal over a `Picker`, key context
  `JumpBar`), `JumpDelegate` (completions of the word under the caret; Enter plans and runs the line), `JumpSources` /
  `LiveEnv` (what a line is resolved against), the wait for a context that was not connected. Its grammar, parser,
  planner, history and completion are `oxikube_app::search::jump`. `:` in a resource table opens it; `[`, `]`, `-`
  replay the history. Actions `palette::OpenJump`, `jump::Back`, `jump::Forward`, `jump::Last`, `jump_bar::Complete`
  (Tab); bus commands of the same names.
- `picker` (E11-S02): `Picker<D: PickerDelegate>`, a fuzzy query field over a virtualised list
  of matches with keyboard selection, confirm / secondary confirm and dismissal, presented
  through the workspace's modal layer; `picker::fuzzy` (nucleo string matching and match
  highlighting for delegates). Derived from Zed's `crates/picker` (GPL-3.0-or-later; headers in
  the files, entry in `THIRD_PARTY_NOTICES.md`). Keys: `picker::*` actions in the `Picker` and
  `Picker > Input` contexts of the per-OS keymaps. First delegate: the container chooser of a pod
  shell (`oxikube_resources_ui::exec::ContainerPickerDelegate`).
- `help` (E11-S10): the `?` overlay (`HelpOverlay`, a `Picker` over `HelpDelegate`): the active key
  context's bindings grouped by command category, searchable, user (`User`) and vim (`Base`)
  bindings marked, unbound defaults listed. `help::Show` is a command; `HelpHost` is one per window,
  mounted by `bins/oxikube`. `?` opens it in a cluster tab where no text field has the focus, and
  closes it while its search field is empty; Escape always closes. Benches:
  `cargo run -p oxikube_palette --features test-support --profile release-fast --example help_bench`.

- `command_palette` (E11-S03): the command palette, `cmd-shift-p` / `ctrl-shift-p` (`palette::Toggle`): `CommandPalette`
  (a modal with the `Palette` key context) over a `Picker<CommandPaletteDelegate>`, one `PaletteHost` per window
  (installed by `bins/oxikube`'s mount, also reached by the bus commands `palette::Toggle` and
  `palette::ToggleShowAll`). Lists `CommandBus` commands for the focused view, selection and session, with
  binding hints and recents first; "Show all" (`cmd-shift-a` / `ctrl-shift-a`) adds the unavailable ones, marked;
  confirm runs the command through the bus path. Benchmark:
  `cargo run -p oxikube_palette --features test-support --profile release-fast --example command_palette_bench`;
  scenario: `cargo xtask perf palette`.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
