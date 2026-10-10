//! [`Outbox`]: what a confirm leaves for the host to run once the palette has closed.

use std::cell::RefCell;
use std::rc::Rc;

use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ResourceRef;

/// One confirmed command, waiting for the palette to close and hand the focus back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The command the user picked.
    pub id: CommandId,
    /// What the bus is sent for it: one `Command` per selected object, as
    /// [`commands_for`](oxikube_app::commands_for) builds them. Empty when the command needs an
    /// operand the palette cannot supply and only the surface's own flow can run it.
    pub commands: Vec<Command>,
    /// `Some(objects)` when the view the palette opened over runs this command through its own
    /// flow ([`CommandSurface::own_commands`](oxikube_workspace::command_surface::CommandSurface::own_commands)):
    /// one delete dialog for the whole selection, a container picker. The host asks that view
    /// first and sends [`commands`](Self::commands) only if it declines.
    pub surface: Option<Vec<ResourceRef>>,
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
