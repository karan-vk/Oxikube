# oxikube_assets

**Layer:** `platform`

Embedded assets: Lucide icons, fonts, bundled themes, default settings/keymaps.

## Contents

- `IconName`: the closed enum of Lucide SVGs shipped (`assets/icons/`, ISC licence in
  `assets/icons/LICENSE`). Only listed icons are embedded in the binary.
- `Assets`: a `gpui::AssetSource` serving them at `icons/<name>.svg`. `oxikube_ui::Assets` chains it
  in front of gpui-component's own bundle; the bin registers that one.
- `BUNDLED_THEME_FAMILIES`: Zed theme-family JSON for One Dark and One Light (`assets/themes/`, MIT
  licence text in `assets/themes/LICENSES.md`), parsed by `oxikube_theme`.
- Settings assets: `default_settings()`, `initial_user_settings_content()`, `settings_schema()`.

## Allowed internal dependencies

- (none)

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
