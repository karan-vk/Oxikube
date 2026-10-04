# oxikube_assets

**Layer:** `platform`

Embedded assets: Lucide icons, fonts, bundled themes, default settings/keymaps.

## Contents

- `IconName`: the closed enum of Lucide SVGs shipped (`assets/icons/`, ISC licence in
  `assets/icons/LICENSE`). Only listed icons are embedded in the binary.
- `Assets`: a `gpui::AssetSource` serving them at `icons/<name>.svg`. `oxikube_ui::Assets` chains it
  in front of gpui-component's own bundle; the bin registers that one.

## Allowed internal dependencies

- (none)

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
