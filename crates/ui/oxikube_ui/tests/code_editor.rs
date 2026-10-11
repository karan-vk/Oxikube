//! The code editor behind `EditorApi` (E10-S04): squiggles from our diagnostics, read-only,
//! undo and redo, folding, multiple cursors and soft wrap, driven through the keymap the library
//! binds, as a user would.

use std::ops::Range;

use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers, ParentElement as _,
    Render, Styled as _, TestAppContext, Window, div, point, px,
};
use oxikube_testkit::gpui_test::{TestApp, TestWindow};
use oxikube_ui::editor::{
    CodeEditor, CodeEditorEvent, CodeEditorOptions, Decoration, DecorationStyle, DiagnosticLevel,
    EditorApi as _, EditorDiagnostic, EditorFont,
};

/// A 320 px wide window around one editor, focused.
struct Host {
    editor: Entity<CodeEditor>,
    changes: Vec<u64>,
}

impl Host {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| CodeEditor::new(CodeEditorOptions::YAML, window, cx));
        cx.subscribe(&editor, |host, _, event: &CodeEditorEvent, _| {
            if let CodeEditorEvent::Changed { version } = event {
                host.changes.push(*version);
            }
        })
        .detach();
        window.focus(&editor.focus_handle(cx), cx);
        Self {
            editor,
            changes: Vec::new(),
        }
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(320.)).h(px(400.)).child(self.editor.clone())
    }
}

fn open(cx: &mut TestAppContext, text: &str) -> TestWindow<Host> {
    let mut app = TestApp::new(cx);
    app.update(oxikube_ui::init);
    let mut window = app.open_window(Host::new);
    let text = text.to_owned();
    with_api(&mut window, move |api| api.set_text(&text));
    window.draw_frame();
    window
}

/// Runs `f` on the editor's `EditorApi`.
fn with_api<R>(
    window: &mut TestWindow<Host>,
    f: impl FnOnce(&mut oxikube_ui::editor::LiveEditor<'_, '_>) -> R,
) -> R {
    let editor = window.read_root(|host, _| host.editor.clone());
    window.update_root(|_, window, cx| {
        editor.update(cx, |editor, cx| {
            let mut api = editor.api(window, cx);
            f(&mut api)
        })
    })
}

fn text(window: &mut TestWindow<Host>) -> String {
    with_api(window, |api| api.text().to_string())
}

fn select(window: &mut TestWindow<Host>, range: Range<usize>) {
    with_api(window, move |api| api.select(range));
}

/// The byte ranges the library underlines.
fn squiggles(window: &mut TestWindow<Host>) -> Vec<Range<usize>> {
    window.read_root(|host, cx| {
        let state = host.editor.read(cx).state().read(cx);
        state
            .diagnostics()
            .map(|set| set.iter().map(|entry| entry.range.clone()).collect())
            .unwrap_or_default()
    })
}

const POD: &str = "apiVersion: v1\nkind: Pod\nmetdata:\n  name: web\n";

#[gpui::test]
fn diagnostics_become_squiggles_and_an_edit_clears_them(cx: &mut TestAppContext) {
    let mut window = open(cx, POD);
    let start = POD.find("metdata").unwrap();
    with_api(&mut window, |api| {
        api.set_diagnostics(vec![
            EditorDiagnostic::new(start..start + 7, DiagnosticLevel::Warning, "unknown field")
                .with_code("unknown-field"),
            EditorDiagnostic::new(0..10, DiagnosticLevel::Error, "bad apiVersion"),
        ]);
    });
    window.draw_frame();
    assert_eq!(squiggles(&mut window), vec![0..10, start..start + 7]);
    let shown = with_api(&mut window, |api| api.diagnostics());
    assert_eq!(shown.len(), 2);
    assert_eq!(shown[0].level, DiagnosticLevel::Error, "sorted by position");
    let status = window.read_root(|host, cx| host.editor.read(cx).status().clone());
    assert_eq!(status.as_ref(), "1 error, 1 warning");

    let before = with_api(&mut window, |api| api.version());
    select(&mut window, POD.len()..POD.len());
    window.simulate_input("x");
    assert_eq!(with_api(&mut window, |api| api.version()), before + 1);
    assert!(squiggles(&mut window).is_empty(), "stale squiggles dropped");
    assert!(with_api(&mut window, |api| api.diagnostics()).is_empty());
    assert_eq!(
        window.read_root(|host, _| host.changes.last().copied()),
        Some(before + 1)
    );
}

