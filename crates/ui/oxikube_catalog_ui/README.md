# oxikube_catalog_ui

**Layer:** `ui`

Cluster catalog home, hotbar, kubeconfig sources management, cloud discovery UI, connect lifecycle, namespace selector.

## Modules

- `sources` (E06-S05): the kubeconfig sources screen, `SourcesView`: the list of files and folders the
  catalog reads with each one's status (errors inline), add file / folder through the platform picker,
  paste (stored `0600`, ADR 0015), remove and reload, as `kubeconfig::*` commands; `SettingsSourceList`
  and `follow` keep it in step with `kubeconfig.sources` in `settings.json`. See `src/sources/mod.rs`.
- `namespaces` (E06-S07): the namespace selector dropdown (All, multi-select, favourites, search,
  `0`-`9` keys). See the module docs in `src/namespaces/mod.rs`.

- `connect` (E06-S06): the connect lifecycle of a cluster tab (`ConnectView`, `DegradedBanner`,
  the pure `ConnectViewModel`). See the module docs in `src/connect/mod.rs`.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_workspace`
- `oxikube_palette`

It also uses the platform crates `oxikube_keymap` and `oxikube_runtime` (ui may depend on platform).

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Modules

- `catalog` (E06-S03): the catalog home, `CatalogView`. See the module docs for the data flow,
  `catalog::test_support` (feature `test-support`) for `RecordingDispatcher` and synthetic entries,
  and `examples/catalog_bench.rs` for the first-paint and filter numbers.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
