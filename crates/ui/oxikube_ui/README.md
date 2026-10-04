# oxikube_ui

**Layer:** `ui`

Thin wrapper over gpui-component: tokens, curated components (Table, DockArea, Dialog, Menu, Input, Tabs, Sidebar, Charts, Markdown, Editor glue), icons, zoom-safe sizes. The ONLY crate allowed to import gpui_component.

## Using it

```rust
// bin, before opening a window
let app = gpui_platform::application().with_assets(oxikube_ui::Assets);
app.run(|cx| {
    oxikube_ui::init(cx);                       // gpui-component + tokens + theme bridge
    cx.open_window(options, |window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        oxikube_ui::root::new_root(workspace, window, cx) // Root owns dialog/sheet/toast layers
    });
});
```

- **Tokens**: `cx.tokens()` / `cx.colors()` (`ActiveTokens`). Swap the source with
  `set_token_source(cx, &my_theme, appearance)` once `oxikube_theme` exists.
- **Zoom**: wrap every literal pixel size in `u(px(..))`; persist dock and panel sizes as
  `Unscaled`. `set_ui_scale(cx, UiScale::new(1.25))` changes the zoom. Table column widths are the
  exception: give `TableColumn` design-time widths and the table applies (and re-applies) the zoom
  itself; user-resized widths are reported and kept unscaled.
- **Icons**: `Icon::new(IconName::Box).size(u(px(14.)))`. Add an icon by dropping the Lucide SVG
  into `oxikube_assets/assets/icons/` and one line in its `icons.rs`.
- **Tables**: implement `TableDelegate`, create a `TableHandle`, render `Table::new(&handle)`.
  Virtualised, uniform row height; no gpui-component types in the trait.
- **Overlays**: `window.open_dialog(cx, |dialog, _, _| ..)` (`dialog::OverlayExt`). gpui-component
  0.7's `Root` renders the dialog, sheet, notification and tooltip layers itself, so there is no
  separate layer helper to call.
- Everything else (`dock`, `dialog`, `menu`, `input`, `tabs`, `sidebar`, `chart`, `markdown`,
  `button`, `layout`) is a curated re-export under our names; there is no `pub use gpui_component::*`.

Tests: `cargo test -p oxikube_ui` (unit + `#[gpui::test]` renders of every component, 10 000-row
virtualisation). Token sampler screenshot (needs a GPU; nightly CI):
`cargo test -p oxikube_ui --features screenshot --test screenshot`. Micro benchmark:
`cargo run -p oxikube_ui --profile release-fast --example table_bench`.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_theme`
- `oxikube_assets`
- `oxikube_settings`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
