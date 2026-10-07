//! [`TerminalServices`]: what every terminal view is built over, kept as an app global so the
//! layout restore (which rebuilds tabs through the `ItemRegistry`) builds them like the commands
//! do.

use std::rc::Rc;

use gpui::{App, Global};
use oxikube_domain::command::Command;
use oxikube_workspace::CommandDispatcher;

use super::launch::TerminalLauncher;
use crate::input::PasteConfirm;

/// What a terminal view needs from the app. Cheap to clone (shared handles).
#[derive(Clone)]
pub struct TerminalServices {
    launcher: Rc<dyn TerminalLauncher>,
    dispatcher: Option<Rc<dyn CommandDispatcher>>,
    paste_confirm: Option<Rc<dyn PasteConfirm>>,
}

impl Global for TerminalServices {}

impl TerminalServices {
    /// Services that start processes with `launcher`; links are not clickable and pastes are not
    /// confirmed until [`with_dispatcher`](Self::with_dispatcher) and
    /// [`with_paste_confirm`](Self::with_paste_confirm).
    pub fn new(launcher: Rc<dyn TerminalLauncher>) -> Self {
        Self {
            launcher,
            dispatcher: None,
            paste_confirm: None,
        }
    }

    /// Where the views send their commands (`terminal::OpenLink`, `terminal::New`): the bus.
    #[must_use]
    pub fn with_dispatcher(mut self, dispatcher: Rc<dyn CommandDispatcher>) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// Where a multi-line paste asks first (`terminal.confirm_multiline_paste`).
    #[must_use]
    pub fn with_paste_confirm(mut self, confirm: Rc<dyn PasteConfirm>) -> Self {
        self.paste_confirm = Some(confirm);
        self
    }

    /// The launcher.
    pub fn launcher(&self) -> &Rc<dyn TerminalLauncher> {
        &self.launcher
    }

    /// The command dispatcher, if one was given.
    pub fn dispatcher(&self) -> Option<&Rc<dyn CommandDispatcher>> {
        self.dispatcher.as_ref()
    }

    /// The paste confirmation, if one was given.
    pub fn paste_confirm(&self) -> Option<&Rc<dyn PasteConfirm>> {
        self.paste_confirm.as_ref()
    }

    /// Sends `command` through the dispatcher; does nothing without one.
    pub fn dispatch(&self, command: Command, cx: &mut App) {
        if let Some(dispatcher) = &self.dispatcher {
            dispatcher.dispatch(command, cx);
        }
    }

    /// The app's services, once [`install`](super::install)ed.
    pub fn try_global(cx: &App) -> Option<Self> {
        cx.try_global::<Self>().cloned()
    }
}
