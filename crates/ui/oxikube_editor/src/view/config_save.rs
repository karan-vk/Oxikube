//! Saving a config file from the editor (E21-S03): the buffer is written to the file's path off
//! the UI thread, atomically, and only while the file still holds the text the buffer started
//! from, so a change by another writer (a binding recorded in the keymap editor) is reported
//! instead of silently overwritten.

use gpui::{AppContext as _, Context, Window};
use oxikube_domain::OxiResult;
use oxikube_keymap::edit::UserKeymapSave;

use super::config_file::ConfigFileKind;
use super::manifest_editor::ManifestEditor;
use super::notice::{Notice, NoticeLevel};

impl ManifestEditor {
    /// Writes the buffer to the config file's path, off the UI thread, atomically (a reader such
    /// as the keymap's watcher never sees half a file), and only while the file still holds the
    /// text the buffer started from: a change by another writer is reported instead of lost.
    pub(super) fn save_config_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(link) = self.config_file.clone() else {
            return;
        };
        let snapshot = self.editor.read(cx).snapshot(cx);
        let version = snapshot.version();
        let kind = link.file.kind;
        self.apply.task = Some(cx.spawn_in(window, async move |this, cx| {
            let text = snapshot.to_text();
            let written = cx
                .background_spawn(async move {
                    let outcome = match kind {
                        ConfigFileKind::Keymap => oxikube_keymap::edit::save_user_keymap_text(
                            &link.file.path,
                            &link.on_disk,
                            &text,
                        ),
                    };
                    outcome.map(|outcome| (outcome, text.to_string()))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.config_file_saved(kind, version, written, cx)
            });
        }));
    }

    fn config_file_saved(
        &mut self,
        kind: ConfigFileKind,
        version: u64,
        written: OxiResult<(UserKeymapSave, String)>,
        cx: &mut Context<Self>,
    ) {
        let notice = match written {
            Ok((UserKeymapSave::Saved, text)) => {
                self.clean_version = version;
                if let Some(link) = &mut self.config_file {
                    link.on_disk = text;
                }
                cx.emit(oxikube_workspace::ItemEvent::UpdateTab);
                Notice::new(NoticeLevel::Info, "Saved")
            }
            Ok((UserKeymapSave::ChangedOnDisk(on_disk), _)) => {
                // The next save overwrites what is there now (and only that).
                if let Some(link) = &mut self.config_file {
                    link.on_disk = on_disk;
                }
                let title = kind.title();
                Notice::new(
                    NoticeLevel::Warning,
                    format!(
                        "Not saved: {title} changed on disk since it was opened (a binding saved \
                         from the Keymap editor?). Save again to overwrite it, or close this tab \
                         and reopen {title} to see the change."
                    ),
                )
            }
            Err(error) => Notice::new(NoticeLevel::Error, format!("Could not save: {error}")),
        };
        self.set_notice(notice, cx);
    }
}
