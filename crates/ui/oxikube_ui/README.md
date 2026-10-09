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

- **Tokens**: `cx.tokens()` / `cx.colors()` (`ActiveTokens`). Themes: after `oxikube_ui::init` and
  `oxikube_theme::init`, call `let _theme = oxikube_ui::follow_active_theme(cx);` (keep the
  subscription): it applies `oxikube_theme`'s active theme now and on every change (settings edit,
  light/dark flip, hot-reloaded file) through `set_theme`, which maps `ThemeTokens` onto
  gpui-component's `ThemeConfig` (`theme_config`) and our `Tokens`. `set_tokens` /
  `set_token_source` still set plain tokens (no theme).
- **Code view** (`oxikube_ui::code_view`): `cx.new(|cx| CodeView::new(Look::YAML | Look::TEXT, cx))`,
  `set_text(Arc<str>, cx)`: a read-only, virtualised text view of any size (the detail's YAML and
  Describe tabs). Rows are laid out and the YAML parsed (tree-sitter) on the background executor;
  a frame shapes only the visible rows. Selection, copy and key scrolling in the `CodeView` key
  context. `oxikube_ui::editor` keeps the glue for gpui-component's full editor (E10), which wraps
  every line on the UI thread when given its text: not for large read-only documents.
- **Zoom**: wrap every literal pixel size in `u(px(..))`; persist dock and panel sizes as
  `Unscaled`. `set_ui_scale(cx, UiScale::new(1.25))` changes the zoom. Table column widths are the
  exception: give `TableColumn` design-time widths and the table applies (and re-applies) the zoom
  itself; user-resized widths are reported and kept unscaled.
- **Icons**: `Icon::new(IconName::Box).size(u(px(14.)))`. Add an icon by dropping the Lucide SVG
  into `oxikube_assets/assets/icons/` and one line in its `icons.rs`.
- **Tables**: implement `TableDelegate`, create a `TableHandle`, render `Table::new(&handle)`.
  Virtualised, uniform row height; no gpui-component types in the trait. Plain text cells: return
  a `TextCell` from `text_cell` and the table draws it itself (one element less, the ellipsis only
  when the text does not fit its column; E07-S09); `render_td` is the fallback.
- **Overlays**: `window.open_dialog(cx, |dialog, _, _| ..)` (`dialog::OverlayExt`). gpui-component
  0.7's `Root` renders the dialog, sheet, notification and tooltip layers itself, so there is no
  separate layer helper to call. `dialog::{Cancel, Confirm}` are the library's Escape / Enter
  actions; `oxikube_workspace`'s modal layer (one modal view over the workspace, focus-trapped)
  listens for the same `Cancel`, so components inside a modal consume Escape first.
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
