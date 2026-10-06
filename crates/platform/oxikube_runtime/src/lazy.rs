//! Services that start on first use, not at start-up (E05-S13, docs/PERFORMANCE.md "Startup").
//!
//! The cold-start budget (≤ 400 ms to the first interactive frame) only holds if nothing heavy runs
//! before that frame. The extension host (WASM engine), cluster discovery, Prometheus detection,
//! the agent registry and the update checker are therefore *not* started by an `init(cx)`: each is
//! a [`LazyService`] whose [`ensure_init`](LazyService::ensure_init) the first code path that needs
//! it calls (opening the extensions page, connecting a cluster, opening the agent panel...).
//!
//! ```ignore
//! pub static EXTENSION_HOST: LazyService<ExtensionHost> =
//!     LazyService::new("extension_host", ExtensionHost::start);
//!
//! // On first use (an action handler, a view opening):
//! let host = EXTENSION_HOST.ensure_init(cx);
//! ```
//!
//! The `init` function runs once per app, on the UI thread, inside a `tracing` span named
//! `lazy_init`, and its cost is kept in the [`LazyServices`] global ([`LazyServices::started`]),
//! which the start-up tests read to prove nothing was started before the first frame. It must stay
//! cheap: build the handle, then put the real work on `spawn_kube` or the background executor
//! (rule 1 of this crate).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use gpui::{App, Global};

/// A service of type `T` built on first use by `init`. See the [module docs](self).
///
/// One `T` per app: the instance is keyed by `T`'s type, so declare each service once (a
/// `static` next to its type is the usual place).
pub struct LazyService<T: 'static> {
    name: &'static str,
    init: fn(&mut App) -> T,
    _marker: PhantomData<fn() -> T>,
}

/// One service that has started: what [`LazyServices::started`] lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedService {
    /// The service's name ([`LazyService::name`]).
    pub name: &'static str,
    /// What its `init` cost on the UI thread.
    pub elapsed: Duration,
}

/// The services started in this app, in the order they started. A GPUI global, installed by the
/// first [`LazyService::ensure_init`].
#[derive(Default)]
pub struct LazyServices {
    instances: HashMap<TypeId, Box<dyn Any>>,
    started: Vec<StartedService>,
    starting: Vec<TypeId>,
}

impl Global for LazyServices {}

impl LazyServices {
    /// The services started so far, oldest first. Empty when none has (or before any
    /// `ensure_init`).
    pub fn started(cx: &App) -> &[StartedService] {
        cx.try_global::<Self>()
            .map(|services| services.started.as_slice())
            .unwrap_or_default()
    }
}

impl<T: 'static> LazyService<T> {
    /// Declares the service `name` (for logs and [`LazyServices::started`]), built by `init` on
    /// first use.
    pub const fn new(name: &'static str, init: fn(&mut App) -> T) -> Self {
        Self {
            name,
            init,
            _marker: PhantomData,
        }
    }

    /// The service's name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Whether the service has started in this app.
    pub fn is_initialised(&self, cx: &App) -> bool {
        self.get(cx).is_some()
    }

    /// The running service, without starting it.
    pub fn get<'a>(&self, cx: &'a App) -> Option<&'a T> {
        cx.try_global::<LazyServices>()?
            .instances
            .get(&TypeId::of::<T>())?
            .downcast_ref::<T>()
    }

    /// The service, started now if this is its first use.
    ///
    /// # Panics
    ///
    /// When `init` (directly or not) asks for the service it is building: a dependency cycle.
    pub fn ensure_init<'a>(&self, cx: &'a mut App) -> &'a T {
        let key = TypeId::of::<T>();
        if !self.is_initialised(cx) {
            let services = cx.default_global::<LazyServices>();
            assert!(
                !services.starting.contains(&key),
                "lazy service `{}` asked for itself while starting",
                self.name
            );
            services.starting.push(key);
            let started = Instant::now();
            let instance =
                tracing::info_span!("lazy_init", service = self.name).in_scope(|| (self.init)(cx));
            let elapsed = started.elapsed();
            tracing::info!(
                service = self.name,
                elapsed_us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX),
                "lazy service started"
            );
            let services = cx.global_mut::<LazyServices>();
            services.starting.retain(|k| *k != key);
            services.instances.insert(key, Box::new(instance));
            services.started.push(StartedService {
                name: self.name,
                elapsed,
            });
        }
        self.get(cx).expect("the service was just started")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static BUILDS: AtomicUsize = AtomicUsize::new(0);

    struct Host(usize);

    static HOST: LazyService<Host> =
        LazyService::new("host", |_| Host(BUILDS.fetch_add(1, Ordering::SeqCst) + 1));

    struct Cyclic;

    static CYCLIC: LazyService<Cyclic> = LazyService::new("cyclic", |cx| {
        CYCLIC.ensure_init(cx);
        Cyclic
    });

    #[gpui::test]
    fn starts_once_on_first_use_and_is_listed(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assert!(!HOST.is_initialised(cx));
            assert!(HOST.get(cx).is_none());
            assert!(LazyServices::started(cx).is_empty(), "nothing at start");

            let first = HOST.ensure_init(cx).0;
            let again = HOST.ensure_init(cx).0;
            assert_eq!(first, again, "built once per app");
            assert!(HOST.is_initialised(cx));
            let started = LazyServices::started(cx);
            assert_eq!(started.len(), 1);
            assert_eq!(started[0].name, "host");
        });
    }

    #[gpui::test]
    #[should_panic(expected = "asked for itself")]
    fn a_cycle_panics_instead_of_recursing(cx: &mut TestAppContext) {
        cx.update(|cx| {
            CYCLIC.ensure_init(cx);
        });
    }
}
