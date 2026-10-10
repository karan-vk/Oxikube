//! [`Outbox`]: what a confirm leaves for the host to run once the palette has closed.

use std::cell::RefCell;
use std::rc::Rc;

use oxikube_app::{CommandTarget, commands_for};
use oxikube_domain::command::{Command, CommandId};

/// One confirmed command, waiting for the palette to close and hand the focus back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The command the user picked.
    pub id: CommandId,
    /// What the bus is sent for it: one `Command` per selected object, as
    /// [`commands_for`](oxikube_app::commands_for) builds them. Empty when
    /// [`surface`](Self::surface) is set: those are built by [`fallback`](Self::fallback) only if
    /// the view declines, so a select-all of a big table costs nothing on confirm.
    pub commands: Vec<Command>,
    /// `Some(target)` when the view the palette opened over runs this command through its own
    /// flow ([`CommandSurface::own_commands`](oxikube_workspace::command_surface::CommandSurface::own_commands)):
    /// one delete dialog for the whole selection (`target.targets`), a container picker. The host
    /// asks that view first and sends the [`fallback`](Self::fallback) commands only if it
    /// declines.
    pub surface: Option<CommandTarget>,
}

impl Launch {
    /// A command the bus runs: `commands` are ready.
    pub fn on_bus(id: CommandId, commands: Vec<Command>) -> Self {
        Self {
            id,
            commands,
            surface: None,
        }
    }

    /// A command the view runs through its own flow, over `target`; nothing is built yet.
    pub fn on_surface(id: CommandId, target: CommandTarget) -> Self {
        Self {
            id,
            commands: Vec::new(),
            surface: Some(target),
        }
    }

    /// What the bus is sent when the view declined: the commands built at confirm, else those
    /// [`commands_for`] makes from the view's target (none when it needs an operand the palette
    /// cannot supply: only the view's own flow can run it).
    pub fn fallback(self) -> Vec<Command> {
        if !self.commands.is_empty() {
            return self.commands;
        }
        self.surface
            .and_then(|target| commands_for(self.id, &target).ok())
            .unwrap_or_default()
    }
}

#[derive(Default)]
struct Inner {
    own: Vec<CommandId>,
    launches: Vec<Launch>,
}

/// The commands a confirm leaves for the host ([`PaletteHost`](super::PaletteHost)), and the
/// commands the focused view runs itself. Cheap to clone: the palette and the host share one.
#[derive(Clone, Default)]
pub struct Outbox(Rc<RefCell<Inner>>);

impl Outbox {
    /// An outbox for a palette opened over a view that runs `own` through its own flow.
    pub fn for_surface(own: Vec<CommandId>) -> Self {
        Self(Rc::new(RefCell::new(Inner {
            own,
            launches: Vec::new(),
        })))
    }

    /// Whether the view the palette opened over runs `id` through its own flow.
    pub fn runs_on_surface(&self, id: CommandId) -> bool {
        self.0.borrow().own.contains(&id)
    }

    /// Leaves `launch` for the host.
    pub fn push(&self, launch: Launch) {
        self.0.borrow_mut().launches.push(launch);
    }

    /// Takes what was left, in order.
    pub fn take(&self) -> Vec<Launch> {
        std::mem::take(&mut self.0.borrow_mut().launches)
    }
}
