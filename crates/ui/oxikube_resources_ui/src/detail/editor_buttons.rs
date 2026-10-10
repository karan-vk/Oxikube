//! "Edit" and "View YAML in Editor" in the header of a detail (E10-S06): the entry points from
//! the drawer (and from a pinned detail tab) to the manifest editor. Each sends its command
//! (`resource::Edit`, `resource::ViewYamlInEditor`), like the row actions and the palette do; the
//! editor opens a tab titled `kind/name` and reads the object fresh.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div, px,
};
use oxikube_domain::command::Command;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::{Icon, IconName, Sizable as _, u};

use super::view::DetailView;

impl DetailView {
    /// Sends `resource::Edit` for this object: an editable manifest tab.
    pub fn request_edit(&mut self, cx: &mut Context<Self>) {
        let command = Command::ResourceEdit {
            target: self.target.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Sends `resource::ViewYamlInEditor` for this object: a read-only manifest tab.
    pub fn request_view_in_editor(&mut self, cx: &mut Context<Self>) {
        let command = Command::ResourceViewYamlInEditor {
            target: self.target.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// The header's two buttons. Opening an editor only reads the object, so they are there on a
    /// read-only cluster too (an edit tab then opens read-only, with a notice).
    pub(super) fn editor_buttons(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .gap(u(px(2.)))
            .child(
                div()
                    .debug_selector(|| "detail-view-editor".to_owned())
                    .child(
                        Button::new("detail-view-editor")
                            .xsmall()
                            .ghost()
                            .icon(Icon::new(IconName::FileCode).size(u(px(14.))))
                            .tooltip("View YAML in the editor (read-only)")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.request_view_in_editor(cx)),
                            ),
                    ),
            )
            .child(
                div().debug_selector(|| "detail-edit".to_owned()).child(
                    Button::new("detail-edit")
                        .xsmall()
                        .ghost()
                        .icon(Icon::new(IconName::Pencil).size(u(px(14.))))
                        .tooltip("Edit the manifest")
                        .on_click(cx.listener(|this, _, _, cx| this.request_edit(cx))),
                ),
            )
            .into_any_element()
    }
}
