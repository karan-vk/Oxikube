//! The layers a keymap is merged from and how they rank.

use gpui::KeyBindingMetaIndex;

/// One source of key bindings. Layers are applied lowest first, so a higher layer wins when
/// two bindings match the same keystroke in the same context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeymapLayer {
    /// The embedded per-OS `default-*.json`.
    Default,
    /// The optional embedded `vim.json`, between the defaults and the user's file.
    Vim,
    /// The user's `keymap.json`.
    User,
}

impl KeymapLayer {
    /// Layers in the order their bindings are added to GPUI's keymap (lowest precedence first).
    pub const ORDER: [KeymapLayer; 3] = [Self::Default, Self::Vim, Self::User];

    /// The metadata GPUI stores with each binding of this layer.
    ///
    /// GPUI's `null` handling compares it: a `NoAction` binding hides bindings of an equal or
    /// weaker source (a larger index) and never one of a stronger source, so a default's `null`
    /// cannot hide a user's binding while the user's `null` hides any default.
    pub const fn meta(self) -> KeyBindingMetaIndex {
        KeyBindingMetaIndex(match self {
            Self::User => 0,
            Self::Vim => 1,
            Self::Default => 2,
        })
    }

    /// The layer a binding was loaded from, from the metadata [`KeymapLayer::meta`] set.
    pub fn from_meta(meta: Option<KeyBindingMetaIndex>) -> Option<Self> {
        match meta?.0 {
            0 => Some(Self::User),
            1 => Some(Self::Vim),
            2 => Some(Self::Default),
            _ => None,
        }
    }

    /// Whether bindings naming an unregistered action are skipped silently instead of reported.
    ///
    /// The embedded layers are written ahead of the crates that register their actions, and a
    /// feature crate may be absent from a build; the user's file is validated strictly.
    pub const fn tolerates_unknown_actions(self) -> bool {
        !matches!(self, Self::User)
    }
}

impl std::fmt::Display for KeymapLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Default => "default keymap",
            Self::Vim => "vim keymap",
            Self::User => "keymap.json",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_round_trips_and_ranks_user_strongest() {
        for layer in KeymapLayer::ORDER {
            assert_eq!(KeymapLayer::from_meta(Some(layer.meta())), Some(layer));
        }
        assert!(KeymapLayer::User.meta().0 < KeymapLayer::Vim.meta().0);
        assert!(KeymapLayer::Vim.meta().0 < KeymapLayer::Default.meta().0);
        assert_eq!(KeymapLayer::from_meta(None), None);
    }
}
