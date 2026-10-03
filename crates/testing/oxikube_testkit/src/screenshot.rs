//! Screenshot helpers: save a PNG and compare an image against a golden with tolerance.
//!
//! Pure image code (no window, no gpui), so the comparison logic is testable everywhere.
//!
//! # Goldens
//!
//! Goldens live under `tests/goldens/<os>/<name>.png` in the crate that owns the test
//! (`<os>` is [`std::env::consts::OS`], e.g. `macos` or `linux`), because glyph rasterisation
//! differs per platform. Use [`golden_path`] to build the path and [`assert_matches_golden`] to
//! compare. To (re)generate goldens, run the test with `OXIKUBE_UPDATE_GOLDENS=1`, review the
//! PNG, and commit it. On a mismatch the actual image is written next to the golden as
//! `<name>.actual.png` and a red-highlight diff as `<name>.diff.png` (both gitignored).

use anyhow::{Context as _, Result, bail, ensure};
use image::Rgba;
pub use image::RgbaImage;
use std::path::{Path, PathBuf};

/// Environment variable that makes [`assert_matches_golden`] overwrite the golden instead of comparing.
pub const UPDATE_GOLDENS_ENV: &str = "OXIKUBE_UPDATE_GOLDENS";

/// How different two images may be and still count as equal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// Largest per-channel (R, G, B, A) absolute difference that still counts as the same pixel.
    pub channel: u8,
    /// Largest fraction of pixels (0.0..=1.0) allowed to differ by more than `channel`.
    pub max_diff_ratio: f64,
}

impl Tolerance {
    /// Pixel-exact: any difference fails.
    pub const EXACT: Tolerance = Tolerance {
        channel: 0,
        max_diff_ratio: 0.0,
    };
}

impl Default for Tolerance {
    /// Absorbs anti-aliasing noise: channels may differ by 8, up to 0.1% of pixels may exceed that.
    fn default() -> Self {
        Tolerance {
            channel: 8,
            max_diff_ratio: 0.001,
        }
    }
}

/// Result of [`compare`].
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// Dimensions of the actual image.
    pub actual_size: (u32, u32),
    /// Dimensions of the expected image.
    pub expected_size: (u32, u32),
    /// Pixels differing by more than the channel tolerance (0 when sizes differ).
    pub differing_pixels: u64,
    /// Total pixels compared (0 when sizes differ).
    pub total_pixels: u64,
    /// Largest per-channel difference seen anywhere.
    pub max_channel_delta: u8,
}

impl Comparison {
    /// Fraction of pixels that differ beyond the channel tolerance.
    pub fn diff_ratio(&self) -> f64 {
        if self.total_pixels == 0 {
            0.0
        } else {
            self.differing_pixels as f64 / self.total_pixels as f64
        }
    }

    /// Whether the images have identical dimensions.
    pub fn same_size(&self) -> bool {
        self.actual_size == self.expected_size
    }

    /// Whether the comparison is within `tolerance`. Different sizes never match.
    pub fn is_within(&self, tolerance: Tolerance) -> bool {
        self.same_size() && self.diff_ratio() <= tolerance.max_diff_ratio
    }
}

/// Compares `actual` against `expected` pixel by pixel.
pub fn compare(actual: &RgbaImage, expected: &RgbaImage, tolerance: Tolerance) -> Comparison {
    let mut result = Comparison {
        actual_size: actual.dimensions(),
        expected_size: expected.dimensions(),
        differing_pixels: 0,
        total_pixels: 0,
        max_channel_delta: 0,
    };
    if !result.same_size() {
        return result;
    }
    result.total_pixels = u64::from(actual.width()) * u64::from(actual.height());
    for (a, e) in actual.pixels().zip(expected.pixels()) {
        let delta = pixel_delta(a, e);
        result.max_channel_delta = result.max_channel_delta.max(delta);
        if delta > tolerance.channel {
            result.differing_pixels += 1;
        }
    }
    result
}

