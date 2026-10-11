//! Hover docs and completion in the running editor (E10-S05): a manifest editor in a workspace,
//! typed into with the Pod schema served by a fake `SchemaPort`. Typing opens the completion
//! menu after the pause, Enter accepts, `ctrl-space` opens it through the bus, and the docs of
//! the token under the cursor appear (`ctrl-shift-space`, or the pointer).

#[path = "assist/common.rs"]
mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, Entity, TestAppContext, VisualTestContext, WeakEntity, Window};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_editor::view::{
    EditorHost, EditorRequest, EditorViewSink, EditorViews, EditorViewsDeps, ManifestEditor,
    SchemaSource, VALIDATION_DEBOUNCE,
};
use oxikube_ports::SchemaPort;
use oxikube_testkit::fakes::FakeSchemaPort;
use oxikube_ui::editor::{COMPLETION_DEBOUNCE, CodeEditor, EditorApi as _};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};

fn cluster() -> ClusterId {
    ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"))
}

struct SinkDispatcher {
    sink: EditorViewSink,
    sent: Rc<RefCell<Vec<Command>>>,
}

impl CommandDispatcher for SinkDispatcher {
    fn dispatch(&self, command: Command, _: &mut App) {
        self.sent.borrow_mut().push(command.clone());
        if let Some(request) = EditorRequest::of(command) {
            self.sink.send(request).expect("the window is open");
        }
    }
}

struct TestSchemas(Arc<FakeSchemaPort>);

impl SchemaSource for TestSchemas {
    fn cluster(&self) -> &ClusterId {
        static CLUSTER: std::sync::OnceLock<ClusterId> = std::sync::OnceLock::new();
        CLUSTER.get_or_init(cluster)
    }
    fn label(&self, _: &App) -> gpui::SharedString {
        "kind-oxikube".into()
    }
    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        Some(self.0.clone())
    }
}

struct TestHost {
    workspace: WeakEntity<Workspace>,
    port: Arc<FakeSchemaPort>,
}

impl EditorHost for TestHost {
    fn active_cluster(&self, _: &App) -> Option<ClusterId> {
        Some(cluster())
    }
    fn workspace(&self, _: &ClusterId, _: &App) -> Option<Entity<Workspace>> {
        self.workspace.upgrade()
    }
    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut App) {}
    fn schemas(&self, _: &ClusterId) -> Rc<dyn SchemaSource> {
        Rc::new(TestSchemas(self.port.clone()))
    }
}

struct Fixture {
    vcx: VisualTestContext,
    sent: Rc<RefCell<Vec<Command>>>,
    editor: Entity<ManifestEditor>,
    _views: Entity<EditorViews>,
}

fn setup(cx: &mut TestAppContext) -> Fixture {
    let (workspace, mut vcx) = open_workspace(cx);
    let port = Arc::new(FakeSchemaPort::new());
    port.insert(
        cluster(),
        oxikube_domain::ids::Gvk::new("", "v1", "Pod"),
        common::pod_schema(),
    );
    let sent = Rc::new(RefCell::new(Vec::new()));
    let (sink, requests) = EditorViewSink::channel();
    let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(SinkDispatcher {
        sink,
        sent: sent.clone(),
    });
    let host = Rc::new(TestHost {
        workspace: workspace.downgrade(),
        port,
    });
    let views = vcx.update(|window, cx| {
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
        oxikube_editor::view::install(dispatcher.clone(), cx);
        oxikube_keymap::init_with_text("", oxikube_keymap::KeymapOptions::default(), cx);
        let deps = EditorViewsDeps {
            host,
            window_workspace: workspace.downgrade(),
            dispatcher,
        };
        EditorViews::start(deps, requests, window, cx)
    });
    vcx.run_until_parked();
    let before = vcx.read(|cx| workspace.read(cx).items_of_type::<ManifestEditor>());
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-shift-e"
    } else {
        "ctrl-shift-e"
    });
    vcx.run_until_parked();
    let editor = vcx
        .read(|cx| workspace.read(cx).items_of_type::<ManifestEditor>())
        .into_iter()
        .find(|e| !before.contains(e))
        .expect("an editor opened");
    Fixture {
        vcx,
        sent,
        editor,
        _views: views,
    }
}

impl Fixture {
    /// Types `text` the way a user does and lets the pause, the schema fetch and the first
    /// validation pass.
    fn type_text(&mut self, text: &str) {
        self.vcx.simulate_input(text);
        self.settle();
    }

    fn settle(&mut self) {
        self.vcx
            .executor()
            .advance_clock(VALIDATION_DEBOUNCE.max(COMPLETION_DEBOUNCE) * 2);
        self.vcx.run_until_parked();
    }

