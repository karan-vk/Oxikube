//! Command surfaces: the views the command palette asks "what is selected here?".
//!
//! The palette lists the commands that can run where the user is, which needs two facts only the
//! focused view knows: which kind of view it is and which objects it acts on. A view that has
//! such objects (a resource table, the detail drawer) implements [`CommandSurface`] and
//! [`register`]s itself with the focus handle it takes keyboard focus on; the palette then asks
//! [`focused`] when it opens, before it takes the focus away. Views without objects need not
//! register: their key context (`LogView`, `Terminal`, ...) says which view has the focus.
//!
//! Nothing is published while the user works: the registry only holds a weak entity and a focus
//! handle per view, and the view is asked once, when the palette opens. Entries of closed views
//! are dropped on the next registration.

use gpui::{App, Entity, FocusHandle, Global, WeakFocusHandle, Window};
use oxikube_app::CommandTarget;
use oxikube_domain::command::ViewContext;

/// A view the palette can read the command target of.
pub trait CommandSurface: 'static {
    /// What kind of view this is, for command availability.
    fn view_context(&self) -> ViewContext;

    /// The cluster, the kind and the objects the view acts on right now.
    fn command_target(&self, cx: &App) -> CommandTarget;
}

/// What a focused surface told the palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceSnapshot {
    /// The kind of view.
    pub view: ViewContext,
    /// What it acts on.
    pub target: CommandTarget,
}

type Reader = Box<dyn Fn(&App) -> Option<SurfaceSnapshot>>;

struct Entry {
    focus: WeakFocusHandle,
    read: Reader,
}

#[derive(Default)]
struct Surfaces(Vec<Entry>);

impl Global for Surfaces {}

/// Registers `surface`, which takes keyboard focus on `focus`. The registry holds neither
/// strongly. Call once, when the view is built.
pub fn register<T: CommandSurface>(surface: &Entity<T>, focus: &FocusHandle, cx: &mut App) {
    let weak = surface.downgrade();
    let entry = Entry {
        focus: focus.downgrade(),
        read: Box::new(move |cx| {
            let surface = weak.upgrade()?;
            let surface = surface.read(cx);
            Some(SurfaceSnapshot {
                view: surface.view_context(),
                target: surface.command_target(cx),
            })
        }),
    };
    let surfaces = cx.default_global::<Surfaces>();
    surfaces.0.retain(|entry| entry.focus.upgrade().is_some());
    surfaces.0.push(entry);
}

/// The registered surface that has the keyboard focus (or contains it), if any. The one that is
/// itself focused wins over one that merely contains the focus.
pub fn focused(window: &Window, cx: &App) -> Option<SurfaceSnapshot> {
    let surfaces = cx.try_global::<Surfaces>()?;
    let mut containing = None;
    for entry in &surfaces.0 {
        let Some(focus) = entry.focus.upgrade() else {
            continue;
        };
        if focus.is_focused(window) {
            return (entry.read)(cx);
        }
        if containing.is_none() && focus.contains_focused(window, cx) {
            containing = Some(entry);
        }
    }
    containing.and_then(|entry| (entry.read)(cx))
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
