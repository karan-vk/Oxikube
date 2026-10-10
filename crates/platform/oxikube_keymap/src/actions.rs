//! The keymap's own actions.

use gpui::actions;

actions!(
    keymap,
    [
        /// Open the user's `keymap.json`, creating it from the commented template when it does not
        /// exist. Named like its command (`keymap::OpenUser`), so a key bound to it in
        /// `keymap.json`, the palette and the MCP tool run one behaviour. The binary handles it:
        /// the keymap is platform code and cannot open a tab or an editor.
        OpenUser,
    ]
);
