//! [`Redacted`]: a wrapper whose `Debug` and `Display` never show the inner value.

use super::patterns::MARKER;
use std::fmt;

/// Holds a secret-bearing value and prints [`MARKER`] instead of it.
///
/// Wrap a field of a derived-`Debug` struct (`token: Redacted<String>`) and the derive cannot
/// leak it. The wrapper deliberately does not implement `Serialize`, `Deref` or `AsRef`: reading
/// the value takes an explicit [`expose`](Redacted::expose). For credentials that must also be
/// zeroised on drop prefer `secrecy::SecretString`; this type is for values that only need to
/// stay out of formatted output.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    /// Wraps `value`.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Borrows the inner value. Calling this is the audit point for "does this leak?".
    pub fn expose(&self) -> &T {
        &self.0
    }

    /// Unwraps the inner value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> From<T> for Redacted<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(MARKER)
    }
}

impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(MARKER)
    }
}
