//! Tests of the keymap editor: the list model, the page in a workspace window over a real keymap,
//! `keymap::ShowEditor` with its sidebar entry, and the key recorder with its write-back (E21-S08).

mod command;
mod model;
mod page;
mod recorder;
mod recorder_model;

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{AppContext as _, Entity, Modifiers, Point, TestAppContext, VisualTestContext};
use oxikube_domain::command::Command;
use oxikube_keymap::KeymapOptions;
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, OpenOptions};

use super::{KeymapEditorDeps, KeymapEditorView};

/// Records every command the page sends.
#[derive(Clone, Default)]
pub(super) struct RecordingBus(Rc<RefCell<Vec<Command>>>);

impl RecordingBus {
    pub fn sent(&self) -> Vec<Command> {
        self.0.borrow().clone()
    }
}

impl CommandDispatcher for RecordingBus {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.0.borrow_mut().push(command);
    }
}

pub(super) struct Harness {
    pub vcx: VisualTestContext,
    pub view: Entity<KeymapEditorView>,
    pub bus: RecordingBus,
    pub workspace: Entity<oxikube_workspace::Workspace>,
}

impl Harness {
    /// Opens the page over the shipped default keymap plus `user` as the user's `keymap.json`.
    pub fn open(cx: &mut TestAppContext, user: &str) -> Self {
        let user = user.to_owned();
        Self::open_with(cx, move |cx| {
            oxikube_keymap::init_with_text(&user, KeymapOptions::default(), cx);
        })
    }

    /// Opens the page over the keymap `install` sets up (a config directory, for a write test).
    pub fn open_with(cx: &mut TestAppContext, install: impl FnOnce(&mut gpui::App)) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| install(cx));
        let bus = RecordingBus::default();
        let deps = KeymapEditorDeps {
            dispatcher: Rc::new(bus.clone()),
            workspace: Some(workspace.downgrade()),
        };
        let mut view = None;
        vcx.update(|window, cx| {
            let entity = cx.new(|cx| KeymapEditorView::new(deps, window, cx));
            view = Some(entity.clone());
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(entity), OpenOptions::default(), window, cx);
            });
        });
        let mut harness = Self {
            vcx,
            view: view.expect("built"),
            bus,
            workspace,
        };
        harness.settle();
        harness
    }

    pub fn settle(&mut self) {
        for _ in 0..3 {
            self.vcx.run_until_parked();
        }
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&KeymapEditorView, &gpui::App) -> R) -> R {
        let view = self.view.clone();
        self.vcx.read(|cx| f(view.read(cx), cx))
    }

    /// The titles-and-keys of the rows showing, top to bottom.
    pub fn shown(&mut self) -> Vec<String> {
        self.read(|view, cx| {
            view.table().read(cx, |delegate| {
                let model = delegate.model();
                (0..model.len())
                    .filter_map(|ix| model.row(ix))
                    .map(|row| format!("{} [{}]", row.action, row.keystrokes_text()))
                    .collect()
            })
        })
    }

    pub fn is_laid_out(&mut self, selector: &str) -> bool {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx.debug_bounds(selector).is_some()
    }

    pub fn centre(&mut self, selector: &str) -> Point<gpui::Pixels> {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center()
    }

    pub fn click(&mut self, selector: &str) {
        let at = self.centre(selector);
        self.vcx.simulate_click(at, Modifiers::none());
        self.settle();
    }
}
