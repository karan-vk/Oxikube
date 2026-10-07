//! Clamping of out-of-range values with a warning.

/// Fewest lines `logs.default_tail` may ask for.
pub const MIN_DEFAULT_TAIL: u32 = 1;
/// Most lines `logs.default_tail` may ask for (a read of the tail is one request; this keeps it
/// within what the buffer's default holds twice over).
pub const MAX_DEFAULT_TAIL: u32 = 100_000;

/// `value` (or `default` when unset) kept within `min..=max`; a value outside it is clamped and
/// the log says which key and what it was clamped to. The settings file is the user's, so the
/// value is shown as written (a number).
pub(super) fn clamp(key: &str, value: Option<i64>, default: i64, min: i64, max: i64) -> i64 {
    let Some(value) = value else { return default };
    let clamped = value.clamp(min, max);
    if clamped != value {
        tracing::warn!(
            key,
            value,
            clamped,
            "setting out of range ({min} to {max}): using the nearest allowed value"
        );
    }
    clamped
}
