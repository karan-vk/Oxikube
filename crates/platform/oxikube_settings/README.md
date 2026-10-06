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

   The schema is printed by the app binary (`oxikube --print-settings-schema`, a hidden flag), which
   links every crate that registers settings. `gen-settings-schema` (and `--check`) also asks it
   which crates registered settings (`--print-settings-crates`) and fails when a crate whose source
   calls `register_settings!` is not among them, so a crate nobody links cannot silently drop out of
   the schema. Make sure `bins/oxikube` depends on your crate and references it (its `init`).

Read with `MySettings::get_global(cx)` or `MySettings::get(Some(SettingsLocation { cluster }), cx)`;
react with `MySettings::observe(cx, ..)` / `observe_in` (fires only when the value changes);
write with `update_user_settings::<MySettings>(cx, None, |content| ..)`. No secrets in settings:
credentials belong in the keychain.

## Per-cluster settings (E06-S08)

`clusters.<id>` is the override layer; every registered setting may appear in it, merged field by
field over the user's top-level values and the defaults. `ClusterSettings` (module `cluster`)
is the first setting that is about clusters: `display_name`, `colour`, `read_only`,
`default_namespace`, `terminal_cwd`, `node_shell_image`, `node_shell_pull_secret`, `prometheus`
(`provider`, `path`, `url`, `auth_secret`), `accessible_namespaces` and `exec_interactivity`.

```jsonc
"clusters": {
  "3f2a9c1b7d4e8a60": {            // the cluster id (16 hex characters)
    "display_name": "Production (eu-west)",
    "colour": "#e5484d",
    "read_only": true
  }
}
```

- Read: `ClusterSettings::resolve(&id, cx)` (one hash lookup). React: `ClusterSettings::observe_cluster`
  fires only when that cluster's resolved value changes. Hand to the app layer:
  `ClusterSettings::table(cx)` (see `bins/oxikube::cluster_prefs`).
- Write (in-app toggles): `ClusterSettings::update_cluster` / `set_read_only` rewrite one value and
  keep the user's comments; a new block gets `display_name` from the hint so the id is recognisable.
- A type error in a cluster's block (a bad colour, a URL with credentials) keeps that cluster's last
  good block and is reported as a diagnostic naming the field; unknown keys are reported with their
  path. A block that is wrong on the very first load falls back to the top-level values.
- `read_only`, `colour` and `display_name` reach open sessions at once; `exec_interactivity` is read
  when a session connects; `default_namespace` decides where a new session starts.
- No secrets: `prometheus.url` refuses credentials, queries and fragments, and the bearer token is a
  keychain entry named by `auth_secret`.

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

- `kubeconfig` (E06-S05): the `kubeconfig.sources` setting, the list of kubeconfig files and folders
  the catalog reads (`{ kind: default|file|dir, path }`, default `[{ "kind": "default" }]`, `~` expanded,
  arrays replace rather than merge). Edited by the sources screen through `update_user_settings`.
