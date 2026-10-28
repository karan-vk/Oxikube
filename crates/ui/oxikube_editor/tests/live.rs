//! Diff vs live and the changed-on-server banner (E10-S07) in a workspace: an editor loads the
//! object its manifest names (`editor::ReloadFromServer` by its key), follows it through a watch
//! on a `FakeResourcePort` (scripted events on the fake's clock, the deterministic runtime),
//! reloads silently while clean, raises the banner while dirty, and Merge / Reload / Keep mine
//! end where the story says. The diff opens in the bottom panel by its key, side by side or
//! inline; closing the tab closes the watch.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Entity, TestAppContext, VisualTestContext, WeakEntity, Window};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_editor::diff::{LiveObject, SyncState, buffer_text};
use oxikube_editor::view::{
    DIFF_DEBOUNCE, DiffLayout, EditorHost, EditorRequest, EditorViewSink, EditorViews,
    EditorViewsDeps, LiveSource, ManifestEditor, SchemaSource,
};
use oxikube_ports::SchemaPort;
use oxikube_testkit::fakes::{FakeResourcePort, FakeSchemaPort, ResourceCall};
use oxikube_testkit::{ScriptedFeed, TICK};
use oxikube_ui::editor::EditorApi as _;
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Item, Workspace};
use serde_json::json;

mod support;

use support::FakeLive;

fn cluster() -> ClusterId {
    ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"))
}

fn config_map(rv: &str, a: &str, b: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "web", "namespace": "shop", "resourceVersion": rv, "uid": "u-1"},
        "data": {"a": a, "b": b, "c": "3", "d": "4", "e": "5"}
    }))
    .expect("fixture ConfigMap is a valid resource")
}

const NAMES_WEB: &str =
    "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: web\n  namespace: shop\n";

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

struct NoSchemas;

impl SchemaSource for NoSchemas {
    fn cluster(&self) -> &ClusterId {
        static CLUSTER: std::sync::OnceLock<ClusterId> = std::sync::OnceLock::new();
        CLUSTER.get_or_init(cluster)
    }
    fn label(&self, _: &App) -> gpui::SharedString {
        "kind-oxikube".into()
    }
    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        Some(Arc::new(FakeSchemaPort::new()))
    }
}

struct TestHost {
    workspace: WeakEntity<Workspace>,
    port: Arc<FakeResourcePort>,
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
        Rc::new(NoSchemas)
    }
    fn live(&self, _: &ClusterId) -> Option<Rc<dyn LiveSource>> {
        Some(Rc::new(FakeLive::new(cluster(), self.port.clone())))
    }
}

struct Fixture {
    workspace: Entity<Workspace>,
    vcx: VisualTestContext,
    sent: Rc<RefCell<Vec<Command>>>,
    dispatcher: Rc<dyn CommandDispatcher>,
    port: Arc<FakeResourcePort>,
    _views: Entity<EditorViews>,
}

/// A window whose cluster serves `feed` as the next watch.
fn setup(cx: &mut TestAppContext, feed: ScriptedFeed) -> Fixture {
    let (workspace, mut vcx) = open_workspace(cx);
    let port = Arc::new(FakeResourcePort::new());
    feed.install(&port);
    let sent = Rc::new(RefCell::new(Vec::new()));
    let (sink, requests) = EditorViewSink::channel();
    let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(SinkDispatcher {
        sink,
        sent: sent.clone(),
    });
    let host = Rc::new(TestHost {
        workspace: workspace.downgrade(),
        port: port.clone(),
    });
    let views = vcx.update(|window, cx| {
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
        oxikube_editor::view::install(dispatcher.clone(), cx);
        oxikube_keymap::init_with_text("", oxikube_keymap::KeymapOptions::default(), cx);
        let deps = EditorViewsDeps {
            host,
            window_workspace: workspace.downgrade(),
            dispatcher: dispatcher.clone(),
        };
        EditorViews::start(deps, requests, window, cx)
    });
    vcx.run_until_parked();
    Fixture {
        workspace,
        vcx,
        sent,
        dispatcher,
        port,
        _views: views,
    }
}

fn key(mac: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        mac
    } else {
        other
    }
}

impl Fixture {
    fn open_editor(&mut self) -> Entity<ManifestEditor> {
        self.vcx
            .simulate_keystrokes(key("cmd-shift-e", "ctrl-shift-e"));
        self.vcx.run_until_parked();
        self.vcx
            .read(|cx| self.workspace.read(cx).items_of_type::<ManifestEditor>())
            .pop()
            .expect("an editor opened")
    }

