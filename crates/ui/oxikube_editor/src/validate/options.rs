//! Knobs of the validator.

/// What [`validate`](super::validate) checks and how much it reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidateOptions {
    /// Skip the root `status` field. The server owns it and ignores what a client sends, so the
    /// editor dims it (E10-S06) and the validator does not look inside. Default `true`.
    pub skip_status: bool,
    /// Stop after this many diagnostics, so a hopelessly wrong 5 MB buffer cannot flood the
    /// editor. Default [`ValidateOptions::DEFAULT_MAX_DIAGNOSTICS`].
    pub max_diagnostics: usize,
}

impl ValidateOptions {
    /// The default cap on diagnostics per validation (the same as the YAML model's cap on
    /// syntax diagnostics).
    pub const DEFAULT_MAX_DIAGNOSTICS: usize = 1_000;
}

impl Default for ValidateOptions {
    fn default() -> Self {
        Self {
            skip_status: true,
            max_diagnostics: Self::DEFAULT_MAX_DIAGNOSTICS,
        }
    }
}