fn pixel_delta(a: &Rgba<u8>, e: &Rgba<u8>) -> u8 {
    a.0.iter()
        .zip(e.0.iter())
        .map(|(a, e)| a.abs_diff(*e))
        .max()
        .unwrap_or(0)
}

/// Builds an image highlighting (in opaque red) every pixel that differs beyond `tolerance`;
/// other pixels are dimmed copies of `actual`. Returns `None` when sizes differ.
pub fn diff_image(
    actual: &RgbaImage,
    expected: &RgbaImage,
    tolerance: Tolerance,
) -> Option<RgbaImage> {
    if actual.dimensions() != expected.dimensions() {
        return None;
    }
    let mut out = RgbaImage::new(actual.width(), actual.height());
    for (x, y, a) in actual.enumerate_pixels() {
        let e = expected.get_pixel(x, y);
        let px = if pixel_delta(a, e) > tolerance.channel {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([a[0] / 3, a[1] / 3, a[2] / 3, 255])
        };
        out.put_pixel(x, y, px);
    }
    Some(out)
}

/// Writes `image` as a PNG to `path`, creating parent directories.
pub fn save_png(image: &RgbaImage, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    image
        .save_with_format(path, image::ImageFormat::Png)
        .with_context(|| format!("writing PNG {}", path.display()))
}

/// Reads a PNG (or any enabled format) from `path` as RGBA8.
pub fn load_png(path: &Path) -> Result<RgbaImage> {
    Ok(image::open(path)
        .with_context(|| format!("reading image {}", path.display()))?
        .to_rgba8())
}

/// Path of the golden `name` under `goldens_root` for the current OS:
/// `<goldens_root>/<os>/<name>.png`. Callers pass `<crate>/tests/goldens`.
pub fn golden_path(goldens_root: &Path, name: &str) -> PathBuf {
    goldens_root
        .join(std::env::consts::OS)
        .join(format!("{name}.png"))
}

/// Compares `actual` with the golden at `golden`, honouring `tolerance`.
///
/// With `OXIKUBE_UPDATE_GOLDENS=1` the golden is (over)written and the check passes. A missing
/// golden is an error that says how to create it. On mismatch, writes `<name>.actual.png` and
/// `<name>.diff.png` beside the golden and returns an error describing the difference.
pub fn check_golden(actual: &RgbaImage, golden: &Path, tolerance: Tolerance) -> Result<()> {
    if std::env::var_os(UPDATE_GOLDENS_ENV).is_some_and(|v| v != "0" && !v.is_empty()) {
        return save_png(actual, golden);
    }
    ensure!(
        golden.exists(),
        "golden {} does not exist; run with {UPDATE_GOLDENS_ENV}=1 to create it",
        golden.display()
    );
    let expected = load_png(golden)?;
    let comparison = compare(actual, &expected, tolerance);
    if comparison.is_within(tolerance) {
        return Ok(());
    }
    let stem = golden
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let actual_path = golden.with_file_name(format!("{stem}.actual.png"));
    save_png(actual, &actual_path)?;
    if let Some(diff) = diff_image(actual, &expected, tolerance) {
        save_png(&diff, &golden.with_file_name(format!("{stem}.diff.png")))?;
    }
    if !comparison.same_size() {
        bail!(
            "golden {} size mismatch: actual {:?}, expected {:?} (actual saved to {})",
            golden.display(),
            comparison.actual_size,
            comparison.expected_size,
            actual_path.display()
        );
    }
    bail!(
        "golden {} differs: {} of {} pixels ({:.4}%) beyond channel tolerance {} \
         (allowed {:.4}%), max channel delta {} (actual saved to {})",
        golden.display(),
        comparison.differing_pixels,
        comparison.total_pixels,
        comparison.diff_ratio() * 100.0,
        tolerance.channel,
        tolerance.max_diff_ratio * 100.0,
        comparison.max_channel_delta,
        actual_path.display()
    )
}

