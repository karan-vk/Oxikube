//! Command surfaces: the views the command palette asks "what is selected here?".
//!
//! The palette lists the commands that can run where the user is, which needs two facts only the
//! focused view knows: which kind of view it is and which objects it acts on. A view that has
//! such objects (a resource table, the detail drawer) implements [`CommandSurface`] and
//! [`register`]s itself with the focus handle it takes keyboard focus on; the palette then asks
//! [`focused`] when it opens, before it takes the focus away. Views without objects need not
//! register: their key context (`LogView`, `Terminal`, ...) says which view has the focus.
//!
//! A surface may also run some commands through its own flow instead of the bus's generic one
//! ([`CommandSurface::own_commands`], [`CommandSurface::run_command`]): a table opens one delete
//! dialog for its whole selection, with the plan and the propagation choice, where the generic
//! path would send one guarded `resource::Delete` per object. The palette hands those commands
//! back to the surface ([`run_on_focused`]) so it behaves as the table's own key and menu do.
//!
//! Nothing is published while the user works: the registry only holds a weak entity and a focus
//! handle per view, and the view is asked once, when the palette opens. Entries of closed views
//! are dropped on the next registration.

use std::rc::Rc;

use gpui::{App, Context, Entity, FocusHandle, Global, WeakFocusHandle, Window};
use oxikube_app::CommandTarget;
use oxikube_domain::command::{CommandId, ViewContext};
use oxikube_domain::ids::ResourceRef;

/// A view the palette can read the command target of.
pub trait CommandSurface: Sized + 'static {
    /// What kind of view this is, for command availability.
    fn view_context(&self) -> ViewContext;

    /// The cluster, the kind and the objects the view acts on right now.
    fn command_target(&self, cx: &App) -> CommandTarget;

    /// The commands this surface runs through its own flow ([`run_command`](Self::run_command))
    /// when the palette confirms them, read when the palette opens. None by default: every
    /// command goes through the bus.
    fn own_commands(&self, _cx: &App) -> Vec<CommandId> {
        Vec::new()
    }

    /// Runs `command` on `targets` through the surface's own flow. Called for a command listed in
    /// [`own_commands`](Self::own_commands), once the palette has closed and the surface has the
    /// focus back. Returns whether it ran the command; `false` leaves it to the bus.
    fn run_command(
        &mut self,
        _command: CommandId,
        _targets: Vec<ResourceRef>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> bool {
        false
    }
}

/// What a focused surface told the palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceSnapshot {
    /// The kind of view.
    pub view: ViewContext,
    /// What it acts on.
    pub target: CommandTarget,
    /// The commands it runs through its own flow ([`CommandSurface::own_commands`]).
    pub own_commands: Vec<CommandId>,
}

type Reader = Rc<dyn Fn(&App) -> Option<SurfaceSnapshot>>;
type Runner = Rc<dyn Fn(CommandId, Vec<ResourceRef>, &mut Window, &mut App) -> bool>;

#[derive(Clone)]
struct Entry {
    focus: WeakFocusHandle,
    read: Reader,
    run: Runner,
}

#[derive(Default)]
struct Surfaces(Vec<Entry>);

impl Global for Surfaces {}

/// Registers `surface`, which takes keyboard focus on `focus`. The registry holds neither
/// strongly. Call once, when the view is built.
pub fn register<T: CommandSurface>(surface: &Entity<T>, focus: &FocusHandle, cx: &mut App) {
    let weak = surface.downgrade();
    let runner = surface.downgrade();
    let entry = Entry {
        focus: focus.downgrade(),
        read: Rc::new(move |cx| {
            let surface = weak.upgrade()?;
            let surface = surface.read(cx);
            Some(SurfaceSnapshot {
                view: surface.view_context(),
                target: surface.command_target(cx),
                own_commands: surface.own_commands(cx),
            })
        }),
        run: Rc::new(move |command, targets, window, cx| {
            runner.upgrade().is_some_and(|surface| {
                surface.update(cx, |surface, cx| {
                    surface.run_command(command, targets, window, cx)
                })
            })
        }),
    };
    let surfaces = cx.default_global::<Surfaces>();
    surfaces.0.retain(|entry| entry.focus.upgrade().is_some());
    surfaces.0.push(entry);
}

/// The registered surface that has the keyboard focus (or contains it): the one that is itself
/// focused wins over one that merely contains the focus.
fn focused_entry(window: &Window, cx: &App) -> Option<Entry> {
    let surfaces = cx.try_global::<Surfaces>()?;
    let mut containing = None;
    for entry in &surfaces.0 {
        let Some(focus) = entry.focus.upgrade() else {
            continue;
        };
        if focus.is_focused(window) {
            return Some(entry.clone());
        }
        if containing.is_none() && focus.contains_focused(window, cx) {
            containing = Some(entry);
        }
    }
    containing.cloned()
}

/// What the registered surface that has the keyboard focus (or contains it) says, if any.
pub fn focused(window: &Window, cx: &App) -> Option<SurfaceSnapshot> {
    let entry = focused_entry(window, cx)?;
    (entry.read)(cx)
}

/// Has the focused surface run `command` on `targets` through its own flow
/// ([`CommandSurface::run_command`]). `false` when no surface has the focus or it declined: the
/// caller sends the command through the bus.
pub fn run_on_focused(
    command: CommandId,
    targets: Vec<ResourceRef>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Some(entry) = focused_entry(window, cx) else {
        return false;
    };
    (entry.run)(command, targets, window, cx)
}

/// The kind of view the key contexts on the focus path say has the focus, for views that do not
/// register as a [`CommandSurface`]: the innermost of the standard view contexts, else the
/// workspace.
pub fn view_of_focus(window: &Window) -> ViewContext {
    let stack = window.context_stack();
    stack
        .iter()
        .rev()
        .find_map(|context| {
            let primary = context.primary()?.key.as_ref();
            ViewContext::ALL
                .into_iter()
                .find(|view| view.key_context() == primary)
        })
        .unwrap_or(ViewContext::Workspace)
}
