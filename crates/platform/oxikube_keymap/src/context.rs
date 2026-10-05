//! Key contexts: how views tell the keymap where they are.
//!
//! A binding with `"context": "Table && !Editing"` is active when the focused element, or an
//! ancestor of it, carries a [`KeyContext`] that satisfies the expression. Views set theirs with
//! `.key_context(...)`; this module keeps the vocabulary uniform so keymap authors can rely on
//! it:
//!
//! | Context | Set by | Flags and values |
//! |---|---|---|
//! | `Workspace` | the workspace root | |
//! | `Dock` | a dock | `position == left\|right\|bottom` |
//! | `Pane` | a pane of the centre group | |
//! | `Table` | a resource table | `Editing` while its filter field has focus, `selection == none\|one\|many` |
//! | `List` | any other list or tree | `Editing` |
//! | `Palette` | the command palette | |
//! | `Modal` | a dialog or sheet | |
//! | `Editor` | the YAML editor | `mode == yaml\|diff`, `Editing` |
//! | `Terminal` | a terminal pane | |
//! | `Logs` | the log viewer | `Editing` while its search field has focus |
//!
//! Every context built with [`KeyContextBuilder`] also carries `os == macos|linux|windows`, so a
//! section can be limited to one OS (`"context": "Table && os == macos"`).
//!
//! A view implements [`KeyContextual`] once and uses the result in `render`:
//!
//! ```ignore
//! impl KeyContextual for ResourceTable {
//!     const KEY_CONTEXT: &'static str = contexts::TABLE;
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
    /// A dock; carries `position`.
    pub const DOCK: &str = "Dock";
    /// A pane of the centre group.
    pub const PANE: &str = "Pane";
    /// A resource table.
    pub const TABLE: &str = "Table";
    /// Any other list or tree.
    pub const LIST: &str = "List";
    /// The command palette.
    pub const PALETTE: &str = "Palette";
    /// A dialog or sheet.
    pub const MODAL: &str = "Modal";
    /// The YAML editor.
    pub const EDITOR: &str = "Editor";
    /// A terminal pane.
    pub const TERMINAL: &str = "Terminal";
    /// The log viewer.
    pub const LOGS: &str = "Logs";
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

    struct Table {
        filtering: bool,
    }

    impl KeyContextual for Table {
        const KEY_CONTEXT: &'static str = contexts::TABLE;

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
        assert!(matches("Table", &browsing));
        assert!(matches("Table && !Editing", &browsing));
        assert!(matches("selection == one", &browsing));
        assert!(!matches("Pane", &browsing));

        let filtering = Table { filtering: true }.key_context();
        assert!(matches("Table && Editing", &filtering));
        assert!(!matches("Table && !Editing", &filtering));
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