#[gpui::test]
fn read_only_rejects_typing(cx: &mut TestAppContext) {
    let mut window = open(cx, "a: 1\n");
    with_api(&mut window, |api| api.set_read_only(true));
    window.draw_frame();
    select(&mut window, 0..0);
    window.simulate_input("zz");
    assert_eq!(text(&mut window), "a: 1\n");
    assert!(with_api(&mut window, |api| api.is_read_only()));

    with_api(&mut window, |api| api.set_read_only(false));
    window.draw_frame();
    window.simulate_input("b");
    assert_eq!(text(&mut window), "ba: 1\n");
}

fn undo_keys() -> (&'static str, &'static str) {
    if cfg!(target_os = "macos") {
        ("cmd-z", "cmd-shift-z")
    } else {
        ("ctrl-z", "ctrl-y")
    }
}

#[gpui::test]
fn undo_and_redo(cx: &mut TestAppContext) {
    let mut window = open(cx, "a: 1\n");
    select(&mut window, 4..4);
    window.simulate_input("2");
    assert_eq!(text(&mut window), "a: 12\n");
    let (undo, redo) = undo_keys();
    window.simulate_keystrokes(undo);
    assert_eq!(text(&mut window), "a: 1\n");
    window.simulate_keystrokes(redo);
    assert_eq!(text(&mut window), "a: 12\n");
}

fn add_cursor_below() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-alt-down"
    } else if cfg!(target_os = "windows") {
        "ctrl-alt-down"
    } else {
        "shift-alt-down"
    }
}

#[gpui::test]
fn a_second_cursor_types_on_both_lines(cx: &mut TestAppContext) {
    let mut window = open(cx, "a: 1\nb: 2\n");
    select(&mut window, 0..0);
    window.simulate_keystrokes(add_cursor_below());
    window.simulate_input("x");
    assert_eq!(text(&mut window), "xa: 1\nxb: 2\n");
}

const LONG: &str = "description: a very long value that cannot fit in a narrow editor of three hundred pixels at all\n";

#[gpui::test]
fn soft_wrap_moves_the_end_of_a_long_line_down(cx: &mut TestAppContext) {
    let mut window = open(cx, LONG);
    let end = LONG.len() - 1;
    let rows = |window: &mut TestWindow<Host>| {
        with_api(window, move |api| {
            let start = api.range_to_bounds(0..0).expect("laid out").origin.y;
            let end = api.range_to_bounds(end..end).expect("laid out").origin.y;
            (start, end)
        })
    };
    assert!(!with_api(&mut window, |api| api.soft_wrap()));
    let (start, end_y) = rows(&mut window);
    assert_eq!(start, end_y, "one row without wrapping");

    with_api(&mut window, |api| api.set_soft_wrap(true));
    window.draw_frame();
    window.draw_frame();
    let (start, end_y) = rows(&mut window);
    assert!(end_y > start, "wrapped onto later rows");

    with_api(&mut window, |api| api.set_soft_wrap(false));
    window.draw_frame();
    window.draw_frame();
    let (start, end_y) = rows(&mut window);
    assert_eq!(start, end_y);
}

const FOLDABLE: &str = "metadata:\n  name: web\n  labels:\n    app: web\nspec: {}\n";

