//! [`AccessMode`] and [`ReclaimPolicy`].

use std::fmt;
use std::sync::Arc;

/// A volume access mode (`spec.accessModes`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AccessMode {
    /// `ReadWriteOnce`: one node reads and writes.
    ReadWriteOnce,
    /// `ReadOnlyMany`: many nodes read.
    ReadOnlyMany,
    /// `ReadWriteMany`: many nodes read and write.
    ReadWriteMany,
    /// `ReadWriteOncePod`: one pod reads and writes.
    ReadWriteOncePod,
    /// A mode this version does not know, kept as written.
    Other(Arc<str>),
}

impl AccessMode {
    /// Parses the API spelling (`ReadWriteOnce`).
    pub fn parse(text: &str) -> Self {
        match text {
            "ReadWriteOnce" => Self::ReadWriteOnce,
            "ReadOnlyMany" => Self::ReadOnlyMany,
            "ReadWriteMany" => Self::ReadWriteMany,
            "ReadWriteOncePod" => Self::ReadWriteOncePod,
            other => Self::Other(Arc::from(other)),
        }
    }

    /// kubectl's short code: `RWO`, `ROX`, `RWX`, `RWOP`; an unknown mode as written.
    pub fn short(&self) -> &str {
        match self {
            Self::ReadWriteOnce => "RWO",
            Self::ReadOnlyMany => "ROX",
            Self::ReadWriteMany => "RWX",
            Self::ReadWriteOncePod => "RWOP",
            Self::Other(text) => text,
        }
    }

    /// The API spelling.
    pub fn as_str(&self) -> &str {
        match self {
            Self::ReadWriteOnce => "ReadWriteOnce",
            Self::ReadOnlyMany => "ReadOnlyMany",
            Self::ReadWriteMany => "ReadWriteMany",
            Self::ReadWriteOncePod => "ReadWriteOncePod",
            Self::Other(text) => text,
        }
    }
}

impl fmt::Display for AccessMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.short())
    }
}

/// `RWO,RWX`: the short codes of `modes`, comma-separated, in order.
pub fn access_modes_text(modes: &[AccessMode]) -> String {
    let mut out = String::new();
    for mode in modes {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(mode.short());
    }
    out
}

/// What happens to a volume when its claim is deleted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReclaimPolicy {
    /// The volume and its data are kept (`Retain`).
    Retain,
    /// The volume and its data are deleted (`Delete`).
    Delete,
    /// The data is scrubbed and the volume reused (`Recycle`, deprecated).
    Recycle,
    /// A policy this version does not know, kept as written.
    Other(Arc<str>),
}

impl ReclaimPolicy {
    /// Parses the API spelling.
    pub fn parse(text: &str) -> Self {
        match text {
            "Retain" => Self::Retain,
            "Delete" => Self::Delete,
            "Recycle" => Self::Recycle,
            other => Self::Other(Arc::from(other)),
        }
    }

    /// The API spelling.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Retain => "Retain",
            Self::Delete => "Delete",
            Self::Recycle => "Recycle",
            Self::Other(text) => text,
        }
    }

    /// Whether deleting the claim loses the data.
    pub fn deletes_data(&self) -> bool {
        matches!(self, Self::Delete | Self::Recycle)
    }
}

impl fmt::Display for ReclaimPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
