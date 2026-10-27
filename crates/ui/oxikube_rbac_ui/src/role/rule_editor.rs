//! [`RuleEditor`]: the inputs of one rule in the dialog.

use gpui::{AppContext as _, Context, Entity, Subscription, Window};
use oxikube_ui::input::{InputEvent, InputState};

use super::dialog::RoleDialog;
use super::draft::{RuleDraft, show_group, show_list, split_groups, split_list};

/// One rule's inputs. The verbs are the dialog's own state (checkboxes), not an input.
pub struct RuleEditor {
    pub(super) api_groups: Entity<InputState>,
    pub(super) resources: Entity<InputState>,
    pub(super) resource_names: Entity<InputState>,
    pub(super) non_resource_urls: Entity<InputState>,
    pub(super) other_verbs: Entity<InputState>,
    /// The verbs checked, in the dialog's order.
    pub(super) verbs: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl RuleEditor {
    /// The inputs for `rule`, filled with it.
    pub(super) fn new(rule: &RuleDraft, window: &mut Window, cx: &mut Context<RoleDialog>) -> Self {
        let input = |text: String,
                     hint: &'static str,
                     window: &mut Window,
                     cx: &mut Context<RoleDialog>| {
            let state = cx.new(|cx| InputState::new(window, cx).placeholder(hint));
            state.update(cx, |state, cx| state.set_value(text, window, cx));
            state
        };
        let groups: Vec<String> = rule
            .api_groups
            .iter()
            .map(|group| show_group(group))
            .collect();
        let api_groups = input(show_list(&groups), "\"\" is the core group", window, cx);
        let resources = input(
            show_list(&rule.resources),
            "pods, deployments, pods/log, *",
            window,
            cx,
        );
        let resource_names = input(show_list(&rule.resource_names), "all objects", window, cx);
        let non_resource_urls = input(
            show_list(&rule.non_resource_urls),
            "/healthz, /metrics",
            window,
            cx,
        );
        let other_verbs = input(
            show_list(&rule.other_verbs()),
            "bind, escalate, use…",
            window,
            cx,
        );
        let mut subscriptions = Vec::new();
        for state in [
            &api_groups,
            &resources,
            &resource_names,
            &non_resource_urls,
            &other_verbs,
        ] {
            subscriptions.push(cx.subscribe_in(state, window, RoleDialog::on_input));
        }
        Self {
            api_groups,
            resources,
            resource_names,
            non_resource_urls,
            other_verbs,
            verbs: rule.verbs.clone(),
            _subscriptions: subscriptions,
        }
    }

    /// The rule as typed.
    pub(super) fn draft(&self, cx: &gpui::App) -> RuleDraft {
        let text = |state: &Entity<InputState>| state.read(cx).value().to_string();
        let mut rule = RuleDraft {
            api_groups: split_groups(&text(&self.api_groups)),
            resources: split_list(&text(&self.resources)),
            verbs: self.verbs.clone(),
            resource_names: split_list(&text(&self.resource_names)),
            non_resource_urls: split_list(&text(&self.non_resource_urls)),
        };
        rule.set_other_verbs(split_list(&text(&self.other_verbs)));
        rule
    }

    /// The text after the last separator of the resources input: what the picker filters on.
    pub(super) fn resource_query(&self, cx: &gpui::App) -> String {
        let text = self.resources.read(cx).value().to_string();
        text.rsplit([',', ' '])
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned()
    }

    /// Replaces the last token of the resources input with `resource`, and adds `group` to the API
    /// groups when it is not there.
    pub(super) fn pick(
        &self,
        group: &str,
        resource: &str,
        window: &mut Window,
        cx: &mut Context<RoleDialog>,
    ) {
        let text = self.resources.read(cx).value().to_string();
        let mut tokens = split_list(&text);
        let query = self.resource_query(cx);
        if !query.is_empty() && tokens.last().is_some_and(|last| *last == query) {
            tokens.pop();
        }
        if !tokens.iter().any(|token| token == resource) {
            tokens.push(resource.to_owned());
        }
        self.resources.update(cx, |state, cx| {
            state.set_value(show_list(&tokens), window, cx)
        });
        let mut groups = split_groups(&self.api_groups.read(cx).value());
        if !groups.iter().any(|own| own == group) {
            groups.push(group.to_owned());
            let shown: Vec<String> = groups.iter().map(|group| show_group(group)).collect();
            self.api_groups.update(cx, |state, cx| {
                state.set_value(show_list(&shown), window, cx)
            });
        }
    }
}

impl RoleDialog {
    /// Any input of a rule changed or got the focus: the resource picker follows the rule being
    /// typed in.
    pub(super) fn on_input(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change | InputEvent::Focus) {
            return;
        }
        let ix = self.rules.iter().position(|rule| rule.resources == *input);
        if ix != self.picking {
            self.picking = ix;
        }
        cx.notify();
    }
}
