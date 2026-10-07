//! The `logs` settings of the view's cluster (E08-S10): read when the view is built, and applied
//! live when they change.
//!
//! `wrap`, `timestamps` and `json_auto_detect` (JSON mode, E08-S05) are the view's own options with the setting as
//! their starting value, so a change of the setting moves them in every open view without
//! reopening the stream (a view the user toggled is moved too: the settings file is the newer
//! word). `default_tail` is the length the tail range reads, so it takes effect at the next read
//! of the tail; `buffer_lines` is the service's ([`follow_settings`](crate::follow_settings)).
//! The observer is woken only when the resolved `logs` value changes, and one change redraws the
//! view once.

use gpui::Context;

use super::LogView;
use crate::LogsSettings;

impl LogView {
    /// Applies the keys of the `logs` settings that changed since the last time.
    pub(crate) fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let new = LogsSettings::resolve(&self.target.cluster, cx);
        let old = std::mem::replace(&mut self.settings, new);
        if new.wrap != old.wrap {
            self.switch_wrap(new.wrap, cx);
        }
        if new.timestamps != old.timestamps {
            self.set_timestamps(new.timestamps, cx);
        }
        if new.json_auto_detect != old.json_auto_detect {
            self.set_json_mode(new.json_auto_detect, cx);
        }
        if new.default_tail != old.default_tail {
            self.options.default_tail = new.default_tail;
        }
    }
}