/// Panicking wrapper over [`check_golden`] for use in tests: golden `name` under
/// `goldens_root` (see [`golden_path`]).
#[track_caller]
pub fn assert_matches_golden(
    actual: &RgbaImage,
    goldens_root: &Path,
    name: &str,
    tolerance: Tolerance,
) {
    if let Err(err) = check_golden(actual, &golden_path(goldens_root, name), tolerance) {
        panic!("{err:#}");
    }
}

/// Whether `image` has at least `min_colors` distinct colours; a cheap "something was drawn,
/// it is not a blank frame" check for smoke tests.
pub fn distinct_colors_at_least(image: &RgbaImage, min_colors: usize) -> bool {
    let mut seen = std::collections::HashSet::new();
    for px in image.pixels() {
        if seen.insert(px.0) && seen.len() >= min_colors {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            Rgba([(x * 255 / width) as u8, (y * 255 / height) as u8, 128, 255])
        })
    }

    fn goldens() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn identical_image_matches_golden() {
        let dir = goldens();
        let image = gradient(64, 48);
        let path = golden_path(dir.path(), "gradient");
        save_png(&image, &path).unwrap();
        check_golden(&image, &path, Tolerance::EXACT).unwrap();
        assert!(path.ends_with(format!("{}/gradient.png", std::env::consts::OS)));
    }

    #[test]
    fn small_noise_within_tolerance_matches() {
        let dir = goldens();
        let mut noisy = gradient(64, 48);
        let path = golden_path(dir.path(), "noise");
        save_png(&noisy, &path).unwrap();
        // Every pixel off by 3 per channel: within channel tolerance 8.
        for px in noisy.pixels_mut() {
            px[0] = px[0].saturating_add(3);
        }
        check_golden(&noisy, &path, Tolerance::default()).unwrap();
        // ...but not pixel exact.
        assert!(check_golden(&noisy, &path, Tolerance::EXACT).is_err());
    }

    #[test]
    fn deliberately_different_image_fails() {
        let dir = goldens();
        let golden = gradient(64, 48);
        let path = golden_path(dir.path(), "changed");
        save_png(&golden, &path).unwrap();

        let mut changed = golden.clone();
        // A 16x16 block (8% of the image) is wildly different.
        for y in 0..16 {
            for x in 0..16 {
                changed.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let err = check_golden(&changed, &path, Tolerance::default()).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("differs"), "{message}");

        let comparison = compare(&changed, &golden, Tolerance::default());
        assert!(comparison.differing_pixels > 200, "{comparison:?}");
        assert!(!comparison.is_within(Tolerance::default()));
        // A loose enough ratio accepts it: the ratio really is what is compared.
        assert!(comparison.is_within(Tolerance {
            channel: 8,
            max_diff_ratio: 0.5
        }));

        // Failure artefacts are written next to the golden.
        assert!(path.with_file_name("changed.actual.png").exists());
        let diff = load_png(&path.with_file_name("changed.diff.png")).unwrap();
        assert_eq!(diff.get_pixel(0, 0), &Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn size_mismatch_fails() {
        let dir = goldens();
        let path = golden_path(dir.path(), "size");
        save_png(&gradient(64, 48), &path).unwrap();
        let err = check_golden(&gradient(32, 48), &path, Tolerance::default()).unwrap_err();
        assert!(format!("{err:#}").contains("size mismatch"));
    }

    #[test]
    fn missing_golden_explains_how_to_create_it() {
        let dir = goldens();
        let path = golden_path(dir.path(), "absent");
        let err = check_golden(&gradient(8, 8), &path, Tolerance::default()).unwrap_err();
        assert!(format!("{err:#}").contains(UPDATE_GOLDENS_ENV));
    }

    #[test]
    fn blank_image_has_one_colour() {
        let blank = RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255]));
        assert!(!distinct_colors_at_least(&blank, 2));
        assert!(distinct_colors_at_least(&gradient(8, 8), 2));
    }
}
