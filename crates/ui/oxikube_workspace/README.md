# oxikube_workspace

**Layer:** `ui`

Window shell: Item/Panel/Pane/Dock model, tabs, status bar, modal/toast layers, layout persistence, cluster tabs, sidebar, notifications panel.

## Using it

A feature crate implements `Item` (a centre tab) or `Panel` (a dockable side view) and never
touches the dock area:

```rust
impl Item for PodList {
    fn tab_content(&self, cx: &App) -> TabContent {
        TabContent::new("Pods").icon(IconName::Box)
    }
    fn item_key(&self, _: &App) -> Option<SharedString> { Some("pods/default".into()) }
}

workspace.update(cx, |ws, cx| {
    ws.open_item(pod_list, window, cx);          // activates an open "pods/default" instead
    ws.add_panel(logs_panel, window, cx);        // into the dock `Panel::position` names
    ws.toggle_panel::<LogsPanel>(window, cx);    // show + focus / hide (Zed's toggle focus)
    ws.split_active_pane(SplitDirection::Right, window, cx);
});
```

- `active_pane`, `pane_group`, `dock(position)` return snapshots (`Pane`, `PaneGroup`, `Dock`).
- `close_item` honours `Item::can_close`; serialisable items (`Item::serialized_kind` +
  `Item::serialize`, a builder registered with `register_item`) go on a bounded reopen-closed
  stack (`reopen_closed_item`).
- Tabs drag between panes, docks resize, panes and dock groups zoom: the dock area does it.
- `oxikube_workspace::init(cx)` registers the window menu and the `workspace::*` key bindings.
- Tests: `cargo test -p oxikube_workspace`. Feature `test-support` exports `TestItem` and
  `TestPanel` for other crates' `#[gpui::test]`s. Frame cost of a dock resize and a tab switch:
  `cargo run -p oxikube_workspace --features test-support --profile release-fast --example workspace_bench`.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_runtime`
- `oxikube_settings`
- `oxikube_keymap`
- `oxikube_theme`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
