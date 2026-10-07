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
- Overlays (E05-S10), all reached through the `Workspace` and never built by features:

  ```rust
  workspace.update(cx, |ws, cx| {
      ws.register_status_item(StatusSide::Left, 100, read_only_badge, cx);   // lower = further left
      ws.toggle_modal(window, cx, |window, cx| DialogModal::new("Delete pod?", cx)
          .message("This cannot be undone.").destructive().on_confirm(|_, _| { /* ... */ }));
      ws.show_toast(Toast::error("Could not reach prod").key("connect/prod")
          .action(ToastAction::new("Retry", |_, _| { /* ... */ })), cx);
  });
  ```

  A `ModalView` is a focusable view that emits `DismissEvent` to close itself (and may veto
  Escape / outside click in `on_before_dismiss`). Escape closes the modal and focus returns to
  the element that had it; Tab stays inside. Toasts dedupe by key, show at most three at once
  and never take focus (`ToastLayer::focus_toasts` is the keyboard way in). Animations are
  capped at 150 ms and off under reduce-motion (`oxikube_ui::motion::reduce_motion`). The layers are not
  gpui-component's `Root` dialog/notification layers, which keep serving `OverlayExt`.
- Tabs drag between panes, docks resize, panes and dock groups zoom: the dock area does it.
- `oxikube_workspace::init(cx)` registers the window menu's actions and the `workspace::*` key
  bindings; the menu bar itself is installed at the end of the first main window's first frame
  (E05-S13: about 19 ms with AppKit, not on the path to the first frame).
- `window::open_main_window_restoring(cx, layout_store, wrap)` (E05-S13) opens the main window
  behind the startup placeholder: the default layout, interactive, with "Restoring layout…" in the
  title bar until the saved layout read through the async `StatePort` replaces it (a failed read
  leaves the default layout usable). The app opens its first window this way.
- Session basics (`session`): `window::New` opens another main window (own `Workspace`);
  `view::ZoomIn`/`ZoomOut`/`ZoomReset` change the `ui_scale` setting (cmd/ctrl `+`, `-`, `0`);
  `reduce_motion` resolves the OS preference (fed by `session::set_os_reduce_motion`, which the binary's
  `os_motion` probe calls at start-up and on change; GPUI does not read it) and the setting into GPUI's flag; a feature that starts exec sessions, port-forwards or
  applies calls `session::register_operation_provider(cx, |cx| vec![RunningOperation::new(..)])`
  so `app::Quit` asks before stopping them (setting `confirm_quit`).
- Tests: `cargo test -p oxikube_workspace`. Feature `test-support` exports `TestItem` and
  `TestPanel`, `TestStatusItem` and `TestModal` for other crates' `#[gpui::test]`s. Screenshots
  (`--features screenshot --test screenshot`, nightly): `workspace`, `workspace_modal`, `main_window_restoring` (the startup placeholder). Frame cost of a dock resize and a tab switch:
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
