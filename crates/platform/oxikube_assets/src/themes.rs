//! Bundled theme families (E05-S08), embedded with `include_str!` so a lookup costs nothing
//! until a theme is actually parsed.
//!
//! The files use Zed's theme-family JSON format (schema v0.2.0); `oxikube_theme` parses them.
//! Four families ship (E22-S06): One (Dark/Light), Ayu (Dark/Light/Mirage) and Gruvbox (six
//! variants), all MIT and taken from Zed's `assets/themes/`, plus Oxikube (Dark/Light), authored
//! for this project with an explicit `oxikube` status-colour block. Licence texts are in
//! `assets/themes/LICENSES.md` and every family is named in `THIRD_PARTY_NOTICES.md`.
//! `oxikube_theme` still falls back to One for any key a family leaves unset.

/// One embedded theme family file.
#[derive(Clone, Copy, Debug)]
pub struct BundledThemeFamily {
    /// File name under `assets/themes/`, for diagnostics.
    pub file: &'static str,
    /// The family JSON text.
    pub json: &'static str,
}

/// Every bundled theme family file, in registration order.
pub const BUNDLED_THEME_FAMILIES: &[BundledThemeFamily] = &[
    BundledThemeFamily {
        file: "one.json",
        json: include_str!("../assets/themes/one.json"),
    },
    BundledThemeFamily {
        file: "oxikube.json",
        json: include_str!("../assets/themes/oxikube.json"),
    },
    BundledThemeFamily {
        file: "ayu.json",
        json: include_str!("../assets/themes/ayu.json"),
    },
    BundledThemeFamily {
        file: "gruvbox.json",
        json: include_str!("../assets/themes/gruvbox.json"),
    },
];

/// The licence texts of the third-party families, shipped next to the files.
pub const BUNDLED_THEME_LICENSES: &str = include_str!("../assets/themes/LICENSES.md");

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

    #[test]
    fn the_four_story_families_are_bundled() {
        let files: Vec<_> = BUNDLED_THEME_FAMILIES.iter().map(|f| f.file).collect();
        assert_eq!(
            files,
            ["one.json", "oxikube.json", "ayu.json", "gruvbox.json"]
        );
    }

    #[test]
    fn every_third_party_family_has_a_licence_and_notice() {
        let notices = include_str!("../../../../THIRD_PARTY_NOTICES.md");
        for (needle, licence) in [
            ("One", "One Dark"),
            ("Ayu", "Ayu Dark"),
            ("Gruvbox", "Gruvbox Dark"),
        ] {
            assert!(
                BUNDLED_THEME_LICENSES.contains(licence),
                "{licence} licence text"
            );
            assert!(
                notices.contains(needle),
                "{needle} is named in THIRD_PARTY_NOTICES.md"
            );
        }
        assert!(
            notices.contains("assets/themes/ayu.json"),
            "Ayu file path in notices"
        );
        assert!(
            notices.contains("assets/themes/gruvbox.json"),
            "Gruvbox file path in notices"
        );
        assert!(
            notices.contains("assets/themes/oxikube.json"),
            "Oxikube file path in notices"
        );
    }

    #[test]
    fn third_party_files_keep_an_author() {
        for family in BUNDLED_THEME_FAMILIES {
            assert!(
                family.json.contains("\"author\""),
                "{} keeps its author field",
                family.file
            );
        }
    }
}
