//! How a status looks: its icon and colour.

use gpui::Hsla;
use oxikube_app::apply::ObjectStatus;
use oxikube_ui::{Colors, IconName};

/// A table row's height before the UI zoom: uniform, as the virtualised list needs.
pub const ROW_HEIGHT: f32 = 24.0;

/// The icon and colour of `status`; `None` is an object still waiting for an answer.
pub fn status_look(status: Option<&ObjectStatus>, colors: &Colors) -> (IconName, Hsla) {
    match status {
        None => (IconName::Circle, colors.text_disabled),
        Some(ObjectStatus::Created) => (IconName::Plus, colors.success),
        Some(ObjectStatus::Configured) => (IconName::Pencil, colors.accent),
        Some(ObjectStatus::Unchanged) => (IconName::Check, colors.text_muted),
        Some(ObjectStatus::DeferredDryRun) => (IconName::Clock, colors.warning),
        Some(ObjectStatus::NoPreviewAvailable) => (IconName::Info, colors.info),
        Some(ObjectStatus::Failed { .. }) => (IconName::CircleX, colors.error),
        Some(ObjectStatus::Skipped { .. }) => (IconName::TriangleAlert, colors.warning),
    }
}

/// The one-line message of a status, the table's Message column.
pub fn status_message(status: Option<&ObjectStatus>) -> String {
    match status {
        Some(ObjectStatus::Failed { reason, .. } | ObjectStatus::Skipped { reason }) => {
            reason.lines().next().unwrap_or_default().to_owned()
        }
        Some(ObjectStatus::DeferredDryRun) => {
            "needs a Namespace or CRD the plan creates: rehearsed after it".to_owned()
        }
        Some(ObjectStatus::NoPreviewAvailable) => {
            "the server cannot dry-run this kind: applied without a preview".to_owned()
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use oxikube_ui::Tokens;

    use super::*;

    #[test]
    fn every_status_has_its_own_icon() {
        let colors = Tokens::dark().colors;
        let failed = ObjectStatus::Failed {
            reason: "no".into(),
            fields: Vec::new(),
            conflicts: Vec::new(),
        };
        let skipped = ObjectStatus::Skipped { reason: "x".into() };
        let all = [
            None,
            Some(&ObjectStatus::Created),
            Some(&ObjectStatus::Configured),
            Some(&ObjectStatus::Unchanged),
            Some(&ObjectStatus::DeferredDryRun),
            Some(&ObjectStatus::NoPreviewAvailable),
            Some(&failed),
            Some(&skipped),
        ];
        let icons: Vec<IconName> = all.iter().map(|s| status_look(*s, &colors).0).collect();
        for (i, a) in icons.iter().enumerate() {
            for b in &icons[i + 1..] {
                assert_ne!(a, b, "two statuses share an icon");
            }
        }
    }

    #[test]
    fn a_failure_shows_its_first_line_in_the_table() {
        let failed = ObjectStatus::Failed {
            reason: "first\nsecond".into(),
            fields: Vec::new(),
            conflicts: Vec::new(),
        };
        assert_eq!(status_message(Some(&failed)), "first");
        assert_eq!(status_message(Some(&ObjectStatus::Created)), "");
    }
}
