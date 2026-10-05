//! Bundled theme families (E05-S08), embedded with `include_str!` so a lookup costs nothing
//! until a theme is actually parsed.
//!
//! The files use Zed's theme-family JSON format (schema v0.2.0); `oxikube_theme` parses them.
//! Only One Dark and One Light ship: both are MIT (Atom's `one-*-ui`, via Zed's
//! `assets/themes/one/one.json`, see `assets/themes/LICENSES.md` and `THIRD_PARTY_NOTICES.md`).
//! Other families (Ayu, Gruvbox) are test fixtures of `oxikube_theme`, not bundled.

/// One embedded theme family file.
#[derive(Clone, Copy, Debug)]
pub struct BundledThemeFamily {
    /// File name under `assets/themes/`, for diagnostics.
    pub file: &'static str,
    /// The family JSON text.
    pub json: &'static str,
}

/// Every bundled theme family file, in registration order.
pub const BUNDLED_THEME_FAMILIES: &[BundledThemeFamily] = &[BundledThemeFamily {
    file: "one.json",
    json: include_str!("../assets/themes/one.json"),
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_families_are_non_empty_theme_family_json() {
        for family in BUNDLED_THEME_FAMILIES {
            assert!(family.json.contains("\"themes\""), "{}", family.file);
            assert!(family.json.contains("v0.2.0"), "{}", family.file);
        }
    }
}
