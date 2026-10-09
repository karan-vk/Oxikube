# oxikube_theme

**Layer:** `platform`

Themes (E05-S08): Zed theme-family JSON (schema v0.2.0) importer into `ThemeTokens`, the
`oxikube` block of Kubernetes status colours, `ThemeRegistry` (bundled One Dark / One Light plus
the user's hot-reloaded `themes/` directory), system-appearance following and the `theme`
setting. Only Zed's file *format* is shared; its GPL `theme` crate is not copied.

## Using it

```rust
// bin init order: settings first (the `theme` setting), then themes, then the UI bridge.
oxikube_settings::init(cx);
oxikube_theme::init(cx);                 // registry + active theme + `<config>/themes/` watcher
oxikube_ui::init(cx);
let _follow = oxikube_ui::follow_active_theme(cx);

// When the main window opens, so the OS light/dark flip reaches the theme:
let _appearance = oxikube_theme::SystemAppearance::follow(window, cx);

let theme = oxikube_theme::ActiveTheme::get(cx);     // Arc<ThemeTokens>
let running = theme.oxikube.status_running;          // k8s status colour
let names = oxikube_theme::ThemeRegistry::global(cx).names();
cx.observe_global::<oxikube_theme::ActiveTheme>(|cx| { /* theme changed */ }).detach();
```

- **`theme` setting** (`default.json`): `"Ayu Dark"`, or `{ "mode": "system" | "light" | "dark",
  "light": "One Light", "dark": "One Dark" }`. A name that is not (yet) installed falls back to
  One Light / One Dark.
- **User themes**: any `*.json` Zed theme-family file in `<config>/themes/` (the config dir of
  `oxikube_settings`). A watcher thread rescans on change, off the UI thread; the registry swaps
  in the finished scan, so deleted files disappear. A user theme with a bundled theme's name wins.
- **`oxikube` block**: optional `"oxikube": { "status.running": "#..", "status.pending", "status.failed",
  "status.succeeded", "status.terminating", "status.unknown", "cluster.tab.1" .. "cluster.tab.8",
  "log.source.1" .. "log.source.10" }`
  next to a theme's `style`. Anything unset is derived from the theme's own status colours and
  player colours.
- **Extending the mapping**: one line in `src/import/table.rs` (and a field in `src/tokens/colors.rs`
  when there is no slot yet). Unmapped keys are ignored with a debug log and listed in the
  `ImportReport`; invalid values are reported there too and keep the fallback.
- **Tests** never start the watcher thread (`init_with_dir`); `tests/watch_gpui.rs` is the one test
  that does, with `cx.executor().allow_parking()`. Fixtures (`tests/fixtures/`) are Zed's Ayu and
  Gruvbox, test input only (licences in `tests/fixtures/LICENSES.md`).
- **Glyph warm-up** (`glyph_warm`, E05-P602): the app is built on `glyph_warm::wrap_platform(..)`
  and calls `glyph_warm::install(warmer, cx)` first thing in `run`. The platform's text system is
  decorated: a worker thread rasterises the glyphs already drawn at the dilation levels a switch to
  any installed theme draws them at (macOS picks the level from the text colour's luminance), so
  the frame that shows a new theme only uploads them. Same platform calls, same bitmaps; between
  frames only; at most 8 MiB kept. `cargo run --release -p oxikube_theme --example glyph_warm`
  prints a switch frame's rasterisation with and without it on CoreText.
- **Cost**: `cargo run --release -p oxikube_theme --example theme_load` (cold init is about
  0.5 ms; the startup budget for settings + keymap + theme together is 30 ms).

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_settings`
- `oxikube_assets`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

Third-party theme content is listed in `THIRD_PARTY_NOTICES.md`.