    fn code(&mut self) -> Entity<oxikube_ui::editor::CodeEditor> {
        self.vcx.read(|cx| self.editor.read(cx).editor().clone())
    }

    fn text(&mut self) -> String {
        let code = self.code();
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| code.api(window, cx).text().to_string())
        })
    }

    fn menu(&mut self) -> Vec<String> {
        let code = self.code();
        self.vcx.read(|cx| code.read(cx).open_completions(cx))
    }

    fn hover(&mut self) -> Option<(std::ops::Range<usize>, String)> {
        let code = self.code();
        self.vcx.read(|cx| code.read(cx).open_hover(cx))
    }

    fn select(&mut self, at: usize) {
        let code = self.code();
        self.vcx
            .update(|window, cx| code.update(cx, |code, cx| code.api(window, cx).select(at..at)));
        self.vcx.run_until_parked();
    }
}

fn complete_key() -> &'static str {
    "ctrl-space"
}

fn hover_key() -> &'static str {
    "ctrl-shift-space"
}

const POD: &str = "apiVersion: v1\nkind: Pod\nspec:\n  containers:\n    - name: web\n      ";

#[gpui::test]
fn typing_a_field_opens_the_menu_and_enter_inserts_it(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    // Schema fetched first (the editor asks on the first validation).
    f.type_text(POD);
    assert!(f.menu().is_empty(), "no menu until a word is typed");

    f.type_text("ima");
    assert_eq!(f.menu(), ["image", "imagePullPolicy"]);

    f.vcx.simulate_keystrokes("enter");
    f.vcx.run_until_parked();
    assert_eq!(f.text(), format!("{POD}image: "));
    assert!(f.menu().is_empty(), "accepting closes the menu");
}

#[gpui::test]
fn the_menu_follows_the_word_and_closes_when_nothing_matches(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.type_text(POD);
    f.type_text("ima");
    assert_eq!(f.menu().len(), 2);
    f.type_text("gePu");
    assert_eq!(f.menu(), ["imagePullPolicy"]);
    f.type_text("x");
    assert!(f.menu().is_empty());
}

#[gpui::test]
fn no_menu_while_typing_in_a_comment(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.type_text(POD);
    f.type_text("# ima");
    assert!(f.menu().is_empty());
}

#[gpui::test]
fn ctrl_space_opens_the_menu_at_the_cursor_through_the_bus(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.type_text(POD);
    assert!(f.menu().is_empty());
    f.vcx.simulate_keystrokes(complete_key());
    f.settle();
    assert!(f.sent.borrow().contains(&Command::EditorTriggerCompletion));
    let menu = f.menu();
    assert!(
        menu.contains(&"image".to_owned()) && menu.contains(&"ports".to_owned()),
        "{menu:?}"
    );
    assert!(
        !menu.contains(&"name".to_owned()),
        "name is already there: {menu:?}"
    );
    assert!(
        !menu.contains(&"container".to_owned()),
        "a blank line is not an item start: {menu:?}"
    );
}

#[gpui::test]
fn ctrl_space_at_the_start_of_a_list_item_offers_the_container_block_first(
    cx: &mut TestAppContext,
) {
    let mut f = setup(cx);
    f.type_text("apiVersion: v1\nkind: Pod\nspec:\n  containers:\n    - ");
    f.vcx.simulate_keystrokes(complete_key());
    f.settle();
    let menu = f.menu();
    assert_eq!(menu[0], "container", "{menu:?}");
    assert_eq!(menu[1], "name", "required fields lead the rest: {menu:?}");
    f.vcx.simulate_keystrokes("enter");
    f.vcx.run_until_parked();
    let text = f.text();
    assert!(
        text.ends_with("    - name: app\n      image: nginx:latest\n      ports:\n        - containerPort: 80\n      resources:\n        requests:\n          cpu: 100m\n          memory: 128Mi\n        limits:\n          cpu: 500m\n          memory: 256Mi"),
        "{text}"
    );
}

#[gpui::test]
fn escape_closes_the_menu_and_changes_nothing_else(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.type_text(POD);
    f.type_text("ima");
    assert!(!f.menu().is_empty());
    f.vcx.simulate_keystrokes("escape");
    f.vcx.run_until_parked();
    assert!(f.menu().is_empty());
    assert_eq!(f.text(), format!("{POD}ima"));
}

#[gpui::test]
fn ctrl_space_after_a_colon_offers_the_enum_values(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.type_text("apiVersion: v1\nkind: Pod\nspec:\n  restartPolicy: ");
    f.vcx.simulate_keystrokes(complete_key());
    f.settle();
    assert_eq!(f.menu(), ["Always", "OnFailure", "Never"]);
    f.vcx.simulate_keystrokes("down down enter");
    f.vcx.run_until_parked();
    assert_eq!(
        f.text(),
        "apiVersion: v1\nkind: Pod\nspec:\n  restartPolicy: Never"
    );
}

