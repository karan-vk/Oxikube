//! [`HelpEntry`]: one row of the help overlay, derived from a resolved key binding.
//!
//! Pure data and functions (no gpui entities), so the grouping, the titles and the override
//! marking are tested without a window.

use gpui::SharedString;
use oxikube_domain::command::{self, CommandCategory};
use oxikube_keymap::{ActiveBinding, BindingInfo, KeymapLayer, SuppressedBinding, stands_for};

/// The group a binding is listed under: the category of the command it runs (E11-S01's
/// [`CommandCategory`], in its display order), or `Navigation` for a key that only moves a cursor
/// or the focus (`resource_table::SelectNext`), which is no command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HelpCategory {
    /// A command's category.
    Command(CommandCategory),
    /// Keys that run no command: moving the selection, the focus, a pane.
    Navigation,
}

impl HelpCategory {
    /// The heading of the group.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Command(category) => category.label(),
            Self::Navigation => "Navigation",
        }
    }
}

/// Which keymap layer a binding comes from, as the overlay's chip says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HelpSource {
    /// The shipped per-OS defaults, and keys other crates bound in code: no chip.
    Default,
    /// The optional vim base keymap (`Base`).
    Base,
    /// The user's `keymap.json` (`User`): an override.
    User,
}

impl HelpSource {
    /// The source of a binding of `layer` (`None`: bound in code, shipped like a default).
    pub fn of(layer: Option<KeymapLayer>) -> Self {
        match layer {
            Some(KeymapLayer::User) => Self::User,
            Some(KeymapLayer::Vim) => Self::Base,
            Some(KeymapLayer::Default) | None => Self::Default,
        }
    }

    /// The chip's text; `None` for the defaults, which carry no chip.
    pub const fn chip(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Base => Some("Base"),
            Self::User => Some("User"),
        }
    }
}

/// Whether a binding works in the focused view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpState {
    /// The key runs the action here.
    Active,
    /// A default key a `null` of this source unbound ("unbound by you"): it does nothing here.
    Unbound(HelpSource),
}

/// One listed binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpEntry {
    /// The group it is listed under.
    pub category: HelpCategory,
    /// The command's title (`View YAML`), or the action's name made readable (`Select next`).
    pub title: SharedString,
    /// The keystrokes of the sequence, each as `keymap.json` spells it (`ctrl-k`, `ctrl-s`).
    pub keystrokes: Vec<String>,
    /// The action's registered name (`resource_table::ViewYaml`), shown as secondary text.
    pub action: &'static str,
    /// The layer the binding comes from.
    pub source: HelpSource,
    /// Whether it works here.
    pub state: HelpState,
    /// The key context it is limited to (`ResourceTable && !Editing`); `None` when it applies
    /// everywhere. Shown when the list is not limited to the focused view.
    pub context: Option<String>,
}

impl HelpEntry {
    /// The entry of a binding in force.
    pub fn active(binding: &ActiveBinding) -> Self {
        Self::of(binding.action, &binding.binding, HelpState::Active)
    }

    /// The entry of a default binding a `null` hides.
    pub fn unbound(binding: &SuppressedBinding) -> Self {
        let state = HelpState::Unbound(HelpSource::of(Some(binding.by)));
        Self::of(binding.action, &binding.binding, state)
    }

    fn of(action: &'static str, binding: &BindingInfo, state: HelpState) -> Self {
        let (category, title) = describe(action);
        Self {
            category,
            title,
            keystrokes: binding.keystrokes.clone(),
            action,
            source: HelpSource::of(binding.layer),
            state,
            context: binding.context.clone(),
        }
    }

    /// The keystrokes joined by spaces, as written in `keymap.json` (`ctrl-k ctrl-s`).
    pub fn keystroke_text(&self) -> String {
        self.keystrokes.join(" ")
    }

    /// The strings the overlay's search matches, one per field: the title, the category, the
    /// keystrokes (as `keymap.json` spells them), the action's name, and the source in words
    /// (`default`, `user override`, `base vim`, plus `unbound`).
    pub fn fields(&self) -> [String; 5] {
        let mut source = match self.source {
            HelpSource::Default => "default",
            HelpSource::Base => "base vim",
            HelpSource::User => "user override",
        }
        .to_owned();
        if matches!(self.state, HelpState::Unbound(_)) {
            source.push_str(" unbound");
        }
        [
            self.title.to_string(),
            self.category.label().to_owned(),
            self.keystroke_text(),
            self.action.to_owned(),
            source,
        ]
    }

    /// Every field in one string, the title first, for queries of several words.
    pub fn haystack(&self) -> String {
        self.fields().join(" ")
    }
}

/// The category and title for the action `name`: the command it is, or the first command a view
/// action stands for, else the action's own name made readable under `Navigation`.
pub fn describe(name: &'static str) -> (HelpCategory, SharedString) {
    let meta = command::lookup_str(name).or_else(|| {
        stands_for::commands_of_action(name)
            .next()
            .and_then(command::lookup)
    });
    match meta {
        Some(meta) => (HelpCategory::Command(meta.category), meta.title.into()),
        None => (HelpCategory::Navigation, readable(name).into()),
    }
}

/// `table::SelectNext` as `Select next`: the verb after the namespace, split at the capitals.
fn readable(name: &str) -> String {
    let verb = name.rsplit_once("::").map_or(name, |(_, verb)| verb);
    let mut text = String::with_capacity(verb.len() + 4);
    for (ix, ch) in verb.chars().enumerate() {
        if ix == 0 {
            text.extend(ch.to_uppercase());
        } else if ch.is_uppercase() {
            text.push(' ');
            text.extend(ch.to_lowercase());
        } else {
            text.push(ch);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_action_is_titled_by_the_command_it_stands_for() {
        let (category, title) = describe("resource_table::ViewYaml");
        assert_eq!(category, HelpCategory::Command(CommandCategory::Resource));
        assert_eq!(title, "View YAML");
    }

    #[test]
    fn a_command_action_is_its_own_command() {
        let (category, title) = describe("help::Show");
        assert_eq!(category, HelpCategory::Command(CommandCategory::View));
        assert!(!title.is_empty());
    }

    #[test]
    fn a_cursor_key_is_navigation_with_a_readable_title() {
        let (category, title) = describe("resource_table::SelectNext");
        assert_eq!(category, HelpCategory::Navigation);
        assert_eq!(title, "Select next");
        assert_eq!(readable("workspace::ToggleLeftDock"), "Toggle left dock");
    }

    #[test]
    fn navigation_sorts_after_every_command_category() {
        assert!(HelpCategory::Command(CommandCategory::Other) < HelpCategory::Navigation);
        assert!(
            HelpCategory::Command(CommandCategory::App)
                < HelpCategory::Command(CommandCategory::Pod)
        );
    }

    #[test]
    fn layers_map_to_the_chips() {
        assert_eq!(HelpSource::of(Some(KeymapLayer::User)).chip(), Some("User"));
        assert_eq!(HelpSource::of(Some(KeymapLayer::Vim)).chip(), Some("Base"));
        assert_eq!(HelpSource::of(Some(KeymapLayer::Default)).chip(), None);
        assert_eq!(HelpSource::of(None).chip(), None, "bound in code");
    }
}
