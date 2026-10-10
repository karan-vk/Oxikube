//! The user's `keymap.json` in the real app (E11-S08): problems in the file become one toast with
//! `keymap.json:<line>` for each, and `keymap::OpenUser` creates the file from its template and
//! opens it. Driven over the app's real start-up with a config directory on disk (no watcher: the
//! tests reload explicitly, as the watcher's callback does).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use futures::executor::block_on;
use gpui::{AppContext as _, TestAppContext};
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;
use crate::mount::keymap::KeymapFileOpener;
use crate::startup::ConfigSource;

type Opened = Rc<RefCell<Vec<PathBuf>>>;

/// The app over a config directory `dir`, with the opener of `keymap.json` replaced by a log.
fn start(cx: &mut TestAppContext, dir: &Path) -> (App, Opened) {
    let opened = Opened::default();
    let log = opened.clone();
    let dir = dir.to_owned();
    let app = App::start_with_env(
        cx,
        TestPorts::seeded(),
        move |env| env.config = ConfigSource::Dir(dir),
        move |cx| {
            cx.set_global(KeymapFileOpener(Rc::new(move |path, _| {
                log.borrow_mut().push(path.to_owned());
            })));
        },
    );
    (app, opened)
}

fn toast_messages(app: &mut App) -> Vec<String> {
    app.read(|ws, cx| {
        ws.toast_layer()
            .read(cx)
            .visible()
            .iter()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

fn run(app: &mut App, command: Command) -> bool {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let outcome = block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")));
    app.vcx.run_until_parked();
    outcome.is_ok()
}

const BAD: &str = "[\n  {\"bindings\": {\n    \"ctrl-x\": \"nope::Missing\",\n    \"not-a-real-modifier-\": \"app::Quit\"\n  }}\n]";

#[gpui::test]
fn problems_in_the_file_are_one_toast_with_their_lines_and_a_fix_removes_it(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("keymap.json");
    std::fs::write(&file, BAD).unwrap();
    let (mut app, _) = start(cx, dir.path());

    // The window opened over the start-up load: the toast is there on the first frame.
    let messages = toast_messages(&mut app);
    assert_eq!(messages.len(), 1, "{messages:?}");
    let mut lines = messages[0].lines();
    assert_eq!(
        lines.next(),
        Some("2 problems in keymap.json; the other bindings were applied.")
    );
    assert_eq!(
        lines.next(),
        Some("keymap.json:3: unknown action `nope::Missing` (binding `ctrl-x`)")
    );
    assert!(
        lines
            .next()
            .is_some_and(|l| l.starts_with("keymap.json:4: "))
    );

    // Editing the file to other problems replaces the toast (same key), it does not stack.
    std::fs::write(
        &file,
        "[\n  {\"bindings\": {\"ctrl-y\": \"nope::Other\"}}\n]",
    )
    .unwrap();
    app.vcx.update(|_, cx| oxikube_keymap::reload(cx));
    app.vcx.run_until_parked();
    let messages = toast_messages(&mut app);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(
        messages[0].starts_with("1 problem in keymap.json;"),
        "{}",
        messages[0]
    );

    // Fixing it takes the toast away.
    std::fs::write(&file, "[]").unwrap();
    app.vcx.update(|_, cx| oxikube_keymap::reload(cx));
    app.vcx.run_until_parked();
    assert!(toast_messages(&mut app).is_empty());
}

#[gpui::test]
fn the_toasts_open_keymap_button_opens_the_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("keymap.json"), BAD).unwrap();
    let (mut app, opened) = start(cx, dir.path());

    let id = app.read(|ws, cx| ws.toast_layer().read(cx).visible()[0].id.as_u64());
    app.click(Box::leak(format!("toast-{id}-action-0").into_boxed_str()));
    app.vcx.run_until_parked();
    assert_eq!(*opened.borrow(), [dir.path().join("keymap.json")]);
}

#[gpui::test]
fn open_user_keymap_creates_the_template_once_and_opens_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("keymap.json");
    let (mut app, opened) = start(cx, dir.path());
    assert!(!file.exists());
    assert!(toast_messages(&mut app).is_empty(), "no file, no problem");

    assert!(run(&mut app, Command::KeymapOpenUser));
    assert_eq!(*opened.borrow(), std::slice::from_ref(&file));
    let template = std::fs::read_to_string(&file).unwrap();
    assert_eq!(template, oxikube_assets::initial_user_keymap_content());

    // An existing file is opened as it is.
    std::fs::write(&file, "[]").unwrap();
    assert!(run(&mut app, Command::KeymapOpenUser));
    assert_eq!(opened.borrow().len(), 2);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "[]");
}

#[gpui::test]
fn without_a_config_directory_the_command_says_so(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(!run(&mut app, Command::KeymapOpenUser));
}

#[gpui::test]
fn the_command_is_in_the_apps_bus_with_a_tool_stub_and_a_key_can_run_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, opened) = start(cx, dir.path());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    assert!(
        bus.all()
            .iter()
            .any(|info| info.id() == CommandId::KEYMAP_OPEN_USER)
    );
    assert!(
        bus.tool(CommandId::KEYMAP_OPEN_USER).is_some(),
        "the MCP tool stub"
    );
    assert_eq!(
        CommandId::KEYMAP_OPEN_USER.tool_name(),
        "app.keymap_open_user"
    );

    // A key bound to the action in keymap.json goes through the bus like the palette would.
    app.vcx.update(|_, cx| {
        oxikube_keymap::reload_user_keymap(
            cx,
            r#"[{"bindings": {"ctrl-alt-shift-k": "keymap::OpenUser"}}]"#,
        );
    });
    app.vcx.update(|window, _| window.activate_window());
    app.press("ctrl-alt-shift-k");
    app.vcx.run_until_parked();
    assert_eq!(*opened.borrow(), [dir.path().join("keymap.json")]);
}

/// A dispatcher that only counts what it is asked to run.
struct Counting(Rc<std::cell::Cell<usize>>);

impl oxikube_workspace::CommandDispatcher for Counting {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {
        self.0.set(self.0.get() + 1);
    }
}

/// With a second window mounted after the first, the key pressed in the first (active) window
/// still reaches the first window's handler: the newer window's handler passes it on.
#[gpui::test]
fn a_newer_windows_handler_does_not_shadow_the_active_windows(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, opened) = start(cx, dir.path());
    let ignored = Rc::new(std::cell::Cell::new(0));
    let other: Rc<dyn oxikube_workspace::CommandDispatcher> = Rc::new(Counting(ignored.clone()));
    let second = app.vcx.update(|_, cx| {
        let second = cx
            .open_window(gpui::WindowOptions::default(), |_, cx| {
                cx.new(|_| gpui::Empty)
            })
            .expect("a second window")
            .into();
        crate::mount::keymap::install_action(second, &other, cx);
        second
    });
    let _ = second;
    app.vcx.update(|_, cx| {
        oxikube_keymap::reload_user_keymap(
            cx,
            r#"[{"bindings": {"ctrl-alt-shift-k": "keymap::OpenUser"}}]"#,
        );
    });
    app.vcx.update(|window, _| window.activate_window());
    app.press("ctrl-alt-shift-k");
    app.vcx.run_until_parked();
    assert_eq!(*opened.borrow(), [dir.path().join("keymap.json")]);
    assert_eq!(
        ignored.get(),
        0,
        "the inactive window's dispatcher stayed idle"
    );
}
