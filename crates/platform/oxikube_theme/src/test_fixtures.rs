//! Zed theme files used as test input.
//!
//! Ayu and Gruvbox are copied from Zed's `assets/themes` (MIT, see
//! `tests/fixtures/LICENSES.md` and `THIRD_PARTY_NOTICES.md`) for tests only; they are not
//! bundled in the application. One is the bundled theme itself.

use crate::import::{ImportedFamily, import_family};

/// Ayu: Ayu Dark, Ayu Light, Ayu Mirage.
pub(crate) const AYU: &str = include_str!("../tests/fixtures/ayu.json");
/// Gruvbox: six dark/light variants.
pub(crate) const GRUVBOX: &str = include_str!("../tests/fixtures/gruvbox.json");
/// One: One Dark and One Light.
pub(crate) const ONE: &str = include_str!("../tests/fixtures/one.json");

/// Imports a fixture, panicking with the error on failure.
pub(crate) fn import(text: &str) -> ImportedFamily {
    import_family(text).expect("fixture imports")
}