#[gpui::test]
fn the_show_hover_key_shows_the_docs_of_the_token_at_the_cursor(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.type_text("apiVersion: v1\nkind: Pod\nspec:\n  restartPolicy: Never\n");
    let at = f.text().find("restartPolicy").unwrap() + 4;
    f.select(at);
    f.vcx.simulate_keystrokes(hover_key());
    f.settle();
    assert!(f.sent.borrow().contains(&Command::EditorShowHover));
    let (range, markdown) = f.hover().expect("a hover popup");
    assert_eq!(&f.text()[range], "restartPolicy");
    assert!(
        markdown.starts_with("**restartPolicy** · `string (enum)`"),
        "{markdown}"
    );
    assert!(markdown.contains("Restart policy for all containers within the pod."));
    assert!(markdown.contains("Allowed values: `Always`, `OnFailure`, `Never`"));
}

#[gpui::test]
fn nothing_is_shown_where_the_schema_is_silent(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.type_text("apiVersion: v1\nkind: Pod\nspec:\n  nope: 1\n");
    let at = f.text().find("nope").unwrap() + 1;
    f.select(at);
    f.vcx.simulate_keystrokes(hover_key());
    f.settle();
    assert!(f.hover().is_none());
}

/// Asks for completions and runs `meanwhile` before the background answer is applied: the
/// request is spawned and the executor is not run until `meanwhile` returns.
fn complete_then(
    f: &mut Fixture,
    meanwhile: impl FnOnce(&mut CodeEditor, &mut Window, &mut Context<CodeEditor>),
) {
    let code = f.code();
    f.vcx.update(|window, cx| {
        code.update(cx, |code, cx| {
            code.show_completions(window, cx);
            meanwhile(code, window, cx);
        });
    });
    f.settle();
}

#[gpui::test]
fn a_control_request_with_nothing_in_between_opens_the_menu(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.type_text(POD);
    complete_then(&mut f, |_, _, _| {});
    assert!(f.menu().contains(&"image".to_owned()), "{:?}", f.menu());
}

#[gpui::test]
fn an_answer_for_an_older_buffer_is_dropped(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.type_text(POD);
    let text = f.text();
    // The buffer changes (same text, new version, cursor put back at the end) while the answer
    // is in flight: only the version check can tell.
    complete_then(&mut f, |code, window, cx| {
        let mut api = code.api(window, cx);
        api.set_text(&text);
        api.select(text.len()..text.len());
    });
    assert!(f.menu().is_empty(), "{:?}", f.menu());
}

#[gpui::test]
fn an_answer_for_a_cursor_that_moved_is_dropped(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.type_text(POD);
    let before = f.text().len() - 1;
    // Same version, different cursor: only the cursor check can tell.
    complete_then(&mut f, |code, window, cx| {
        code.api(window, cx).select(before..before);
    });
    assert!(f.menu().is_empty(), "{:?}", f.menu());
}

impl Fixture {
    /// Where `needle` is drawn, in window coordinates.
    fn bounds_of(&mut self, needle: &str) -> gpui::Bounds<gpui::Pixels> {
        let at = self.text().find(needle).expect("needle in the buffer");
        let code = self.code();
        self.vcx
            .update(|window, cx| {
                code.update(cx, |code, cx| {
                    code.api(window, cx).range_to_bounds(at..at + needle.len())
                })
            })
            .expect("the text is on screen")
    }
}

#[gpui::test]
fn the_pointer_over_a_field_shows_its_docs(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.type_text("apiVersion: v1\nkind: Pod\nspec:\n  restartPolicy: Never\n  hostNetwork: true\n");
    f.vcx.run_until_parked();
    let bounds = f.bounds_of("hostNetwork");
    f.vcx
        .simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
    f.vcx
        .executor()
        .advance_clock(std::time::Duration::from_millis(400));
    f.vcx.run_until_parked();
    let (range, markdown) = f.hover().expect("a hover popup");
    assert_eq!(&f.text()[range], "hostNetwork");
    assert!(
        markdown.starts_with("**hostNetwork** · `boolean`"),
        "{markdown}"
    );
    assert!(
        markdown.contains("Host networking requested for this pod."),
        "{markdown}"
    );

    // Moving the pointer to another token replaces the popup.
    let bounds = f.bounds_of("apiVersion");
    f.vcx
        .simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
    f.vcx
        .executor()
        .advance_clock(std::time::Duration::from_millis(400));
    f.vcx.run_until_parked();
    let (_, markdown) = f.hover().expect("apiVersion is documented too");
    assert!(markdown.starts_with("**apiVersion**"), "{markdown}");
}
