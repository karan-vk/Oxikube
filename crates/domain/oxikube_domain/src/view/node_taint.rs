//! [`NodeTaint`] and [`TaintEffect`]: a node's `spec.taints`, formatted the way `kubectl describe
//! node` prints them.

use std::fmt;
use std::sync::Arc;

use super::{arc_of, arr_of, str_of, sub};
use crate::resource::Resource;

/// What a taint does to pods that do not tolerate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaintEffect {
    /// New pods are not scheduled onto the node.
    NoSchedule,
    /// The scheduler tries to avoid the node.
    PreferNoSchedule,
    /// New pods are not scheduled and running ones are evicted.
    NoExecute,
}

impl TaintEffect {
    /// Parse the API spelling; `None` for an effect this client does not know.
    pub fn parse(effect: &str) -> Option<Self> {
        match effect {
            "NoSchedule" => Some(Self::NoSchedule),
            "PreferNoSchedule" => Some(Self::PreferNoSchedule),
            "NoExecute" => Some(Self::NoExecute),
            _ => None,
        }
    }

    /// The API spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoSchedule => "NoSchedule",
            Self::PreferNoSchedule => "PreferNoSchedule",
            Self::NoExecute => "NoExecute",
        }
    }
}

/// One entry of `spec.taints`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeTaint {
    /// The taint key, for example `node-role.kubernetes.io/control-plane`.
    pub key: Arc<str>,
    /// The value, when set.
    pub value: Option<Arc<str>>,
    /// The effect; `None` when the server sent one this client does not know (shown as written
    /// in [`raw_effect`](Self::raw_effect)).
    pub effect: Option<TaintEffect>,
    /// The effect as the API spelled it.
    pub raw_effect: Arc<str>,
}

impl NodeTaint {
    /// The taints of a Node, in API order. Entries without a key are skipped.
    pub fn of_node(res: &Resource) -> Vec<NodeTaint> {
        arr_of(sub(res.json(), "spec"), "taints")
            .iter()
            .filter_map(|t| {
                let raw_effect = arc_of(t, "effect").unwrap_or_else(|| Arc::from(""));
                Some(NodeTaint {
                    key: Arc::from(str_of(t, "key")?),
                    value: arc_of(t, "value"),
                    effect: TaintEffect::parse(&raw_effect),
                    raw_effect,
                })
            })
            .collect()
    }

    /// Whether this is the taint the node lifecycle controller adds to a cordoned node.
    pub fn is_unschedulable(&self) -> bool {
        &*self.key == "node.kubernetes.io/unschedulable"
    }
}

/// `key=value:Effect`, or `key:Effect` without a value (kubectl's form).
impl fmt::Display for NodeTaint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key)?;
        if let Some(value) = &self.value {
            write!(f, "={value}")?;
        }
        write!(f, ":{}", self.raw_effect)
    }
}
