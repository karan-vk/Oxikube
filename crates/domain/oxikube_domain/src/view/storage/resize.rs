//! [`check_resize`]: the rule behind the PVC resize dialog and the `pvc::Resize` handler.

use std::fmt;
use std::sync::Arc;

use super::class::StorageClassSummary;
use crate::quantity::Quantity;

/// Whether a claim's StorageClass lets it grow (`allowVolumeExpansion`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expansion {
    /// The class allows expansion.
    Allowed,
    /// The class exists and does not allow expansion.
    Disallowed {
        /// The class.
        class: Arc<str>,
    },
    /// The claim names no class (a pre-provisioned or statically bound claim): the API server
    /// refuses to resize a claim whose class cannot be checked.
    NoClass,
    /// The claim names a class that could not be read (it is gone, or the user may not read
    /// StorageClasses). The server's own check decides, and a dry run reports it.
    Unknown {
        /// The class.
        class: Arc<str>,
    },
}

impl Expansion {
    /// What `class` (the claim's class, read, or `None` when it could not be) says for a claim
    /// that names `class_name`.
    pub fn of(class_name: Option<&str>, class: Option<&StorageClassSummary>) -> Self {
        match (class_name, class) {
            (None, _) => Self::NoClass,
            (Some(_), Some(class)) if class.allow_expansion => Self::Allowed,
            (Some(_), Some(class)) => Self::Disallowed {
                class: class.name.clone(),
            },
            (Some(name), None) => Self::Unknown {
                class: Arc::from(name),
            },
        }
    }
}

/// Why a resize is refused before anything is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResizeBlock {
    /// Nothing was typed.
    Empty,
    /// The text is not a quantity (`20Gi`, `500M`).
    Invalid(String),
    /// The new size is not greater than the current one: a volume can only grow.
    NotLarger {
        /// The current size.
        current: Quantity,
        /// The size asked for.
        requested: Quantity,
    },
    /// The class does not allow expansion.
    ExpansionDisallowed {
        /// The class.
        class: Arc<str>,
    },
    /// The claim has no StorageClass.
    NoStorageClass,
}

impl fmt::Display for ResizeBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("Enter the new size, for example 20Gi."),
            Self::Invalid(text) => write!(f, "{text:?} is not a size; use a quantity such as 20Gi."),
            Self::NotLarger { current, requested } => write!(
                f,
                "The new size {requested} must be greater than the current size {current}; a volume cannot shrink."
            ),
            Self::ExpansionDisallowed { class } => write!(
                f,
                "StorageClass {class} does not allow volume expansion (allowVolumeExpansion is not true)."
            ),
            Self::NoStorageClass => f.write_str(
                "This claim has no StorageClass, and only claims provisioned from a class that allows expansion can be resized.",
            ),
        }
    }
}

impl std::error::Error for ResizeBlock {}

/// Checks a resize of a claim now asking for `current` (its requested size, else its capacity;
/// `None` when it states none) to the size typed as `input`, in a class with `expansion`.
///
/// Returns the new size, or the first reason it is refused: nothing typed, not a quantity, a
/// class that cannot be checked or does not allow expansion, a size that is not greater than the
/// current one. An [`Expansion::Unknown`] class does not block: the server decides, and the
/// dry run the guard's handler sends first reports its refusal.
///
/// # Errors
///
/// A [`ResizeBlock`].
pub fn check_resize(
    current: Option<&Quantity>,
    input: &str,
    expansion: &Expansion,
) -> Result<Quantity, ResizeBlock> {
    match expansion {
        Expansion::NoClass => return Err(ResizeBlock::NoStorageClass),
        Expansion::Disallowed { class } => {
            return Err(ResizeBlock::ExpansionDisallowed {
                class: class.clone(),
            });
        }
        Expansion::Allowed | Expansion::Unknown { .. } => {}
    }
    let input = input.trim();
    if input.is_empty() {
        return Err(ResizeBlock::Empty);
    }
    let requested = Quantity::parse(input).map_err(|_| ResizeBlock::Invalid(input.to_owned()))?;
    if requested.is_negative() || requested.is_zero() {
        return Err(ResizeBlock::Invalid(input.to_owned()));
    }
    match current {
        Some(current) if &requested <= current => Err(ResizeBlock::NotLarger {
            current: *current,
            requested,
        }),
        _ => Ok(requested),
    }
}
