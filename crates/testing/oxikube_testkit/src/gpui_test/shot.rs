//! Screenshot tests: [`ScreenshotApp`] drives a headless window like [`TestApp`](super::TestApp)
//! does and captures it; [`run_golden_cases`] is the `main` of a `harness = false` test.
//!
//! Needs a GPU device (Metal on macOS, Vulkan such as Mesa lavapipe on Linux) and, on macOS, the
//! process main thread for the platform text system, hence `harness = false`. Everything else is
//! as deterministic as `TestApp`: gpui's test scheduler, no wall-clock waits.

use std::{path::Path, process::ExitCode, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, ensure};
use gpui::{
    Action, AnyWindowHandle, App, AssetSource, Entity, HeadlessAppContext, KeyBinding, Keystroke,
    Pixels, Render, Size, Window,
};

use crate::{
    headless::{HEADLESS_SCALE_FACTOR, capture_window, headless_context_with_assets},
    screenshot::{
        RgbaImage, Tolerance, UPDATE_GOLDENS_ENV, check_golden, distinct_colors_at_least,
        golden_path,
    },
};

/// A headless GPUI app with the host's real text system and GPU renderer.
///
/// The verbs match [`TestApp`](super::TestApp) / [`TestWindow`](super::TestWindow); the window is
/// named by the [`AnyWindowHandle`] [`ScreenshotApp::open_window`] returned.
pub struct ScreenshotApp {
    cx: HeadlessAppContext,
}

impl ScreenshotApp {
    /// An app whose views load no assets.
    pub fn new() -> Self {
        Self::with_assets(Arc::new(()))
    }

    /// An app whose views load icons and fonts from `assets` (`oxikube_ui::Assets`).
    pub fn with_assets(assets: Arc<dyn AssetSource>) -> Self {
        Self {
            cx: headless_context_with_assets(assets),
        }
    }

    /// Runs `f` with the [`App`] and lets the tasks it made runnable run.
    pub fn update<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        let result = self.cx.update(f);
        self.cx.run_until_parked();
        result
    }

    /// Registers key bindings.
    pub fn bind_keys(&mut self, bindings: impl IntoIterator<Item = KeyBinding>) {
        self.update(|cx| cx.bind_keys(bindings));
    }

    /// Opens a window of `size` (logical pixels, drawn at [`HEADLESS_SCALE_FACTOR`]x) whose root
    /// is the entity `build_root` returns, and lets it settle.
    pub fn open_window<V: Render + 'static>(
        &mut self,
        size: Size<Pixels>,
        build_root: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Result<AnyWindowHandle> {
        let handle = self
            .cx
            .open_window(size, build_root)
            .context("opening the headless window")?;
        self.cx.run_until_parked();
        Ok(handle.into())
    }

    /// Types a space-separated list of keystrokes into `window` through the bound keymap.
    pub fn simulate_keystrokes(&mut self, window: AnyWindowHandle, keystrokes: &str) -> Result<()> {
        for text in keystrokes.split(' ') {
            let keystroke =
                Keystroke::parse(text).with_context(|| format!("`{text}` is not a keystroke"))?;
            self.cx.update_window(window, |_, window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            })?;
        }
        self.cx.run_until_parked();
        Ok(())
    }

    /// Dispatches `action` to the focused element of `window`.
    pub fn dispatch_action(&mut self, window: AnyWindowHandle, action: impl Action) -> Result<()> {
        self.cx.update_window(window, |_, window, cx| {
            window.dispatch_action(action.boxed_clone(), cx);
        })?;
        self.cx.run_until_parked();
        Ok(())
    }

    /// Runs every runnable task until nothing is left.
    pub fn run_until_parked(&self) {
        self.cx.run_until_parked();
    }

    /// Moves the test clock forward and runs what falls due.
    pub fn advance_clock(&self, duration: Duration) {
        self.cx.advance_clock(duration);
        self.cx.run_until_parked();
    }

    /// Draws a frame and returns what the window rendered
    /// (`size * `[`HEADLESS_SCALE_FACTOR`]` pixels).
    pub fn capture(&mut self, window: AnyWindowHandle) -> Result<RgbaImage> {
        capture_window(&mut self.cx, window)
    }
}

impl Default for ScreenshotApp {
    fn default() -> Self {
        Self::new()
    }
}

/// One screenshot of a `harness = false` test: a name (the golden's file stem) and a function that
/// renders it.
pub struct GoldenCase {
    /// Golden file stem, `tests/goldens/<os>/<name>.png`.
    pub name: &'static str,
    /// Logical size of the window the case renders, to check the image's pixel size.
    pub size: (u32, u32),
    /// Renders the case, usually with a fresh [`ScreenshotApp`].
    pub render: fn() -> Result<RgbaImage>,
}

/// Checks one case: the image has the pixel size of the window, it is not blank, and it matches the
/// golden when one exists for this OS (or `OXIKUBE_UPDATE_GOLDENS=1` writes it).
fn check_case(goldens: &Path, case: &GoldenCase, tolerance: Tolerance) -> Result<String> {
    let image = (case.render)()?;
    let expected_size = (
        case.size.0 * HEADLESS_SCALE_FACTOR,
        case.size.1 * HEADLESS_SCALE_FACTOR,
    );
    ensure!(
        image.dimensions() == expected_size,
        "{}: unexpected image size {:?}, expected {expected_size:?}",
        case.name,
        image.dimensions()
    );
    ensure!(
        distinct_colors_at_least(&image, 8),
        "{}: the frame looks blank",
        case.name
    );
    let golden = golden_path(goldens, case.name);
    let updating = std::env::var_os(UPDATE_GOLDENS_ENV).is_some_and(|v| v != "0" && !v.is_empty());
    if golden.exists() || updating {
        check_golden(&image, &golden, tolerance)?;
        Ok(format!("{} matches {}", case.name, golden.display()))
    } else {
        Ok(format!(
            "{}: no golden for {} yet; structural checks only",
            case.name,
            std::env::consts::OS
        ))
    }
}

/// The `main` of a screenshot test (`[[test]] harness = false`): runs every case, prints one line
/// each, and exits non-zero when any failed. A failing case does not stop the others, so one run
/// (a nightly leg) reports every broken golden. `crate_dir` is `env!("CARGO_MANIFEST_DIR")`;
/// goldens live in `<crate_dir>/tests/goldens/<os>/`.
#[allow(clippy::print_stdout, clippy::print_stderr)]
pub fn run_golden_cases(crate_dir: &str, cases: &[GoldenCase]) -> ExitCode {
    let goldens = Path::new(crate_dir).join("tests/goldens");
    let mut failed = 0;
    for case in cases {
        match check_case(&goldens, case, Tolerance::default()) {
            Ok(line) => println!("{line}"),
            Err(err) => {
                eprintln!("screenshot case `{}` failed: {err:#}", case.name);
                failed += 1;
            }
        }
    }
    if failed > 0 {
        eprintln!("{failed} of {} screenshot case(s) failed", cases.len());
        return ExitCode::FAILURE;
    }
    println!("{} screenshot case(s): ok", cases.len());
    ExitCode::SUCCESS
}
