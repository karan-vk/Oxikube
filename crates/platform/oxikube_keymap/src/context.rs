//! Key contexts: how views tell the keymap where they are.
//!
//! A binding with `"context": "ResourceTable && !Editing"` is active when the focused element, or an
//! ancestor of it, carries a [`KeyContext`] that satisfies the expression. Views set theirs with
//! `.key_context(...)`; this module keeps the vocabulary uniform so keymap authors can rely on
//! it:
//!
//! | Context | Set by | Flags and values |
//! |---|---|---|
//! | `Workspace` | the workspace root | |
//! | `ClusterTab` | a cluster tab; an ancestor of every view inside it | `connected` once the session is up |
//! | `Dock` | a dock | `position == left\|right\|bottom` |
//! | `Pane` | a pane of the centre group | |
//! | `ResourceTable` | a resource table | `Editing` while its filter field has focus, `selection == none\|one\|many`, `kind` is the listed kind (`Pod`, `Deployment`), `scope == namespaced\|cluster` |
//! | `List` | any other list or tree | `Editing` |
//! | `DetailDrawer` | the resource detail: the drawer of a cluster tab, or its pinned tab (E07-U559) | `mount == drawer\|tab`, `kind` |
//! | `LogView` | the log viewer (E08-S02) | `Editing` while its search field has focus, `searching`, `wrap`, `autoscroll` |
//! | `Terminal` | a terminal pane | `searching` |
//! | `ManifestEditor` | the YAML manifest editor (E10) | `mode == yaml\|diff\|json`, `Editing` |
//! | `Palette` | the command palette (E11-S03) | |
//! | `Picker` | a picker (E11-S02): the palette, the container chooser, ... (its query field is `Picker > Input`) | |
//! | `JumpBar` | the `:` jump bar (E11-S05) | |
//! | `Modal` | a dialog or sheet | |
//! | `Catalog` | the cluster catalog home (E06-S03) | `Editing` while its search field has focus |
//!
//! The stack of a focused table is `Workspace > Dock > ClusterTab > Workspace > Pane >
//! ResourceTable` (each cluster tab hosts a workspace of its own), so a binding in `ClusterTab`
//! reaches every view inside the tab and nothing outside it (the catalog home, settings).
//!
//! The deeper context wins, so the same key can mean different things per view. Bare-letter
//! verbs (k9s's `y`, `d`, `e`, `l`, `s`) are bound only in `ResourceTable` and `DetailDrawer`,
//! always with `!Editing`, so a letter typed into a filter or search field is text. The
//! text-entry contexts (`Terminal`, `ManifestEditor`, `Palette`, `JumpBar`, an `Input`) bind
//! modifier chords only, and never `ctrl-d` in a `Terminal`: it is EOF.
//!
//! A terminal in focus gets every plain `ctrl-` chord (`ctrl-w`, `ctrl-k`, `ctrl-q`, ...): GPUI
//! matches bindings before the focused element sees the key, so a binding in `Workspace` or with
//! no context on one of those keys never reaches the shell. Off macOS use `ctrl-shift-<key>` for
//! application shortcuts, or unbind (`null`) it in a later `Terminal` section (not `!Terminal`: GPUI
//! evaluates a negation false on an empty context stack, so the key would be dead while nothing
//! is focused). The `keymap_shadowing` test in
//! `oxikube_terminal` checks the shipped defaults.
//!
//! Every context built with [`KeyContextBuilder`] also carries `os == macos|linux|windows`, so a
//! section can be limited to one OS (`"context": "ResourceTable && os == macos"`).
//!
//! A view implements [`KeyContextual`] once and uses the result in `render`:
//!
//! ```ignore
//! impl KeyContextual for ResourceTable {
//!     const KEY_CONTEXT: &'static str = contexts::RESOURCE_TABLE;
//!     fn extend_key_context(&self, context: &mut KeyContextBuilder) {
//!         context.flag_if(self.filter_focused, contexts::EDITING);
//!     }
//! }
//! // div().key_context(self.key_context())
//! ```

use gpui::{KeyContext, SharedString};

