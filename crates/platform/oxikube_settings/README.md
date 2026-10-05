# oxikube_settings

**Layer:** `platform`

Typed, layered, hot-reloading settings (Zed's settings design, E05-S06):
`default.json` (embedded, from `oxikube_assets`) → user `settings.json` (JSONC) →
`clusters.<id>` overrides; `Settings` trait + `inventory` registration; comment-preserving
`update_user_settings`; schemars `settings.schema.json`; `notify` hot reload.

## Adding a setting

1. In the owning crate, a content struct with `Option` fields deriving `Serialize`,
   `Deserialize`, `JsonSchema`, `Default` (use
   `#[serde(default, skip_serializing_if = "Option::is_none")]`).
2. A runtime struct (`PartialEq`) implementing `Settings` with `KEY` (or `None` for root-level
   fields), `type Content` and `from_content`.
3. `oxikube_settings::register_settings!(MySettings);` and call the crate's `init(cx)` from the
   binary so the crate is linked.
4. Defaults in `crates/platform/oxikube_assets/assets/settings/default.json`, then
   `cargo xtask gen-settings-schema` (CI runs it with `--check`).

   The schema is printed by the `settings_schema` example, which links only this crate, so a
   setting registered in another crate would be silently absent from it. `gen-settings-schema`
   (and `--check`) therefore fails when a crate that calls `register_settings!` is not linked
   into the generator. Until the generator moves to a target linking every settings crate
   (E05-S06b, issue #454), that story must land first.

Read with `MySettings::get_global(cx)` or `MySettings::get(Some(SettingsLocation { cluster }), cx)`;
react with `MySettings::observe(cx, ..)` / `observe_in` (fires only when the value changes);
write with `update_user_settings::<MySettings>(cx, None, |content| ..)`. No secrets in settings:
credentials belong in the keychain.

## Config dir

`$OXIKUBE_CONFIG_DIR`, else `$XDG_CONFIG_HOME/oxikube` or `~/.config/oxikube` (macOS, Linux),
`%APPDATA%\Oxikube` (Windows). `settings.json` is created from a commented template on first run.
Tests use `init_with_dir` (no watcher thread).

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- platform crates (`oxikube_assets`)

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

Vendored Zed code (GPL-3.0-or-later) is listed in `THIRD_PARTY_NOTICES.md`.