    fn set_text(&mut self, editor: &Entity<ManifestEditor>, text: &str) {
        let code = self.vcx.read(|cx| editor.read(cx).editor().clone());
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| code.api(window, cx).set_text(text));
        });
        self.vcx.run_until_parked();
    }

    fn text(&mut self, editor: &Entity<ManifestEditor>) -> String {
        let code = self.vcx.read(|cx| editor.read(cx).editor().clone());
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| code.api(window, cx).text().to_string())
        })
    }

    fn state(&mut self, editor: &Entity<ManifestEditor>) -> SyncState {
        self.vcx.read(|cx| editor.read(cx).live_sync().state())
    }

    fn dirty(&mut self, editor: &Entity<ManifestEditor>) -> bool {
        self.vcx.read(|cx| editor.read(cx).tab_content(cx).dirty)
    }

    fn run(&mut self, command: Command) {
        let dispatcher = self.dispatcher.clone();
        self.vcx.update(|_, cx| dispatcher.dispatch(command, cx));
        self.vcx.run_until_parked();
    }

    /// The next scripted tick of the watch.
    fn tick(&mut self) {
        self.port.clock().advance(TICK);
        self.vcx.run_until_parked();
        // The watch's results notify at frame cadence; a test window draws on the backstop.
        self.vcx.executor().advance_clock(TICK);
        self.vcx.run_until_parked();
    }

    fn shown(&mut self, selector: &'static str) -> bool {
        self.vcx.debug_bounds(selector).is_some()
    }

    /// An editor loaded from `web` by the reload key: clean, following the object.
    fn loaded_editor(&mut self) -> Entity<ManifestEditor> {
        let editor = self.open_editor();
        self.set_text(&editor, NAMES_WEB);
        self.vcx
            .simulate_keystrokes(key("cmd-alt-shift-r", "ctrl-alt-shift-r"));
        self.vcx.run_until_parked();
        editor
    }
}

fn feed() -> ScriptedFeed {
    ScriptedFeed::new().initial([config_map("1", "1", "2")])
}

#[gpui::test]
fn reload_loads_the_named_object_and_follows_it_through_one_watch(cx: &mut TestAppContext) {
    let mut f = setup(cx, feed().modify(1, config_map("2", "10", "2")));
    let editor = f.loaded_editor();
    assert!(f.sent.borrow().contains(&Command::EditorReloadFromServer));
    assert_eq!(f.text(&editor), buffer_text(&config_map("1", "1", "2")));
    assert!(!f.dirty(&editor), "a loaded buffer is clean");
    assert_eq!(f.port.live_watches(), 1);
    let watch = f
        .port
        .recorded_calls()
        .into_iter()
        .find_map(|call| match call {
            ResourceCall::Watch {
                namespace, options, ..
            } => Some((namespace, options.field_selector)),
            _ => None,
        });
    assert_eq!(
        watch,
        Some((
            Some("shop".to_owned()),
            Some("metadata.name=web".to_owned())
        ))
    );

    // The object changes on the server: the clean buffer follows, silently.
    f.tick();
    assert_eq!(f.text(&editor), buffer_text(&config_map("2", "10", "2")));
    assert_eq!(f.state(&editor), SyncState::Clean);
    assert!(!f.shown("manifest-banner"));
    let rv = f.vcx.read(|cx| {
        editor
            .read(cx)
            .live_sync()
            .live_resource_version()
            .map(str::to_owned)
    });
    assert_eq!(
        rv.as_deref(),
        Some("2"),
        "the live resourceVersion, for S08"
    );
}

