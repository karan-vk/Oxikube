//! The glue against fake probes, and the `gsettings` probe against a stand-in executable.

use futures::channel::mpsc;
use gpui::TestAppContext;
use oxikube_settings::update_user_settings;
use oxikube_workspace::session::{ReduceMotionSetting, SessionSettings, os_reduce_motion};

use super::{OsMotionProbe, Watch, follow, gsettings::parse_animations_enabled};
use crate::startup::{self, StartupEnv};

/// A probe whose values the test sends.
struct Fake(mpsc::UnboundedReceiver<bool>);

impl OsMotionProbe for Fake {
    fn watch(self: Box<Self>) -> Watch {
        Watch::new(self.0, ())
    }
}

fn fake() -> (mpsc::UnboundedSender<bool>, Box<dyn OsMotionProbe>) {
    let (tx, rx) = mpsc::unbounded();
    (tx, Box::new(Fake(rx)))
}

/// The real init order (in memory, fakes for the ports): settings, the workspace's session.
fn start(cx: &mut TestAppContext) {
    cx.update(|cx| startup::init(cx, StartupEnv::test()).expect("the init order runs"));
}

fn effective(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.reduce_motion())
}

fn set_setting(cx: &mut TestAppContext, value: ReduceMotionSetting) {
    cx.update(|cx| {
        update_user_settings::<SessionSettings>(cx, None, move |c| c.reduce_motion = Some(value))
            .detach();
    });
    cx.run_until_parked();
}

#[gpui::test]
fn the_current_value_and_each_change_reach_the_session(cx: &mut TestAppContext) {
    start(cx);
    let (tx, probe) = fake();
    cx.update(|cx| follow(cx, probe));
    assert!(!effective(cx), "nothing reported yet: motion stays on");

    tx.unbounded_send(true).unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| os_reduce_motion(cx)));
    assert!(effective(cx), "`system` follows the OS at start-up");

    tx.unbounded_send(false).unwrap();
    cx.run_until_parked();
    assert!(!effective(cx), "and when the OS changes it back");
}

#[gpui::test]
fn the_setting_still_beats_the_os_value(cx: &mut TestAppContext) {
    start(cx);
    let (tx, probe) = fake();
    cx.update(|cx| follow(cx, probe));
    tx.unbounded_send(true).unwrap();
    cx.run_until_parked();

    set_setting(cx, ReduceMotionSetting::Off);
    assert!(!effective(cx), "off wins over a reducing OS");
    tx.unbounded_send(true).unwrap();
    tx.unbounded_send(false).unwrap();
    cx.run_until_parked();
    assert!(!effective(cx), "OS reports do not override `off`");

    set_setting(cx, ReduceMotionSetting::On);
    assert!(effective(cx), "on wins over a relaxed OS");
    set_setting(cx, ReduceMotionSetting::System);
    assert!(!effective(cx), "system is back to the OS value");
    tx.unbounded_send(true).unwrap();
    cx.run_until_parked();
    assert!(effective(cx));
}

#[gpui::test]
fn repeated_values_and_a_silent_probe_change_nothing(cx: &mut TestAppContext) {
    start(cx);
    let (tx, probe) = fake();
    cx.update(|cx| follow(cx, probe));
    for _ in 0..3 {
        tx.unbounded_send(true).unwrap();
    }
    cx.run_until_parked();
    assert!(effective(cx));

    // A second `follow` is ignored: the first probe stays the source.
    cx.update(|cx| follow(cx, Box::new(Silent)));
    tx.unbounded_send(false).unwrap();
    cx.run_until_parked();
    assert!(!effective(cx));
}

/// A probe that never reports.
struct Silent;

impl OsMotionProbe for Silent {
    fn watch(self: Box<Self>) -> Watch {
        Watch::silent()
    }
}

#[gpui::test]
fn a_silent_probe_leaves_motion_on(cx: &mut TestAppContext) {
    start(cx);
    cx.update(|cx| follow(cx, Box::new(Silent)));
    cx.run_until_parked();
    assert!(!effective(cx));
}

