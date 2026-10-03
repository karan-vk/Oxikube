# GPUI and gpui-component gotchas

Learned from Zed, gpui-kit, Kubyl and Periscope. Each item cost someone a day.

## Dependencies
- GPUI comes from the weekly `gpui-pre-*` snapshot crates (renamed Zed crates; `use gpui::*`
  still works). gpui-component is compiled against exactly one snapshot. Pin all of
  `gpui-pre`, `gpui-pre-platform`, `gpui-pre-macros`, `gpui-component`, `gpui-base`,
  `gpui-kit-assets` with `=` and bump them together in a dedicated PR.
  `cargo xtask check-gpui-pin` enforces alignment and prints the Zed commit of the snapshot.
- Never add `git = "https://github.com/zed-industries/zed"`: it would create a second
  `gpui` crate whose types do not unify with gpui-component's.
- `gpui_platform` features: `font-kit` is required on macOS (no text otherwise),
  `x11`+`wayland` on Linux, `runtime_shaders` so macOS builds do not need Xcode's Metal
  toolchain.

## Runtime and tasks
- GPUI executors are not tokio. Kubernetes work runs on tokio via
  `oxikube_runtime::spawn_kube(cx, fut)`, which aborts the tokio task when the returned
  GPUI `Task` drops. Never block the foreground executor.
- Dropping a `Task` cancels it. Storing a task in `self.task` and clearing it from inside
  the task cancels the task mid-flight: use a "done" flag plus `.detach()`.
- Streams (watches, logs, terminal bytes) must batch and call `cx.notify()` at most once
  per frame (`oxikube_runtime::notify_coalesced`).

## gpui-component
- `Root` owns the dialog/sheet/notification overlay layers. Only `oxikube_ui` renders
  `Root`; feature views never do.
- Resizable panels and dock sizes are absolute pixels. Wrap sizes with `oxikube_ui::u(px)`
  so UI zoom (rem scale) works, and store dock sizes unscaled.
- Root resets rem size each frame from the theme font size; the workspace re-applies zoom.
- `Table` is built on `uniform_list`: rows must have uniform height. Variable-height
  content goes in the detail drawer, not the row.
- Editor (`EditorState`) lacks inline widgets and soft-wrap docs: draw overlays with a
  `canvas` using `EditorState::range_to_bounds`; keep the text buffer as the source of
  truth and never re-serialise YAML the user is editing.
- Theme: our `ThemeTokens` are converted to gpui-component's `ThemeConfig` in
  `oxikube_ui::theme_bridge`. Do not read gpui-component theme fields in views.

## Tests
- `render_to_image` needs `gpui/test-support` AND `gpui_platform/test-support`.
- Background OS threads waking tasks make tests non-deterministic (Linux CI only).
- See `testing.md`.

## Platform
- Linux: request client-side decorations; Wayland needs a `.desktop` file with the app id
  `dev.karan.oxikube`; CI needs the wayland/xkbcommon/x11-xcb/fontconfig/freetype/vulkan/
  alsa dev packages.
- macOS: `cargo run` unbundled sets the Dock icon at runtime; notifications need a bundle.
- Windows is a later epic; nobody has run this stack interactively there yet. Do not
  assume it works.

## Accessibility
- gpui-component sets AccessKit roles on its components. Custom elements (terminal,
  editor overlays, charts) must set roles/names themselves. Screen-reader support is a
  known gap; keep keyboard navigation complete as the baseline.
