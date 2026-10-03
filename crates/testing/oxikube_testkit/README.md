# oxikube_testkit

**Layer:** `testing`

Port fakes, fixtures, builders, kind helpers, gpui test helpers.

## Screenshots

- Feature `screenshot`: `oxikube_testkit::screenshot` saves PNGs and compares an image against a
  golden with a per-channel tolerance and a max differing-pixel ratio (pure image code, no window).
- Feature `gpui-screenshot` (implies `screenshot`): `oxikube_testkit::headless::capture_view` draws a
  view off-screen via `Window::render_to_image` (turns on `gpui/test-support` and
  `gpui_platform/test-support`). Needs a GPU device: Metal on macOS, Vulkan (Mesa lavapipe is fine)
  on Linux. Windows are 2x device pixels.
- Goldens live under `<crate>/tests/goldens/<os>/<name>.png` (`<os>` = `std::env::consts::OS`).
  Create or refresh them with `OXIKUBE_UPDATE_GOLDENS=1 cargo test ...`, review the PNG, commit it.
  A mismatch writes `<name>.actual.png` and `<name>.diff.png` beside the golden (gitignored).
- Run: `cargo test -p oxikube_testkit --features screenshot`.
- The whole app: `OXIKUBE_SCREENSHOT=out.png cargo run -p oxikube --features screenshot`
  (never in default or release builds); smoke tests: `cargo test -p oxikube --features screenshot`.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
