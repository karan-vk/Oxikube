//! Tests of the help overlay (E11-S10).
//!
//! | File | Covers |
//! |---|---|
//! | `model.rs` | pure: grouping, category order, search rows, chips, empty state, 2 000 entries (no window) |
//! | `open.rs` | `?` through the shipped keymap: opens per focused context, not in a text field, closes, focus returns |
//! | `search.rs` | typing filters by title, category, keystroke; keyboard navigation; virtualisation |
//! | `overrides.rs` | a user rebind and `null`, the vim base, marked in the list |
//! | `surface.rs` | the dialog's role and name, the empty state, the unfocused (full) list, the bus command |

mod model;
mod open;
mod overrides;
mod search;
mod surface;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyContext, ParentElement as _, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, div,
};
use oxikube_keymap::{KeyContextBuilder, KeymapOptions, KeymapPlatform, contexts};
use oxikube_ui::IconName;
use oxikube_ui::input::{Input, InputState};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{Item, ItemEvent, TabContent, Workspace};

use super::{HelpHost, HelpModel, HelpOverlay};
use crate::picker::{Picker, PickerDelegate as _};

// The actions the shipped keymaps name, declared under their real names (the crates that own them
// are not linked into this test), so their bindings are live as in the app.
// Only the registration matters (each action's name); the types are never named.
#[allow(dead_code)]
mod declared {
    use gpui::actions;

    actions!(
        resource_table,
        [
            ViewYaml,
            ViewDescribe,
            EditSelected,
            DeleteSelected,
            ViewLogs,
            ShellSelected,
            FocusFilter,
            ToggleWide,
            SelectNext,
            SelectPrevious
        ]
    );
    actions!(
        log_view,
        [
            ToggleWrap,
            ToggleAutoscroll,
            Find,
            TogglePrevious,
            ToggleTimestamps
        ]
    );
    actions!(palette, [OpenJump, Toggle]);
    mod vim {
        use gpui::actions;

        actions!(table, [SelectNext, SelectPrevious]);
    }
}

/// What the probe view pretends to be.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Where {
    /// A pod table with a row selected, no field focused.
    Table,
    /// The log viewer.
    Logs,
    /// A table whose filter field has the focus (`Editing`, with a real text input inside).
    TableFilter,
}

/// A centre item that stands where a cluster tab's view stands: `ClusterTab > <view>` contexts
/// around the focus.
pub struct Probe {
    focus: FocusHandle,
    place: Where,
    input: Entity<InputState>,
}

impl Probe {
    fn new(place: Where, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle().tab_stop(true),
            place,
            input: cx.new(|cx| InputState::new(window, cx)),
        }
    }

    fn view_context(&self) -> KeyContext {
        let (name, editing) = match self.place {
            Where::Table => (contexts::RESOURCE_TABLE, false),
            Where::Logs => (contexts::LOGS, false),
            Where::TableFilter => (contexts::RESOURCE_TABLE, true),
        };
        let mut builder = KeyContextBuilder::new(name);
        builder.flag_if(editing, contexts::EDITING);
        if self.place != Where::Logs {
            builder.value("kind", "Pod").value("selection", "one");
        }
        builder.build()
    }
}

impl gpui::EventEmitter<ItemEvent> for Probe {}

impl Focusable for Probe {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.place == Where::TableFilter {
            self.input.read(cx).focus_handle(cx)
        } else {
            self.focus.clone()
        }
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut tab = KeyContextBuilder::new(contexts::CLUSTER_TAB);
        tab.flag("connected");
        let view = div()
            .id("probe-view")
            .size_full()
            .key_context(self.view_context())
            .when_input(self.place == Where::TableFilter, &self.input)
            .when_focus(self.place != Where::TableFilter, &self.focus);
        div()
            .size_full()
            .key_context(tab.build())
            .child(view.child("probe"))
    }
}

trait ProbeExt: Sized {
    fn when_input(self, on: bool, input: &Entity<InputState>) -> Self;
    fn when_focus(self, on: bool, focus: &FocusHandle) -> Self;
}

impl ProbeExt for gpui::Stateful<gpui::Div> {
    fn when_input(self, on: bool, input: &Entity<InputState>) -> Self {
        if on {
            self.child(Input::new(input))
        } else {
            self
        }
    }

    fn when_focus(self, on: bool, focus: &FocusHandle) -> Self {
        if on { self.track_focus(focus) } else { self }
    }
}

impl Item for Probe {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("probe").icon(IconName::FileText)
    }
}

/// A workspace window with the shipped keymap, the overlay's host installed, and a [`Probe`]
/// focused.
pub struct Fixture {
    pub workspace: Entity<Workspace>,
    pub vcx: VisualTestContext,
    pub probe: Entity<Probe>,
}

