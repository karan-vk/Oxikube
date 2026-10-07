//! The GNOME probe: `org.gnome.desktop.interface enable-animations` through `gsettings`.
//!
//! GNOME's "Animations" switch is the Linux reduce-motion preference: off means reduced motion.
//! Both reads go through the `gsettings` command (it talks to dconf or the settings portal, as
//! the desktop is configured) on a thread of this module, never on the UI thread: `gsettings
//! monitor` is spawned first so no change is missed, then `gsettings get` reports the current
//! value, then each line the monitor prints is a change. Without `gsettings` (no GNOME, a
//! sandbox) the thread ends and nothing is ever reported, so only the `on` setting works.

use std::{
    ffi::OsString,
    io::{BufRead as _, BufReader},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, MutexGuard},
};

use futures::channel::mpsc::UnboundedSender;

use super::{OsMotionProbe, Watch};

const SCHEMA: &str = "org.gnome.desktop.interface";
const KEY: &str = "enable-animations";

/// Runs `program` (`gsettings`) for the animations key.
pub(super) struct Gsettings {
    program: OsString,
}

impl Default for Gsettings {
    fn default() -> Self {
        Self {
            program: "gsettings".into(),
        }
    }
}

#[cfg(test)]
impl Gsettings {
    /// Uses another executable with `gsettings`' command line.
    pub(super) fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }
}

/// Where the monitor process is, shared between the reader thread and the [`Watch`]'s guard.
enum Monitor {
    Starting,
    Running(Child),
    Stopped,
}

/// Ends the monitor process when the watch is dropped.
struct Stop(Arc<Mutex<Monitor>>);

fn lock(monitor: &Mutex<Monitor>) -> MutexGuard<'_, Monitor> {
    monitor.lock().unwrap_or_else(|e| e.into_inner())
}

impl Drop for Stop {
    fn drop(&mut self) {
        if let Monitor::Running(mut child) =
            std::mem::replace(&mut *lock(&self.0), Monitor::Stopped)
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl OsMotionProbe for Gsettings {
    fn watch(self: Box<Self>) -> Watch {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let monitor = Arc::new(Mutex::new(Monitor::Starting));
        let shared = monitor.clone();
        let spawned = std::thread::Builder::new()
            .name("os-reduce-motion".into())
            .spawn(move || run(&self.program, &tx, &shared));
        if let Err(err) = spawned {
            tracing::debug!(%err, "cannot start the reduce-motion thread");
        }
        Watch::new(rx, Stop(monitor))
    }
}

/// The thread body: start the monitor, report the current value, then every change.
fn run(program: &OsString, tx: &UnboundedSender<bool>, monitor: &Arc<Mutex<Monitor>>) {
    let spawned = Command::new(program)
        .args(["monitor", SCHEMA, KEY])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(err) => {
            tracing::debug!(%err, "gsettings is not available: the OS reduce-motion preference is not read");
            return;
        }
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
    {
        let mut slot = lock(monitor);
        if matches!(*slot, Monitor::Stopped) {
            // The watch was dropped while the process started.
            drop(slot);
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        *slot = Monitor::Running(child);
    }

    let current = Command::new(program)
        .args(["get", SCHEMA, KEY])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    if let Ok(output) = current
        && let Some(enabled) = parse_animations_enabled(&String::from_utf8_lossy(&output.stdout))
        && tx.unbounded_send(!enabled).is_err()
    {
        return;
    }

    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(enabled) = parse_animations_enabled(&line)
            && tx.unbounded_send(!enabled).is_err()
        {
            break;
        }
    }
    // End of output (the watch dropped and killed it, or gsettings quit) or nobody listening.
    drop(Stop(monitor.clone()));
}

/// Whether animations are enabled, from `gsettings get` (`true`) or one `gsettings monitor` line
/// (`enable-animations: false`).
pub(super) fn parse_animations_enabled(line: &str) -> Option<bool> {
    match line.rsplit(':').next()?.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}
