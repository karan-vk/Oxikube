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

- `picker` (E11-S02): `Picker<D: PickerDelegate>`, a fuzzy query field over a virtualised list
  of matches with keyboard selection, confirm / secondary confirm and dismissal, presented
  through the workspace's modal layer; `picker::fuzzy` (nucleo string matching and match
  highlighting for delegates). Derived from Zed's `crates/picker` (GPL-3.0-or-later; headers in
  the files, entry in `THIRD_PARTY_NOTICES.md`). Keys: `picker::*` actions in the `Picker` and
  `Picker > Input` contexts of the per-OS keymaps. First delegate: the container chooser of a pod
  shell (`oxikube_resources_ui::exec::ContainerPickerDelegate`).

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