#[gpui::test]
fn folding_a_mapping_hides_its_lines(cx: &mut TestAppContext) {
    let mut window = open(cx, FOLDABLE);
    window.draw_frame();
    let spec = FOLDABLE.find("spec").unwrap();
    // Unfolded, down from the end of line 0 lands on line 1.
    select(&mut window, 9..9);
    window.simulate_keystrokes("down");
    let cursor = with_api(&mut window, |api| api.selections()[0].start);
    assert!(
        cursor < FOLDABLE.find("  labels").unwrap(),
        "line 1, got {cursor}"
    );
    // A fold never hides the cursor: put it on the header first.
    select(&mut window, 0..0);
    // The fold chevron sits in the gutter, just left of the text (`FOLD_ICON_HITBOX_WIDTH` is
    // 18 px, after a 6 px margin): click the middle of line 0's.
    let line = with_api(&mut window, |api| api.range_to_bounds(0..0)).expect("laid out");
    let at = point(
        line.origin.x - px(15.),
        line.origin.y + line.size.height / 2.,
    );
    window.simulate_click(at, Modifiers::none());
    window.draw_frame();

    // The cursor at the end of line 0 moves down past the folded mapping, to `spec`.
    select(&mut window, 9..9);
    window.simulate_keystrokes("down");
    let cursor = with_api(&mut window, |api| api.selections()[0].start);
    assert!(
        cursor >= spec,
        "down from a folded header lands after the fold, got offset {cursor}"
    );
}

#[gpui::test]
fn decorations_follow_edits(cx: &mut TestAppContext) {
    let mut window = open(cx, "status:\n  phase: Running\n");
    with_api(&mut window, |api| {
        api.add_decoration(Decoration {
            range: 0..7,
            style: DecorationStyle::Dimmed,
        });
        api.add_decoration(Decoration {
            range: 10..15,
            style: DecorationStyle::Highlight,
        });
    });
    window.draw_frame();
    // Two characters typed before both ranges move them along.
    select(&mut window, 0..0);
    window.simulate_input("# ");
    let moved = with_api(&mut window, |api| api.decorations());
    assert_eq!(
        moved,
        vec![
            Decoration {
                range: 2..9,
                style: DecorationStyle::Dimmed
            },
            Decoration {
                range: 12..17,
                style: DecorationStyle::Highlight
            },
        ]
    );
    with_api(&mut window, |api| api.clear_decorations());
    assert!(with_api(&mut window, |api| api.decorations()).is_empty());
}

#[gpui::test]
fn tab_size_changes_the_indent_without_touching_the_text_or_undo(cx: &mut TestAppContext) {
    let mut window = open(cx, "");
    window.simulate_input("a:");
    assert_eq!(with_api(&mut window, |api| api.tab_size()), 2);
    window.simulate_keystrokes("tab");
    assert_eq!(text(&mut window), "a:  ");

    let version = with_api(&mut window, |api| {
        api.set_tab_size(4);
        api.version()
    });
    assert_eq!(with_api(&mut window, |api| api.tab_size()), 4);
    assert_eq!(with_api(&mut window, |api| api.version()), version);
    assert_eq!(text(&mut window), "a:  ", "the text is as it was");
    window.simulate_keystrokes("tab");
    assert_eq!(
        text(&mut window),
        "a:      ",
        "the next indent is four spaces"
    );

    let (undo, _) = undo_keys();
    window.simulate_keystrokes(undo);
    window.simulate_keystrokes(undo);
    assert_eq!(
        text(&mut window),
        "a:",
        "the history from before the change is intact"
    );
}

