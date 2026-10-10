//! `#[gpui::test]`s of the picker, through the shipped keymap and the workspace's modal layer.
//!
//! | File | Covers |
//! |---|---|
//! | `keys.rs` | down / up / home / end / page keys, wrapping, unselectable matches, enter, the secondary confirm key, escape and focus return, clicks |
//! | `matching.rs` | typing filters, stale results, confirm while matching, the empty state, virtualisation |

mod keys;
mod matching;

use gpui::{Entity, FocusHandle, Focusable as _, Modifiers, TestAppContext, VisualTestContext};
use oxikube_keymap::KeymapOptions;
use oxikube_workspace::Workspace;
use oxikube_workspace::test_support::{TestItem, open_workspace};

use super::Picker;
use super::test_support::{TestDelegate, TestRecord};

/// A workspace window with the shipped keymap and an item that has the focus.
struct Fixture {
    workspace: Entity<Workspace>,
    vcx: VisualTestContext,
    item_focus: FocusHandle,
}

impl Fixture {
    fn new(cx: &mut TestAppContext) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        // As the binary does: the shipped keymap is bound after the component library, so its
        // `Picker > Input` bindings win ties with the query field's own.
        vcx.update(|_, cx| oxikube_keymap::init_with_text("", KeymapOptions::default(), cx));
        let item_focus = vcx.update(|window, cx| {
            let item = TestItem::build("Pods", cx);
            workspace.update(cx, |ws, cx| ws.open_item(item.clone(), window, cx));
            let focus = item.read(cx).focus_handle(cx);
            focus.focus(window, cx);
            focus
        });
        vcx.run_until_parked();
        Self {
            workspace,
            vcx,
            item_focus,
        }
    }

    /// Opens a picker over `delegate` in the modal layer and returns what the delegate records.
    fn open(&mut self, delegate: TestDelegate) -> TestRecord {
        let record = delegate.record();
        let workspace = self.workspace.clone();
        self.vcx.update(|window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.toggle_modal(window, cx, move |window, cx| {
                    Picker::uniform_list(delegate, window, cx)
                });
            });
        });
        self.settle();
        record
    }

    fn picker(&mut self) -> Option<Entity<Picker<TestDelegate>>> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<Picker<TestDelegate>>()
        })
    }

    fn read<R>(&mut self, f: impl FnOnce(&Picker<TestDelegate>) -> R) -> R {
        let picker = self.picker().expect("the picker is open");
        self.vcx.update(|_, cx| f(picker.read(cx)))
    }

    fn update<R>(
        &mut self,
        f: impl FnOnce(
            &mut Picker<TestDelegate>,
            &mut gpui::Window,
            &mut gpui::Context<Picker<TestDelegate>>,
        ) -> R,
    ) -> R {
        let picker = self.picker().expect("the picker is open");
        self.vcx
            .update(|window, cx| picker.update(cx, |picker, cx| f(picker, window, cx)))
    }

    fn selected(&mut self) -> Option<String> {
        self.read(|picker| picker.delegate.selected_text())
    }

    fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.settle();
    }

    fn type_text(&mut self, text: &str) {
        self.vcx.simulate_input(text);
        self.settle();
    }

    fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.draw();
    }

    fn draw(&mut self) {
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn click(&mut self, selector: &'static str, modifiers: Modifiers) {
        self.draw();
        let at = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"))
            .center();
        self.vcx.simulate_click(at, modifiers);
        self.settle();
    }

    fn item_has_focus(&mut self) -> bool {
        let focus = self.item_focus.clone();
        self.vcx.update(|window, _| focus.is_focused(window))
    }
}

/// The platform's secondary-confirm keystroke in the shipped keymap.
fn secondary_enter() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    }
}
