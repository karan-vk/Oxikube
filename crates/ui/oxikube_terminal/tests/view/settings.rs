//! The `terminal` settings on terminal tabs (E09-S11): a changed shell applies to terminals opened
//! afterwards only, the bell does what the setting says, the cursor blinks on the clock, and a
//! cluster's own block wins for its terminals.

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_settings::SettingsStore;
use oxikube_terminal::TerminalSettings;
use oxikube_terminal::element::BLINK_INTERVAL;
use oxikube_terminal::view::{BackendDescriptor, FLASH_DURATION, LocalLauncher, TerminalView};

use super::*;

fn configure(cx: &mut TestAppContext, user: &str) {
    cx.update(|cx| {
        if !cx.has_global::<SettingsStore>() {
            let store = SettingsStore::new(oxikube_assets::default_settings()).expect("defaults");
            cx.set_global(store);
            oxikube_terminal::init(cx);
        }
        SettingsStore::update_global(cx, |store, _| {
            store.set_user_settings(user).expect("valid settings")
        });
    });
}

fn title(h: &mut Harness, view: &gpui::Entity<TerminalView>) -> String {
    h.vcx.update(|_, cx| view.read(cx).title().to_string())
}

#[gpui::test]
fn a_changed_shell_applies_to_new_terminals_only(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "shell": "/bin/zsh" } }"#);
    let mut h = harness(cx);
    let running = h.open(BackendDescriptor::local(None));
    assert_eq!(title(&mut h, &running), "zsh");

    configure(cx, r#"{ "terminal": { "shell": "/usr/bin/fish" } }"#);
    h.frame();
    assert_eq!(
        title(&mut h, &running),
        "zsh",
        "the running shell is not replaced"
    );
    assert_eq!(h.launches().len(), 1, "and was not restarted");
    let opened_later = h.open(BackendDescriptor::local(None));
    assert_eq!(title(&mut h, &opened_later), "fish", "the next one uses it");
    assert_eq!(h.launches().len(), 2);
}

#[gpui::test]
fn a_cluster_s_terminal_setting_wins_for_its_terminals(cx: &mut TestAppContext) {
    let id = cluster();
    configure(
        cx,
        &format!(
            r#"{{ "terminal": {{ "shell": "/bin/zsh" }},
                 "clusters": {{ "{id}": {{ "terminal": {{ "shell": "/bin/bash" }} }} }} }}"#
        ),
    );
    let mut h = harness(cx);
    let (plain, cluster_shell) = h.vcx.update(|_, cx| {
        (
            TerminalSettings::for_cluster(cx, None).shell,
            TerminalSettings::for_cluster(cx, Some(&id)).shell,
        )
    });
    assert_eq!(plain.as_deref(), Some("/bin/zsh"));
    assert_eq!(cluster_shell.as_deref(), Some("/bin/bash"));
    let local = h.open(BackendDescriptor::local(None));
    assert_eq!(title(&mut h, &local), "zsh");
    let for_cluster = h.open(BackendDescriptor::local(Some(id)));
    assert_eq!(
        title(&mut h, &for_cluster),
        "bash",
        "its tab names the cluster's shell"
    );
}

#[gpui::test]
fn the_visual_bell_flashes_briefly(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "bell": "visual" } }"#);
    let mut h = harness(cx);
    let _view = h.open(BackendDescriptor::local(None));
    h.frame();
    assert!(!h.drawn("terminal-bell"));

    h.backend(0).output("\x07");
    h.frame();
    assert!(h.drawn("terminal-bell"), "the flash shows");
    h.vcx.executor().advance_clock(FLASH_DURATION);
    h.frame();
    assert!(!h.drawn("terminal-bell"), "and goes");
}

#[gpui::test]
fn a_bell_burst_is_one_flash_and_none_means_nothing(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "bell": "none" } }"#);
    let mut h = harness(cx);
    let _view = h.open(BackendDescriptor::local(None));
    h.backend(0).output("\x07\x07\x07");
    h.frame();
    assert!(!h.drawn("terminal-bell"), "bell: none");
    assert_eq!(
        h.vcx.update(|_, cx| _view.read(cx).bells_sounded()),
        0,
        "and no sound either"
    );

    // The setting is read when the bell rings: no reopening needed.
    configure(cx, r#"{ "terminal": { "bell": "visual" } }"#);
    h.backend(0).output("\x07\x07\x07");
    h.frame();
    assert!(h.drawn("terminal-bell"));
    h.vcx.executor().advance_clock(FLASH_DURATION);
    h.frame();
    assert!(!h.drawn("terminal-bell"), "one flash ended all three bells");
}

#[gpui::test]
fn an_audible_bell_flashes_nothing(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "bell": "audible" } }"#);
    let mut h = harness(cx);
    let view = h.open(BackendDescriptor::local(None));
    h.backend(0).output("\x07");
    h.frame();
    assert!(
        !h.drawn("terminal-bell"),
        "the sound is the window's, not a flash"
    );
    assert_eq!(
        h.vcx.update(|_, cx| view.read(cx).bells_sounded()),
        1,
        "the system sound played once"
    );
    h.frame();
    assert_eq!(
        h.vcx.update(|_, cx| view.read(cx).bells_sounded()),
        1,
        "and is not repeated by the next frame"
    );
}

#[gpui::test]
fn a_blinking_cursor_repaints_on_the_clock_only_while_shown(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "cursor_blink": true } }"#);
    let mut h = harness(cx);
    let view = h.open(BackendDescriptor::local(None));
    h.frame();
    h.vcx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(view.read(cx), cx);
        window.focus(&focus, cx);
    });
    h.frame();
    let notifies = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let counter = notifies.clone();
    h.vcx.update(|_, cx| {
        cx.observe(&view, move |_, _| counter.set(counter.get() + 1))
            .detach();
    });
    h.vcx.executor().advance_clock(BLINK_INTERVAL);
    h.vcx.run_until_parked();
    assert_eq!(notifies.get(), 1, "one repaint per half blink");
    h.frame();
    h.vcx.executor().advance_clock(BLINK_INTERVAL);
    h.vcx.run_until_parked();
    assert_eq!(notifies.get(), 2);
}

#[gpui::test]
fn a_cluster_s_shell_reaches_the_process_a_launch_starts(cx: &mut TestAppContext) {
    let id = cluster();
    configure(
        cx,
        &format!(
            r#"{{ "terminal": {{ "shell": "/bin/zsh", "shell_args": ["-l"] }},
                 "clusters": {{ "{id}": {{ "terminal": {{ "shell": "/bin/bash", "shell_args": ["--norc"] }} }} }} }}"#
        ),
    );
    let size = oxikube_ports::TerminalSize::new(80, 24);
    let (plain, clustered, explicit) = cx.update(|cx| {
        (
            LocalLauncher::local_options(&BackendDescriptor::local(None), size, cx).unwrap(),
            LocalLauncher::local_options(&BackendDescriptor::local(Some(id.clone())), size, cx)
                .unwrap(),
            LocalLauncher::local_options(
                &BackendDescriptor::local(Some(id.clone())).with_shell("/bin/fish", vec![]),
                size,
                cx,
            )
            .unwrap(),
        )
    });
    assert_eq!(plain.shell.as_deref(), Some("/bin/zsh"));
    assert_eq!(plain.args, ["-l"]);
    assert_eq!(clustered.shell.as_deref(), Some("/bin/bash"));
    assert_eq!(
        clustered.args,
        ["--norc"],
        "the cluster's own arguments too"
    );
    assert_eq!(
        explicit.shell.as_deref(),
        Some("/bin/fish"),
        "a descriptor's own shell beats both"
    );
}
