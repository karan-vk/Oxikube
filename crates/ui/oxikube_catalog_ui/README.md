# oxikube_catalog_ui

**Layer:** `ui`

Cluster catalog home, hotbar, kubeconfig sources management, cloud discovery UI, connect lifecycle, namespace selector.

## Modules

- `namespaces` (E06-S07): the namespace selector dropdown (All, multi-select, favourites, search,
  `0`-`9` keys). See the module docs in `src/namespaces/mod.rs`.

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