#[gpui::test]
fn replace_text_is_one_undo_step_that_keeps_the_history(cx: &mut TestAppContext) {
    let mut window = open(cx, "a: 1\n");
    select(&mut window, 4..4);
    window.simulate_input("2");
    assert_eq!(text(&mut window), "a: 12\n");
    let before = with_api(&mut window, |api| api.version());

    with_api(&mut window, |api| api.replace_text("{\n  \"a\": 12\n}\n"));
    window.draw_frame();
    assert_eq!(text(&mut window), "{\n  \"a\": 12\n}\n");
    assert_eq!(
        with_api(&mut window, |api| api.version()),
        before + 1,
        "one change event for the whole replacement"
    );

    let (undo, redo) = undo_keys();
    window.simulate_keystrokes(undo);
    assert_eq!(
        text(&mut window),
        "a: 12\n",
        "one undo restores the text from before"
    );
    window.simulate_keystrokes(redo);
    assert_eq!(text(&mut window), "{\n  \"a\": 12\n}\n");
    window.simulate_keystrokes(undo);
    window.simulate_keystrokes(undo);
    assert_eq!(
        text(&mut window),
        "a: 1\n",
        "earlier typing is still in the history"
    );
}

#[gpui::test]
fn replace_text_works_on_a_read_only_buffer(cx: &mut TestAppContext) {
    let mut window = open(cx, "a: 1\n");
    with_api(&mut window, |api| api.set_read_only(true));
    with_api(&mut window, |api| api.replace_text("b: 2\n"));
    assert_eq!(text(&mut window), "b: 2\n");
    assert!(with_api(&mut window, |api| api.is_read_only()));
}

#[gpui::test]
fn the_grammar_can_change_without_touching_text_or_history(cx: &mut TestAppContext) {
    let mut window = open(cx, "{\"a\": 1}\n");
    select(&mut window, 0..0);
    window.simulate_input(" ");
    let before = with_api(&mut window, |api| (api.text(), api.version()));
    assert_eq!(with_api(&mut window, |api| api.language()), "yaml");

    with_api(&mut window, |api| api.set_language("json"));
    window.draw_frame();
    assert_eq!(with_api(&mut window, |api| api.language()), "json");
    let name = window.read_root(|host, cx| host.editor.read(cx).state().read(cx).language_name());
    assert_eq!(name.as_ref(), "json");
    assert_eq!(
        with_api(&mut window, |api| (api.text(), api.version())),
        before
    );
    let (undo, _) = undo_keys();
    window.simulate_keystrokes(undo);
    assert_eq!(
        text(&mut window),
        "{\"a\": 1}\n",
        "the typed space is still undoable"
    );
}

#[gpui::test]
fn the_font_sets_the_text_size_in_zoomed_pixels(cx: &mut TestAppContext) {
    let mut window = open(cx, "a: 1\n");
    let size = |window: &mut TestWindow<Host>| {
        window.read_root(|host, cx| f32::from(host.editor.read(cx).text_size(cx)))
    };
    let theme = size(&mut window);
    assert_eq!(
        with_api(&mut window, |api| api.font()),
        EditorFont::default()
    );

    let font = EditorFont {
        family: Some("Menlo".into()),
        size: Some(14.0),
    };
    with_api(&mut window, |api| api.set_font(font.clone()));
    assert_eq!(with_api(&mut window, |api| api.font()), font);
    assert_eq!(size(&mut window), 14.0);

    window.update_root(|_, _, cx| oxikube_ui::set_ui_scale(cx, oxikube_ui::UiScale::new(1.5)));
    assert_eq!(size(&mut window), 21.0, "14 points at a zoom of 1.5");

    window.update_root(|_, _, cx| oxikube_ui::set_ui_scale(cx, oxikube_ui::UiScale::IDENTITY));
    with_api(&mut window, |api| api.set_font(EditorFont::default()));
    assert_eq!(size(&mut window), theme, "back to the theme's size");
}

#[gpui::test]
fn the_json_grammar_is_available(cx: &mut TestAppContext) {
    let _app = TestApp::new(cx);
    assert!(
        gpui_component::highlighter::LanguageRegistry::singleton()
            .language("json")
            .is_some(),
        "tree-sitter-json is part of gpui-component's `tree-sitter` feature"
    );
}
