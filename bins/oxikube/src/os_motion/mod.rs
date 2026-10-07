//! The OS reduce-motion preference, fed to the session (E05-F473).
//!
//! GPUI does not read the accessibility "reduce motion" preference, so with the default
//! `reduce_motion: system` setting nothing would follow the OS. This module is the platform seam
//! that does: an [`OsMotionProbe`] reports the preference as a stream of values, and [`follow`]
//! feeds each one to [`oxikube_workspace::session::set_os_reduce_motion`], which resolves it
//! against the `reduce_motion` setting and writes the result to GPUI's flag.
//!
//! | Platform | Source | Changes |
//! |---|---|---|
//! | macOS | `NSWorkspace.accessibilityDisplayShouldReduceMotion` (`macos` module) | `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification` |
//! | Linux | GNOME `org.gnome.desktop.interface enable-animations` through `gsettings` (`gsettings` module) | `gsettings monitor` |
//! | other | none: the OS value stays `false`, only the `on` override works | none |
//!
//! # Cost
//!
//! Nothing here runs in the init order's stages. The bin calls [`follow`] after the main window is
//! open; the macOS probe reads one property (microseconds) and registers one observer, the Linux
//! probe spawns its `gsettings` processes from a plain thread and the values arrive over a
//! channel, so the UI thread never waits on the OS. The glue is one foreground task that wakes
//! only when the preference changes.

use futures::{StreamExt as _, channel::mpsc};
use gpui::{App, Global, Task};
use oxikube_workspace::session::{os_reduce_motion, set_os_reduce_motion};

#[cfg(any(target_os = "linux", test))]
mod gsettings;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(test)]
mod tests;

/// A running subscription to the OS preference.
pub struct Watch {
    changes: mpsc::UnboundedReceiver<bool>,
    /// Stops the OS-side subscription when dropped (removes the observer, kills the monitor).
    guard: Box<dyn std::any::Any>,
}

impl Watch {
    /// A subscription delivering `true` when the OS asks for reduced motion and `false` when it
    /// does not: the current value first, then every change. `guard` is dropped with the watch.
    pub fn new(changes: mpsc::UnboundedReceiver<bool>, guard: impl std::any::Any) -> Self {
        Self {
            changes,
            guard: Box::new(guard),
        }
    }

    /// A subscription that never reports (a platform without a probe, or one that failed).
    pub fn silent() -> Self {
        let (_, changes) = mpsc::unbounded();
        Self::new(changes, ())
    }
}

/// The platform seam: reads the OS reduce-motion preference and reports its changes.
pub trait OsMotionProbe {
    /// Starts watching. Called on the UI thread, so it must return without waiting on the OS:
    /// anything slow runs on a thread of its own and reports through the [`Watch`].
    fn watch(self: Box<Self>) -> Watch;
}

/// The probe of the running platform ([`Watch::silent`] where there is none).
pub fn system_probe() -> Box<dyn OsMotionProbe> {
    #[cfg(target_os = "macos")]
    return Box::new(macos::Workspace);
    #[cfg(target_os = "linux")]
    return Box::new(gsettings::Gsettings::default());
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return Box::new(Silent);
}

/// The probe of a platform that has none.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
struct Silent;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl OsMotionProbe for Silent {
    fn watch(self: Box<Self>) -> Watch {
        Watch::silent()
    }
}

/// The follower task, held for the life of the app. Dropping it drops the [`Watch`], which
/// stops the OS-side subscription.
struct Follower(#[allow(dead_code)] Task<()>);

impl Global for Follower {}

/// Starts feeding `probe`'s values to the session. Idempotent: a second call does nothing. The
/// subscription ends when the app quits.
pub fn follow(cx: &mut App, probe: Box<dyn OsMotionProbe>) {
    if cx.has_global::<Follower>() {
        return;
    }
    let started = std::time::Instant::now();
    let Watch { mut changes, guard } = probe.watch();
    let task = cx.spawn(async move |cx| {
        let _guard = guard;
        while let Some(reduce) = changes.next().await {
            cx.update(|cx| report(cx, reduce));
        }
    });
    cx.set_global(Follower(task));
    tracing::debug!(
        elapsed_us = started.elapsed().as_micros() as u64,
        "following the OS reduce-motion preference"
    );
    // The Linux monitor is a child process: end it with the app rather than leave it behind.
    cx.on_app_quit(|cx| {
        if cx.has_global::<Follower>() {
            cx.remove_global::<Follower>();
        }
        async {}
    })
    .detach();
}

/// Applies one reported value; a repeat of the current value changes nothing.
fn report(cx: &mut App, reduce: bool) {
    if os_reduce_motion(cx) != reduce {
        tracing::debug!(reduce, "the OS reduce-motion preference changed");
        set_os_reduce_motion(cx, reduce);
    }
}
