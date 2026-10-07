//! The toolbar's Sources menu: every pod (and, when a pod has several, every container) a
//! multi-pod view reads, checked while its lines are shown. A click sends `logs::ToggleSource`;
//! the source keeps streaming while it is off.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _, div,
};
use oxikube_ui::Sizable as _;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::Disableable as _;
use oxikube_ui::menu::{DropdownMenu as _, PopupMenuItem};

use crate::view::LogView;

/// One line of the menu: what it says and what a click switches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceChoice {
    /// The pod.
    pub pod: String,
    /// The container, or `None` for the whole pod.
    pub container: Option<String>,
    /// What the menu says: the pod, or `pod / container` indented under it.
    pub label: String,
    /// Whether the lines are shown.
    pub shown: bool,
}

impl LogView {
    /// The menu's lines: each pod, then its containers when it has more than one.
    pub fn source_choices(&self) -> Vec<SourceChoice> {
        let Some(state) = &self.aggregate else {
            return Vec::new();
        };
        let mut pods: Vec<&str> = Vec::new();
        for source in &state.sources {
            if !pods.contains(&&*source.pod) {
                pods.push(&source.pod);
            }
        }
        let mut choices = Vec::new();
        for pod in pods {
            choices.push(SourceChoice {
                pod: pod.to_owned(),
                container: None,
                label: pod.to_owned(),
                shown: !state.hidden.is_pod_hidden(pod),
            });
            let containers: Vec<&str> = state
                .sources
                .iter()
                .filter(|s| &*s.pod == pod)
                .map(|s| &*s.container)
                .collect();
            if containers.len() > 1 {
                for container in containers {
                    choices.push(SourceChoice {
                        pod: pod.to_owned(),
                        container: Some(container.to_owned()),
                        label: format!("    {container}"),
                        shown: !state.hidden.is_hidden(pod, container),
                    });
                }
            }
        }
        choices
    }

    /// The Sources button and its menu.
    pub(crate) fn sources_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let choices = self.source_choices();
        let hidden = choices
            .iter()
            .filter(|c| c.container.is_none() && !c.shown)
            .count();
        let label = if hidden == 0 {
            format!(
                "Sources ({})",
                choices.iter().filter(|c| c.container.is_none()).count()
            )
        } else {
            format!("Sources ({hidden} off)")
        };
        let disabled = choices.is_empty();
        let view = cx.entity().downgrade();
        let button = Button::new("log-sources")
            .label(label)
            .ghost()
            .xsmall()
            .disabled(disabled)
            .dropdown_menu(move |mut menu, _, _| {
                for choice in &choices {
                    let view = view.clone();
                    let (pod, container) = (choice.pod.clone(), choice.container.clone());
                    menu = menu.item(
                        PopupMenuItem::new(choice.label.clone())
                            .checked(choice.shown)
                            .on_click(move |_, _, cx| {
                                view.update(cx, |view, cx| {
                                    view.request_toggle_source(&pod, container.as_deref(), cx);
                                })
                                .ok();
                            }),
                    );
                }
                menu
            });
        div()
            .debug_selector(|| "log-sources".into())
            .child(button)
            .into_any_element()
    }
}