#[gpui::test]
fn a_server_change_while_dirty_raises_the_banner_and_reload_drops_the_edits(
    cx: &mut TestAppContext,
) {
    let mut f = setup(cx, feed().modify(1, config_map("2", "10", "2")));
    let editor = f.loaded_editor();
    let edited = f.text(&editor).replace(r#"b: "2""#, r#"b: "20""#);
    f.set_text(&editor, &edited);
    assert!(f.dirty(&editor));
    f.tick();
    assert_eq!(f.state(&editor), SyncState::Banner);
    assert!(f.shown("manifest-banner"));
    assert_eq!(
        f.text(&editor),
        edited,
        "a dirty buffer is never replaced unasked"
    );

    f.vcx
        .debug_bounds("manifest-reload")
        .expect("Reload is offered");
    f.run(Command::EditorReloadFromServer);
    assert_eq!(f.text(&editor), buffer_text(&config_map("2", "10", "2")));
    assert_eq!(f.state(&editor), SyncState::Clean);
    assert!(!f.dirty(&editor));
    assert!(!f.shown("manifest-banner"));
}

#[gpui::test]
fn merge_keeps_both_changes_when_they_do_not_overlap(cx: &mut TestAppContext) {
    let mut f = setup(cx, feed().modify(1, config_map("2", "10", "2")));
    let editor = f.loaded_editor();
    let edited = format!(
        "# mine\n{}",
        f.text(&editor).replace(r#"e: "5""#, r#"e: "50""#)
    );
    f.set_text(&editor, &edited);
    f.tick();
    assert_eq!(f.state(&editor), SyncState::Banner);

    f.run(Command::EditorMergeFromServer);
    let text = f.text(&editor);
    assert!(
        text.starts_with("# mine\n"),
        "the buffer's comment stays: {text}"
    );
    assert!(text.contains(r#"a: "10""#), "the server's change: {text}");
    assert!(text.contains(r#"e: "50""#), "the buffer's change: {text}");
    assert_eq!(f.state(&editor), SyncState::Dirty);
    assert!(f.dirty(&editor));
    assert!(!f.shown("manifest-banner"));
}

#[gpui::test]
fn a_conflicting_merge_waits_for_a_side_per_conflict(cx: &mut TestAppContext) {
    let mut f = setup(cx, feed().modify(1, config_map("2", "10", "2")));
    let editor = f.loaded_editor();
    let edited = f.text(&editor).replace(r#"a: "1""#, r#"a: "7""#);
    f.set_text(&editor, &edited);
    f.tick();
    f.run(Command::EditorMergeFromServer);
    assert_eq!(f.state(&editor), SyncState::Merging);
    assert!(
        f.shown("manifest-conflicts"),
        "the conflicts are in the panel"
    );
    assert_eq!(
        f.text(&editor),
        edited,
        "nothing merged before every side is picked"
    );

    f.run(Command::EditorAcceptMerge);
    assert_eq!(
        f.state(&editor),
        SyncState::Merging,
        "a conflict has no side yet"
    );
    let conflicts = f.vcx.read(|cx| {
        editor
            .read(cx)
            .live_sync()
            .pending_merge()
            .map(|m| m.conflicts())
    });
    for conflict in 0..conflicts.expect("a pending merge") {
        f.run(Command::EditorTakeServer {
            conflict: conflict as u32,
        });
    }
    f.run(Command::EditorAcceptMerge);
    let text = f.text(&editor);
    assert!(text.contains(r#"a: "10""#), "{text}");
    assert_eq!(f.state(&editor), SyncState::Dirty);
}

/// The server's change, then a `resourceVersion`-only tick of it (a ConfigMap has no status).
fn feed_with_a_bookkeeping_tick() -> ScriptedFeed {
    feed()
        .modify(1, config_map("2", "10", "2"))
        .modify(2, config_map("3", "10", "2"))
}

fn unresolved(f: &mut Fixture, editor: &Entity<ManifestEditor>) -> Option<usize> {
    f.vcx.read(|cx| {
        editor
            .read(cx)
            .live_sync()
            .pending_merge()
            .map(|m| m.unresolved())
    })
}

#[gpui::test]
fn a_bookkeeping_tick_keeps_a_pending_merge_and_its_picks(cx: &mut TestAppContext) {
    let mut f = setup(cx, feed_with_a_bookkeeping_tick());
    let editor = f.loaded_editor();
    let edited = f.text(&editor).replace(r#"a: "1""#, r#"a: "7""#);
    f.set_text(&editor, &edited);
    f.tick();
    f.run(Command::EditorMergeFromServer);
    assert_eq!(f.state(&editor), SyncState::Merging);
    f.run(Command::EditorTakeServer { conflict: 0 });
    let picked = unresolved(&mut f, &editor);

    f.tick();
    assert_eq!(f.state(&editor), SyncState::Merging, "the merge survives");
    assert_eq!(unresolved(&mut f, &editor), picked, "and so does the pick");
    assert!(f.shown("manifest-conflicts"));
}

#[gpui::test]
fn a_bookkeeping_tick_while_the_merge_runs_does_not_drop_it(cx: &mut TestAppContext) {
    let mut f = setup(cx, feed_with_a_bookkeeping_tick());
    let editor = f.loaded_editor();
    let edited = f.text(&editor).replace(r#"a: "1""#, r#"a: "7""#);
    f.set_text(&editor, &edited);
    f.tick();
    // Merge asked, and the tick lands before its background result is taken.
    let dispatcher = f.dispatcher.clone();
    f.vcx
        .update(|_, cx| dispatcher.dispatch(Command::EditorMergeFromServer, cx));
    f.tick();
    assert_eq!(
        f.state(&editor),
        SyncState::Merging,
        "the merge was made from the same object, server fields aside"
    );
}

#[gpui::test]
fn keep_mine_dismisses_the_banner_and_keeps_the_buffer(cx: &mut TestAppContext) {
    let mut f = setup(cx, feed().modify(1, config_map("2", "10", "2")));
    let editor = f.loaded_editor();
    let edited = f.text(&editor).replace(r#"b: "2""#, r#"b: "20""#);
    f.set_text(&editor, &edited);
    f.tick();
    // The banner's own button.
    let keep = f.vcx.debug_bounds("manifest-keep").expect("Keep mine");
    f.vcx
        .simulate_click(keep.center(), gpui::Modifiers::default());
    f.vcx.run_until_parked();
    assert!(f.sent.borrow().contains(&Command::EditorKeepMine));
    assert_eq!(f.state(&editor), SyncState::Dirty);
    assert_eq!(f.text(&editor), edited);
    assert!(!f.shown("manifest-banner"));
}

#[gpui::test]
fn the_diff_opens_side_by_side_and_switches_to_inline(cx: &mut TestAppContext) {
    let mut f = setup(cx, feed());
    let editor = f.loaded_editor();
    // A comment only: no change.
    let commented = format!("# a note\n{}", f.text(&editor));
    f.set_text(&editor, &commented);
    f.vcx
        .simulate_keystrokes(key("cmd-shift-d", "ctrl-shift-d"));
    f.vcx.executor().advance_clock(DIFF_DEBOUNCE);
    f.vcx.run_until_parked();
    assert!(f.shown("manifest-diff"));
    let diff = f.vcx.read(|cx| editor.read(cx).diff()).expect("computed");
    assert!(diff.is_empty(), "comments are not changes");

    let changed = commented.replace(r#"a: "1""#, r#"a: "5""#);
    f.set_text(&editor, &changed);
    f.vcx.executor().advance_clock(DIFF_DEBOUNCE);
    f.vcx.run_until_parked();
    let diff = f.vcx.read(|cx| editor.read(cx).diff()).expect("computed");
    assert_eq!((diff.hunks(), diff.counts()), (1, (1, 1)));
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).diff_layout()),
        DiffLayout::SideBySide
    );
    f.vcx
        .simulate_keystrokes(key("cmd-alt-shift-d", "ctrl-alt-shift-d"));
    f.vcx.run_until_parked();
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).diff_layout()),
        DiffLayout::Inline
    );
    assert!(f.shown("manifest-diff"));
}

#[gpui::test]
fn a_typed_manifest_is_compared_with_the_object_it_names(cx: &mut TestAppContext) {
    let mut f = setup(cx, feed());
    let editor = f.open_editor();
    let typed = format!("{NAMES_WEB}data:\n  a: '1'\n  b: '3'\n  c: '3'\n  d: '4'\n  e: '5'\n");
    f.set_text(&editor, &typed);
    f.run(Command::EditorShowDiff);
    let diff = f.vcx.read(|cx| editor.read(cx).diff()).expect("computed");
    assert_eq!(diff.counts(), (1, 1), "b differs");
    assert_eq!(
        f.text(&editor),
        typed,
        "showing a diff never touches the buffer"
    );
    let target = f
        .vcx
        .read(|cx| editor.read(cx).live_target().map(|t| t.name.to_string()));
    assert_eq!(target.as_deref(), Some("web"));
}

#[gpui::test]
fn closing_the_tab_closes_the_watch(cx: &mut TestAppContext) {
    let mut f = setup(cx, feed());
    let editor = f.loaded_editor();
    assert_eq!(f.port.live_watches(), 1);
    let workspace = f.workspace.clone();
    let id = editor.entity_id();
    f.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| assert!(ws.close_item(id, window, cx)));
    });
    f.vcx.run_until_parked();
    assert_eq!(f.port.live_watches(), 0, "the subscription is released");
}

#[gpui::test]
fn a_watch_that_ended_is_restarted_by_the_next_diff(cx: &mut TestAppContext) {
    // The first watch lists the object and ends (a dropped connection); the next one stays open.
    let first =
        oxikube_testkit::Timeline::immediate([oxikube_ports::DeltaBatch::from_deltas(vec![
            oxikube_ports::Delta::Restarted(vec![config_map("1", "1", "2")]),
        ])]);
    let mut f = setup(cx, ScriptedFeed::new());
    f.port.insert(config_map("1", "1", "2"));
    f.port.script().watch.clear();
    f.port.script().watch.push_ok(first);
    let editor = f.loaded_editor();
    assert_eq!(f.port.live_watches(), 0, "the first watch ended");
    let watches = |f: &Fixture| {
        f.port
            .recorded_calls()
            .into_iter()
            .filter(|call| matches!(call, ResourceCall::Watch { .. }))
            .count()
    };
    assert_eq!(watches(&f), 1);

    f.run(Command::EditorShowDiff);
    assert_eq!(watches(&f), 2, "the diff asked for a new watch");
    assert_eq!(f.port.live_watches(), 1);
    let diff = f.vcx.read(|cx| editor.read(cx).diff()).expect("computed");
    assert!(diff.is_empty());
}

#[gpui::test]
fn a_manifest_without_a_namespace_never_follows_a_same_named_object_elsewhere(
    cx: &mut TestAppContext,
) {
    // `shop/web` exists, `default/web` does not: the manifest names `default/web`, as `kubectl
    // apply` reads it, so the live object is gone and every line is added.
    let mut f = setup(cx, feed());
    let editor = f.open_editor();
    let typed = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: web\ndata:\n  a: '1'\n";
    f.set_text(&editor, typed);
    f.run(Command::EditorShowDiff);
    let live = f.vcx.read(|cx| editor.read(cx).live_sync().live().cloned());
    assert!(matches!(live, Some(LiveObject::Gone)), "{live:?}");
    let diff = f.vcx.read(|cx| editor.read(cx).diff()).expect("computed");
    assert_eq!(diff.counts().0, 0, "nothing of shop/web is compared");

    f.run(Command::EditorReloadFromServer);
    assert_eq!(f.text(&editor), typed, "shop/web is never loaded");
}

#[gpui::test]
fn keep_mine_without_a_banner_leaves_the_clean_buffer_reloading_silently(cx: &mut TestAppContext) {
    let mut f = setup(cx, feed().modify(1, config_map("2", "10", "2")));
    let editor = f.loaded_editor();
    // From the palette (or the MCP tool): there is no banner to answer.
    f.run(Command::EditorKeepMine);
    assert_eq!(f.state(&editor), SyncState::Clean);
    f.tick();
    assert_eq!(f.text(&editor), buffer_text(&config_map("2", "10", "2")));
    assert!(!f.shown("manifest-banner"));
}

#[gpui::test]
fn a_long_conflict_builds_only_the_rows_on_screen(cx: &mut TestAppContext) {
    let server = Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "web", "namespace": "shop", "resourceVersion": "2", "uid": "u-1"},
        "data": {"a": "10", "b": "2", "c": "3", "d": "4", "e": "50"}
    }))
    .unwrap();
    let mut f = setup(cx, feed().modify(1, server));
    let editor = f.loaded_editor();
    // Conflict 0: one server line against 300 of the buffer's; conflict 1 far below it.
    let notes: String = (0..300).map(|i| format!("  # note {i}\n")).collect();
    let edited = f
        .text(&editor)
        .replace("  a: \"1\"\n", &format!("  a: \"7\"\n{notes}"))
        .replace(r#"e: "5""#, r#"e: "70""#);
    f.set_text(&editor, &edited);
    f.tick();
    f.run(Command::EditorMergeFromServer);
    let conflicts = f.vcx.read(|cx| {
        editor
            .read(cx)
            .live_sync()
            .pending_merge()
            .map(|m| m.conflicts())
    });
    assert_eq!(conflicts, Some(2));
    f.vcx.run_until_parked();
    assert!(
        f.shown("manifest-take-mine-0"),
        "the first conflict is on screen"
    );
    assert!(
        !f.shown("manifest-take-mine-1"),
        "the second is 300 lines down: not built"
    );
}
