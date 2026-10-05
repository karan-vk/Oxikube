//! Keymap assets (E05-S07), embedded with `include_str!` so reading them costs nothing at
//! startup. The format is Zed's `keymap.json` (JSON with comments); `oxikube_keymap` parses and
//! layers them.

/// The OS a default keymap is written for. Shortcuts differ (`cmd` vs `ctrl`), so each OS has
/// its own file, like Zed's `default-macos.json` / `default-linux.json` / `default-windows.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeymapPlatform {
    /// macOS (`cmd` is the primary modifier).
    MacOs,
    /// Linux and the BSDs (`ctrl`).
    Linux,
    /// Windows (`ctrl`).
    Windows,
}

impl KeymapPlatform {
    /// The platform this binary was compiled for. Anything that is not macOS or Windows
    /// follows the Linux keymap.
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// The embedded default keymap for `platform` (the bottom layer of the keymap).
pub fn default_keymap(platform: KeymapPlatform) -> &'static str {
    match platform {
        KeymapPlatform::MacOs => include_str!("../assets/keymaps/default-macos.json"),
        KeymapPlatform::Linux => include_str!("../assets/keymaps/default-linux.json"),
        KeymapPlatform::Windows => include_str!("../assets/keymaps/default-windows.json"),
    }
}

/// The embedded optional vim layer (`vim.json`), enabled by the keymap's vim flag.
pub fn vim_keymap() -> &'static str {
    include_str!("../assets/keymaps/vim.json")
}

/// The commented template for a new user `keymap.json`.
pub fn initial_user_keymap_content() -> &'static str {
    include_str!("../assets/keymaps/initial_user_keymap.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_keymap_is_non_empty_json_array_text() {
        for text in [
            default_keymap(KeymapPlatform::MacOs),
            default_keymap(KeymapPlatform::Linux),
            default_keymap(KeymapPlatform::Windows),
            vim_keymap(),
            initial_user_keymap_content(),
        ] {
            assert!(text.contains('['), "{text}");
        }
    }

    #[test]
    fn the_os_keymaps_differ() {
        assert_ne!(
            default_keymap(KeymapPlatform::MacOs),
            default_keymap(KeymapPlatform::Linux)
        );
        assert_ne!(
            default_keymap(KeymapPlatform::Windows),
            default_keymap(KeymapPlatform::Linux)
        );
    }
}