impl Fixture {
    pub fn new(cx: &mut TestAppContext, place: Where) -> Self {
        Self::with_keymap(cx, place, "", KeymapOptions::default())
    }

    pub fn with_keymap(
        cx: &mut TestAppContext,
        place: Where,
        user: &str,
        options: KeymapOptions,
    ) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            // As the binary does: the shipped keymap after the component library's own bindings.
            let options = KeymapOptions {
                platform: if cfg!(target_os = "macos") {
                    KeymapPlatform::MacOs
                } else {
                    KeymapPlatform::Linux
                },
                ..options
            };
            oxikube_keymap::init_with_text(user, options, cx);
            crate::help::init(cx);
        });
        let probe = vcx.update(|window, cx| {
            let probe = cx.new(|cx| Probe::new(place, window, cx));
            workspace.update(cx, |ws, cx| ws.open_item(probe.clone(), window, cx));
            let host = std::rc::Rc::new(HelpHost::new(&workspace));
            host.install(window, cx);
            // Keep the host alive for the window: a leaked Rc in a global is the test's owner.
            cx.set_global(KeepHost(host));
            let focus = probe.read(cx).focus_handle(cx);
            focus.focus(window, cx);
            probe
        });
        vcx.run_until_parked();
        let mut fixture = Self {
            workspace,
            vcx,
            probe,
        };
        fixture.draw();
        fixture
    }

    /// Replaces the search text, as if typed.
    pub fn set_query(&mut self, query: &str) {
        let picker = self.picker();
        let query = query.to_owned();
        self.vcx
            .update(|window, cx| picker.update(cx, |p, cx| p.set_query(&query, window, cx)));
        self.settle();
    }

    /// The index of the selected row.
    pub fn selected(&mut self) -> usize {
        let picker = self.picker();
        self.vcx
            .update(|_, cx| picker.read(cx).delegate.selected_index())
    }

    pub fn draw(&mut self) {
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
    }

    pub fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.draw();
    }

    pub fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.settle();
    }

    pub fn type_text(&mut self, text: &str) {
        self.vcx.simulate_input(text);
        self.settle();
    }

    /// The open overlay, if any.
    pub fn overlay(&mut self) -> Option<Entity<HelpOverlay>> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<HelpOverlay>()
        })
    }

    pub fn picker(&mut self) -> Entity<Picker<super::HelpDelegate>> {
        let overlay = self.overlay().expect("the help overlay is open");
        self.vcx.update(|_, cx| overlay.read(cx).picker().clone())
    }

    /// `(category label, title, keystrokes)` of every entry row, in display order, plus the
    /// header labels in order.
    pub fn listing(&mut self) -> Listing {
        let picker = self.picker();
        self.vcx.update(|_, cx| {
            let delegate = &picker.read(cx).delegate;
            let mut out = Listing::default();
            for row in delegate.rows() {
                match row {
                    super::Row::Header { category, .. } => {
                        out.headers.push(category.label().to_owned());
                        out.rows.push(Line::Header);
                    }
                    super::Row::Entry { entry, .. } => {
                        let e = &delegate.model().entries()[*entry];
                        out.rows.push(Line::Entry {
                            keys: e.keystroke_text(),
                            action: e.action,
                        });
                    }
                }
            }
            out
        })
    }

    pub fn focused_is_probe(&mut self) -> bool {
        let probe = self.probe.clone();
        self.vcx
            .update(|window, cx| probe.read(cx).focus_handle(cx).contains_focused(window, cx))
    }

    pub fn model(&mut self) -> HelpModel {
        let picker = self.picker();
        self.vcx
            .update(|_, cx| picker.read(cx).delegate.model().clone())
    }
}

struct KeepHost(#[allow(dead_code)] std::rc::Rc<HelpHost>);

impl gpui::Global for KeepHost {}

#[derive(Default)]
pub struct Listing {
    pub headers: Vec<String>,
    pub rows: Vec<Line>,
}

pub enum Line {
    Header,
    Entry { keys: String, action: &'static str },
}

impl Listing {
    pub fn has(&self, keys: &str, action: &str) -> bool {
        self.rows.iter().any(|row| {
            matches!(row, Line::Entry { keys: k, action: a, .. } if k == keys && *a == action)
        })
    }

    pub fn has_keys(&self, keys: &str) -> bool {
        self.rows
            .iter()
            .any(|row| matches!(row, Line::Entry { keys: k, .. } if k == keys))
    }

    pub fn entry_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row, Line::Entry { .. }))
            .count()
    }
}

impl Probe {
    /// The text typed into the probe's input, when it has one.
    pub fn typed(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }
}