/// The standard context names (see the table in the [module docs](self)).
pub mod contexts {
    /// The workspace root.
    pub const WORKSPACE: &str = "Workspace";
    /// A cluster tab; an ancestor of every view inside it.
    pub const CLUSTER_TAB: &str = "ClusterTab";
    /// A dock; carries `position`.
    pub const DOCK: &str = "Dock";
    /// A pane of the centre group.
    pub const PANE: &str = "Pane";
    /// A resource table; carries `kind`, `scope` and `selection`.
    pub const RESOURCE_TABLE: &str = "ResourceTable";
    /// Any other list or tree.
    pub const LIST: &str = "List";
    /// The resource detail view: the drawer of a cluster tab, or its pinned tab.
    pub const DETAIL_DRAWER: &str = "DetailDrawer";
    /// The log viewer.
    pub const LOGS: &str = "LogView";
    /// A terminal pane.
    pub const TERMINAL: &str = "Terminal";
    /// The YAML manifest editor.
    pub const MANIFEST_EDITOR: &str = "ManifestEditor";
    /// The command palette.
    pub const PALETTE: &str = "Palette";
    /// A picker (`oxikube_palette::Picker`): a query field over a list of matches.
    pub const PICKER: &str = "Picker";
    /// The `:` jump bar.
    pub const JUMP_BAR: &str = "JumpBar";
    /// A dialog or sheet.
    pub const MODAL: &str = "Modal";
    /// The cluster catalog home.
    pub const CATALOG: &str = "Catalog";
    /// The contexts of the Phase 1 views, the ones the default keymaps define sections for
    /// (E11-S07), in the order of the table in the [module docs](super). A new view that takes
    /// keys adds its name here so the defaults tests cover it.
    pub const PHASE_1: [&str; 9] = [
        WORKSPACE,
        CLUSTER_TAB,
        RESOURCE_TABLE,
        DETAIL_DRAWER,
        LOGS,
        TERMINAL,
        MANIFEST_EDITOR,
        PALETTE,
        JUMP_BAR,
    ];
    /// Flag: a text field inside the context has focus, so bare-letter bindings (vim layer)
    /// must not fire.
    pub const EDITING: &str = "Editing";
}

/// Builds a [`KeyContext`]: a primary name, flags and key/value pairs, plus the `os` value.
#[derive(Clone, Debug)]
pub struct KeyContextBuilder {
    context: KeyContext,
}

impl KeyContextBuilder {
    /// A context named `name` (use the constants in [`contexts`]) with the `os` value set.
    pub fn new(name: impl Into<SharedString>) -> Self {
        let mut context = KeyContext::new_with_defaults();
        context.add(name);
        Self { context }
    }

    /// Add a flag, matched as an identifier (`Editing`, `!Editing`).
    pub fn flag(&mut self, flag: impl Into<SharedString>) -> &mut Self {
        self.context.add(flag);
        self
    }

    /// Add `flag` when `condition` holds.
    pub fn flag_if(&mut self, condition: bool, flag: impl Into<SharedString>) -> &mut Self {
        if condition {
            self.context.add(flag);
        }
        self
    }

    /// Add a key/value pair, matched as `key == value` or `key != value`.
    pub fn value(
        &mut self,
        key: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> &mut Self {
        self.context.set(key, value);
        self
    }

    /// The finished context.
    pub fn build(&self) -> KeyContext {
        self.context.clone()
    }
}

/// A view that declares its key context in one place.
pub trait KeyContextual {
    /// The primary context name, one of [`contexts`] for the shared kinds.
    const KEY_CONTEXT: &'static str;

    /// Add the flags and values that depend on the view's state. The default adds nothing.
    fn extend_key_context(&self, _context: &mut KeyContextBuilder) {}

    /// The context to pass to `.key_context(...)` in `render`.
    fn key_context(&self) -> KeyContext {
        let mut builder = KeyContextBuilder::new(Self::KEY_CONTEXT);
        self.extend_key_context(&mut builder);
        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use gpui::KeyBindingContextPredicate;

    use super::*;
    use oxikube_domain::command::ViewContext;

    /// A command's availability names a view by the keymap context it runs in, so a binding in
    /// `LogView` and a command available in `ViewContext::Logs` agree on where they apply.
    #[test]
    fn every_view_context_is_a_standard_key_context() {
        let standard = [
            contexts::WORKSPACE,
            contexts::CATALOG,
            contexts::RESOURCE_TABLE,
            contexts::DETAIL_DRAWER,
            contexts::LOGS,
            contexts::TERMINAL,
            contexts::MANIFEST_EDITOR,
            contexts::PALETTE,
        ];
        let named: Vec<_> = ViewContext::ALL.iter().map(|v| v.key_context()).collect();
        assert_eq!(named, standard);
    }

    struct Table {
        filtering: bool,
    }

    impl KeyContextual for Table {
        const KEY_CONTEXT: &'static str = contexts::RESOURCE_TABLE;

        fn extend_key_context(&self, context: &mut KeyContextBuilder) {
            context
                .flag_if(self.filtering, contexts::EDITING)
                .value("selection", "one");
        }
    }

    fn matches(expr: &str, context: &KeyContext) -> bool {
        KeyBindingContextPredicate::parse(expr)
            .unwrap()
            .eval(std::slice::from_ref(context))
    }

    #[test]
    fn a_view_declares_its_context_once() {
        let browsing = Table { filtering: false }.key_context();
        assert!(matches("ResourceTable", &browsing));
        assert!(matches("ResourceTable && !Editing", &browsing));
        assert!(matches("selection == one", &browsing));
        assert!(!matches("Pane", &browsing));

        let filtering = Table { filtering: true }.key_context();
        assert!(matches("ResourceTable && Editing", &filtering));
        assert!(!matches("ResourceTable && !Editing", &filtering));
    }

    #[test]
    fn every_context_names_the_os() {
        let context = KeyContextBuilder::new(contexts::WORKSPACE).build();
        let os = if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "linux"
        };
        assert!(matches(&format!("Workspace && os == {os}"), &context));
    }
}
