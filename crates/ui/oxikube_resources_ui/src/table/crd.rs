//! What a custom resource table adds to the generic one (E07-S07): the version switcher and the
//! note that its columns are the basic ones.
//!
//! * **Versions.** A CRD can serve several versions at once. [`ResourceViews`] tells the table
//!   which ones discovery serves for its kind ([`ResourceTable::set_served_versions`]); with more
//!   than one the tab says which it shows and the toolbar has a switcher. Choosing another sends
//!   `resource::OpenList` for it, so each version is its own tab (side by side, and the same
//!   command a sidebar click sends).
//! * **Basic columns.** A custom resource table is on the Table feed, whose columns are the
//!   server's (`additionalPrinterColumns`). An aggregated API may ignore the Table `Accept`
//!   header and answer with plain objects; the feed then falls back to the generic Name /
//!   Namespace / Age columns, and the toolbar says so ([`ResourceTable::basic_columns`]) instead
//!   of leaving a table that is quietly poorer than `kubectl get`.
//!
//! [`ResourceViews`]: crate::ResourceViews

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::ResourceKind;
use oxikube_ports::TableSource;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::h_flex;
use oxikube_ui::menu::{DropdownMenu as _, PopupMenuItem};
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};
use oxikube_workspace::ItemEvent;

use super::view::{ResourceTable, plural_title};

impl ResourceTable {
    /// The tab's title: the kind's plural, with the version when the kind has several.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The versions of this table's kind the cluster serves, newest first (empty until discovery
    /// answered, and for a kind that is not custom).
    pub fn served_versions(&self) -> &[ResourceKind] {
        &self.served
    }

    /// Whether the version switcher is on: the cluster serves more than one version of the kind.
    pub fn has_version_switcher(&self) -> bool {
        self.served.len() > 1
    }

    /// Whether the feed delivered plain objects instead of the server's Table, so the columns are
    /// the generic ones (Name, Namespace, Age) and not `kubectl get`'s.
    pub fn basic_columns(&self) -> bool {
        self.columns_source == Some(TableSource::Objects)
    }

    /// Takes what discovery serves for the kind. With several versions the tab names the one
    /// shown (two versions of a kind are two tabs).
    pub fn set_served_versions(&mut self, served: Vec<ResourceKind>, cx: &mut Context<Self>) {
        if served == self.served {
            return;
        }
        self.served = served;
        let base = plural_title(&self.kind);
        self.title = if self.has_version_switcher() {
            format!("{base} ({})", self.kind.gvk.version).into()
        } else {
            base.into()
        };
        cx.emit(ItemEvent::UpdateTab);
        cx.notify();
    }

    /// Shows the table of `version` of the kind: sends `resource::OpenList` for it, as a sidebar
    /// click would, which opens (or focuses) that version's tab. Nothing for the version shown.
    pub fn switch_version(&mut self, version: &str, cx: &mut Context<Self>) {
        if &*self.kind.gvk.version == version {
            return;
        }
        let Some(kind) = self.served.iter().find(|k| &*k.gvk.version == version) else {
            return;
        };
        let command = Command::ResourceOpenList {
            cluster: self.cluster.clone(),
            gvk: kind.gvk.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Records where the feed's columns came from (`apply` calls it with each delta that carries
    /// columns).
    pub(super) fn note_columns_source(&mut self, source: TableSource) {
        self.columns_source = Some(source);
    }

    /// The version switcher: the shown version, and a menu of the served ones. `None` when there
    /// is nothing to switch.
    pub(super) fn version_switcher(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        if !self.has_version_switcher() {
            return None;
        }
        let view = cx.entity().downgrade();
        let shown = self.kind.gvk.version.to_string();
        let versions: Vec<(String, Gvk, bool)> = self
            .served
            .iter()
            .map(|k| (k.gvk.version.to_string(), k.gvk.clone(), k.preferred))
            .collect();
        let button = Button::new("resource-table-version")
            .label(format!("Version {shown}"))
            .ghost()
            .xsmall()
            .dropdown_menu(move |mut menu, _, _| {
                for (version, _, preferred) in &versions {
                    let label = if *preferred {
                        format!("{version} (preferred)")
                    } else {
                        version.clone()
                    };
                    let (view, target) = (view.clone(), version.clone());
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(*version == shown)
                            .on_click(move |_, _, cx| {
                                view.update(cx, |table, cx| table.switch_version(&target, cx))
                                    .ok();
                            }),
                    );
                }
                menu
            });
        Some(
            div()
                .debug_selector(|| "resource-table-version".into())
                .child(button),
        )
    }

    /// The note that the columns are the basic ones, with the reason on hover. `None` when the
    /// server's own columns are shown.
    pub(super) fn basic_columns_note(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        if !self.basic_columns() {
            return None;
        }
        let colors = cx.colors();
        let tip = SharedString::from(
            "This API did not answer with a table, so only Name, Namespace and Age are known. \
             kubectl shows the same for it.",
        );
        Some(
            h_flex()
                .id("resource-table-basic-columns")
                .debug_selector(|| "resource-table-basic-columns".into())
                .gap(u(px(4.)))
                .items_center()
                .px(u(px(6.)))
                .h(u(px(20.)))
                .rounded(u(px(10.)))
                .bg(colors.element)
                .border_1()
                .border_color(colors.border_variant)
                .text_size(u(px(11.)))
                .text_color(colors.text_muted)
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .child(Icon::new(IconName::Info).size(u(px(12.))))
                .child("Basic columns"),
        )
    }
}