#[gpui::test]
fn the_subscription_ends_when_the_app_quits(cx: &mut TestAppContext) {
    start(cx);
    struct Flag(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (_tx, rx) = mpsc::unbounded();
    let guard = Flag(dropped.clone());
    struct Guarded(mpsc::UnboundedReceiver<bool>, Flag);
    impl OsMotionProbe for Guarded {
        fn watch(self: Box<Self>) -> Watch {
            Watch::new(self.0, self.1)
        }
    }
    cx.update(|cx| follow(cx, Box::new(Guarded(rx, guard))));
    cx.run_until_parked();
    assert!(!dropped.load(std::sync::atomic::Ordering::SeqCst));
    cx.update(|cx| cx.shutdown());
    cx.run_until_parked();
    assert!(
        dropped.load(std::sync::atomic::Ordering::SeqCst),
        "the guard (the observer, the monitor process) is released on quit"
    );
}

#[test]
fn gsettings_output_is_parsed_as_animations_enabled() {
    assert_eq!(parse_animations_enabled("true\n"), Some(true));
    assert_eq!(parse_animations_enabled("false\n"), Some(false));
    assert_eq!(
        parse_animations_enabled("enable-animations: false\n"),
        Some(false)
    );
    assert_eq!(
        parse_animations_enabled("enable-animations: true"),
        Some(true)
    );
    assert_eq!(parse_animations_enabled(""), None);
    assert_eq!(parse_animations_enabled("uint32 3"), None);
}

#[cfg(unix)]
mod stand_in {
    //! The `gsettings` probe's processes, with a shell script in place of `gsettings`.

    use std::{os::unix::fs::PermissionsExt as _, path::Path, time::Duration};

    use futures::StreamExt as _;

    use super::super::gsettings::Gsettings;
    use super::{OsMotionProbe as _, Watch};

    /// A `gsettings` that answers `get` with `current`, and whose `monitor` records its pid, prints
    /// `changes` and then waits.
    fn script(dir: &Path, current: &str, changes: &[&str]) -> std::path::PathBuf {
        let path = dir.join("gsettings");
        let prints: String = changes
            .iter()
            .map(|c| format!("echo 'enable-animations: {c}'\n"))
            .collect();
        let body = format!(
            "#!/bin/sh\ncase \"$1\" in\n get) echo {current};;\n monitor) echo $$ > '{pid}'\n{prints} exec sleep 30;;\nesac\n",
            pid = dir.join("monitor.pid").display()
        );
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn collect(watch: &mut Watch, n: usize) -> Vec<bool> {
        futures::executor::block_on(async {
            let mut out = Vec::new();
            while out.len() < n {
                let next = watch.changes.next();
                let timeout = futures_timer(Duration::from_secs(10));
                match futures::future::select(Box::pin(next), Box::pin(timeout)).await {
                    futures::future::Either::Left((Some(value), _)) => out.push(value),
                    _ => break,
                }
            }
            out
        })
    }

    async fn futures_timer(duration: Duration) {
        let (tx, rx) = futures::channel::oneshot::channel::<()>();
        std::thread::spawn(move || {
            std::thread::sleep(duration);
            let _ = tx.send(());
        });
        let _ = rx.await;
    }

    #[test]
    fn animations_off_means_reduce_and_changes_follow() {
        let dir = tempfile::tempdir().unwrap();
        let program = script(dir.path(), "false", &["true", "false"]);
        let mut watch = Box::new(Gsettings::with_program(program)).watch();
        assert_eq!(collect(&mut watch, 3), [true, false, true]);
    }

    #[test]
    fn dropping_the_watch_kills_the_monitor() {
        let dir = tempfile::tempdir().unwrap();
        let program = script(dir.path(), "true", &[]);
        let mut watch = Box::new(Gsettings::with_program(program)).watch();
        assert_eq!(collect(&mut watch, 1), [false]);
        // The monitor writes its pid as it starts, which may be after `get` answered.
        let pid_file = dir.path().join("monitor.pid");
        let pid = (0..500)
            .find_map(|_| {
                std::thread::sleep(Duration::from_millis(10));
                let pid = std::fs::read_to_string(&pid_file).ok()?;
                (!pid.trim().is_empty()).then(|| pid.trim().to_owned())
            })
            .expect("the monitor wrote its pid");
        let alive = |pid: &str| {
            std::process::Command::new("kill")
                .args(["-0", pid])
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        assert!(alive(&pid), "the monitor runs while the watch lives");
        drop(watch);
        assert!(!alive(&pid), "and is gone when the watch is dropped");
    }

    #[test]
    fn a_missing_gsettings_reports_nothing() {
        let mut watch = Box::new(Gsettings::with_program("/nonexistent/oxikube-gsettings")).watch();
        assert_eq!(collect(&mut watch, 1), Vec::<bool>::new());
    }
}
