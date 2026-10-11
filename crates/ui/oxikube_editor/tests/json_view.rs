//! JSON mode in the manifest editor (E10-S10), end to end in a workspace: the convert commands
//! through the shipped keys, the toolbar and the bus (one undo step, the notice, read-only,
//! stale results), the language switch, detection while typing, and the strict JSON check
//! reaching the squiggles.

#[path = "view_common/mod.rs"]
mod common;

use std::time::Duration;

use common::*;
use gpui::{Entity, TestAppContext};
use oxikube_domain::command::{Command, CommandId, ViewContext, lookup};
use oxikube_editor::json::Language;
use oxikube_editor::view::{ManifestEditor, NoticeLevel, VALIDATION_DEBOUNCE};
use oxikube_ui::editor::EditorApi as _;

fn mod_key(rest: &str) -> String {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    format!("{modifier}-{rest}")
}

fn convert_to_json_key() -> String {
    mod_key("alt-j")
}

fn convert_to_yaml_key() -> String {
    mod_key("alt-y")
}

fn undo_key() -> String {
    mod_key("z")
}

const DEPLOYMENT_YAML: &str = "\
# The web tier.
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web   # the name
spec:
  replicas: 3
";

const DEPLOYMENT_JSON: &str = "\
{
  \"apiVersion\": \"apps/v1\",
  \"kind\": \"Deployment\",
  \"metadata\": {
    \"name\": \"web\"
  },
  \"spec\": {
    \"replicas\": 3
  }
}
";

impl Fixture {
    /// Loads `text` into `editor` as a paste would, and lets the first check run.
    fn load(&mut self, editor: &Entity<ManifestEditor>, text: &str) {
        let code = self.vcx.read(|cx| editor.read(cx).editor().clone());
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| code.api(window, cx).replace_text(text));
        });
        self.advance(VALIDATION_DEBOUNCE);
    }

    fn language(&mut self, editor: &Entity<ManifestEditor>) -> Language {
        self.vcx.read(|cx| editor.read(cx).language())
    }

    fn grammar(&mut self, editor: &Entity<ManifestEditor>) -> &'static str {
        self.vcx
            .read(|cx| editor.read(cx).editor().read(cx).language())
    }

    fn notice(&mut self, editor: &Entity<ManifestEditor>) -> Option<(NoticeLevel, String)> {
        self.vcx.read(|cx| {
            editor
                .read(cx)
                .notice()
                .map(|n| (n.level, n.text.to_string()))
        })
    }

    /// Clicks the element tagged `selector`.
    fn click(&mut self, selector: &'static str) {
        let bounds = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is shown"));
        self.vcx
            .simulate_click(bounds.center(), gpui::Modifiers::default());
        self.vcx.run_until_parked();
    }

    fn context_mode(&mut self) -> Option<String> {
        self.vcx.update(|window, _| {
            window.context_stack().iter().find_map(|context| {
                context
                    .contains("ManifestEditor")
                    .then(|| context.get("mode").map(|m| m.to_string()))
                    .flatten()
            })
        })
    }
}

#[gpui::test]
fn the_convert_key_replaces_the_buffer_and_one_undo_restores_the_yaml(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    f.load(&editor, DEPLOYMENT_YAML);
    assert_eq!(f.language(&editor), Language::Yaml);
    f.type_text("#");
    let typed = f.text(&editor);

    f.vcx.simulate_keystrokes(&convert_to_json_key());
    f.vcx.run_until_parked();
    assert!(f.sent.borrow().contains(&Command::EditorConvertToJson));
    // `#` joined the first comment line: the buffer held 2 comments.
    assert_eq!(f.text(&editor), format!("{}\n", DEPLOYMENT_JSON.trim_end()));
    assert_eq!(
        f.language(&editor),
        Language::Json,
        "highlighted and read as JSON"
    );
    assert_eq!(f.grammar(&editor), "json");
    let (level, text) = f.notice(&editor).expect("a notice");
    assert_eq!(level, NoticeLevel::Warning);
    assert_eq!(
        text,
        "Converted to JSON, but 2 comments dropped. Undo brings the YAML back."
    );

    // One undo step brings back the whole YAML, the comments included, and its language.
    f.vcx.simulate_keystrokes(&undo_key());
    f.vcx.run_until_parked();
    assert_eq!(f.text(&editor), typed);
    f.advance(VALIDATION_DEBOUNCE);
    assert_eq!(f.language(&editor), Language::Yaml);
    assert_eq!(f.grammar(&editor), "yaml");
    assert_eq!(
        f.notice(&editor),
        None,
        "the notice goes with the next edit"
    );

    // And forward again: redo is the same single step.
    f.vcx.simulate_keystrokes(&mod_key("shift-z"));
    f.vcx.run_until_parked();
    assert!(f.text(&editor).starts_with("{\n  \"apiVersion\""));
}

#[gpui::test]
fn converting_back_to_yaml_is_also_one_undo_step(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    f.load(&editor, DEPLOYMENT_JSON);
    assert_eq!(
        f.language(&editor),
        Language::Json,
        "detected from the leading brace"
    );

    f.vcx.simulate_keystrokes(&convert_to_yaml_key());
    f.vcx.run_until_parked();
    assert!(f.sent.borrow().contains(&Command::EditorConvertToYaml));
    assert_eq!(
        f.text(&editor),
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: 3\n"
    );
    assert_eq!(f.language(&editor), Language::Yaml);
    let (level, text) = f.notice(&editor).expect("a notice");
    assert_eq!(level, NoticeLevel::Info);
    assert_eq!(text, "Converted to YAML. Undo brings the JSON back.");

    f.vcx.simulate_keystrokes(&undo_key());
    f.vcx.run_until_parked();
    assert_eq!(f.text(&editor), DEPLOYMENT_JSON);
}

#[gpui::test]
fn a_conversion_that_cannot_be_done_leaves_the_buffer_and_says_why(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    let broken = "kind: [\n";
    f.load(&editor, broken);
    f.vcx.simulate_keystrokes(&convert_to_json_key());
    f.vcx.run_until_parked();
    assert_eq!(f.text(&editor), broken);
    let (level, text) = f.notice(&editor).expect("a notice");
    assert_eq!(level, NoticeLevel::Error);
    assert!(
        text.starts_with("Cannot convert to JSON: line 1:"),
        "{text}"
    );
    assert_eq!(f.language(&editor), Language::Yaml);
}

#[gpui::test]
fn a_read_only_editor_refuses_to_convert(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    f.load(&editor, DEPLOYMENT_YAML);
    f.vcx.simulate_keystrokes(&mod_key("alt-r"));
    f.vcx.run_until_parked();
    f.vcx.simulate_keystrokes(&convert_to_json_key());
    f.vcx.run_until_parked();
    assert_eq!(f.text(&editor), DEPLOYMENT_YAML);
    let (level, text) = f.notice(&editor).expect("a notice");
    assert_eq!(level, NoticeLevel::Error);
    assert!(text.contains("read-only"), "{text}");
}

#[gpui::test]
fn read_only_switched_on_while_a_conversion_runs_refuses_its_result(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    f.load(&editor, DEPLOYMENT_YAML);
    f.vcx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.convert_to(Language::Json, window, cx)
        });
    });
    // Before the background conversion is polled: the editor becomes read-only.
    f.vcx.update(|window, cx| {
        editor.update(cx, |editor, cx| editor.toggle_read_only(window, cx));
    });
    f.vcx.run_until_parked();
    assert_eq!(f.text(&editor), DEPLOYMENT_YAML);
    assert_eq!(f.language(&editor), Language::Yaml);
    let (level, text) = f.notice(&editor).expect("a notice");
    assert_eq!(level, NoticeLevel::Error);
    assert!(text.contains("read-only"), "{text}");
}

#[gpui::test]
fn typing_while_a_conversion_runs_drops_its_result(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    f.load(&editor, DEPLOYMENT_YAML);
    f.vcx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.convert_to(Language::Json, window, cx)
        });
    });
    // Before the background conversion is polled: one more keystroke.
    f.type_text("x");
    f.vcx.run_until_parked();
    assert!(
        f.text(&editor).starts_with("x# The web tier."),
        "{}",
        f.text(&editor)
    );
    let (level, text) = f.notice(&editor).expect("a notice");
    assert_eq!(level, NoticeLevel::Warning);
    assert!(text.contains("changed while converting"), "{text}");
}

#[gpui::test]
fn the_toolbar_offers_the_other_language_and_acts_on_its_own_editor(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    f.load(&editor, DEPLOYMENT_YAML);
    f.vcx.run_until_parked();

    f.click("manifest-convert-Untitled-1");
    assert!(f.sent.borrow().contains(&Command::EditorConvertToJson));
    assert_eq!(f.language(&editor), Language::Json);

    // Now JSON: the button offers YAML.
    f.click("manifest-convert-Untitled-1");
    assert!(f.sent.borrow().contains(&Command::EditorConvertToYaml));
    assert_eq!(f.language(&editor), Language::Yaml);
    assert!(f.text(&editor).starts_with("apiVersion: apps/v1"));
}

#[gpui::test]
fn the_language_chip_switches_the_mode_without_touching_the_text(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    let text = "{\"a\": 1,}\n";
    f.load(&editor, text);
    assert_eq!(f.language(&editor), Language::Json);
    // JSON mode: the trailing comma is a syntax error with a span on the comma.
    let comma = text.find(',').unwrap();
    assert_eq!(f.squiggles(&editor), vec![comma..comma + 1]);
    let version = f.vcx.read(|cx| editor.read(cx).editor().read(cx).version());

    f.click("manifest-language-Untitled-1");
    f.advance(VALIDATION_DEBOUNCE);
    assert!(f.sent.borrow().contains(&Command::EditorUseYaml));
    assert_eq!(f.language(&editor), Language::Yaml);
    assert_eq!(f.grammar(&editor), "yaml");
    assert_eq!(f.text(&editor), text, "the text is untouched");
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).editor().read(cx).version()),
        version,
        "no edit, so nothing in the undo history"
    );
    assert!(
        f.squiggles(&editor).is_empty(),
        "flow YAML accepts the comma"
    );
    assert_eq!(f.context_mode().as_deref(), Some("yaml"));
}

#[gpui::test]
fn the_override_command_wins_over_the_content(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    f.load(&editor, "a: 1\n");
    assert_eq!(f.language(&editor), Language::Yaml);

    f.vcx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.use_language(Language::Json, window, cx)
        });
    });
    f.advance(VALIDATION_DEBOUNCE);
    assert_eq!(f.language(&editor), Language::Json);
    assert_eq!(f.grammar(&editor), "json");
    assert_eq!(f.context_mode().as_deref(), Some("json"));
    assert!(!f.squiggles(&editor).is_empty(), "`a: 1` is not JSON");

    // Typing does not undo the user's choice.
    f.type_text("b: 2\n");
    f.advance(VALIDATION_DEBOUNCE);
    assert_eq!(f.language(&editor), Language::Json);
}

#[gpui::test]
fn typing_a_brace_switches_an_undecided_editor_to_json(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    assert_eq!(f.language(&editor), Language::Yaml);
    assert_eq!(f.context_mode().as_deref(), Some("yaml"));
    f.type_text("{\"kind\": \"Pod\"}");
    f.advance(VALIDATION_DEBOUNCE);
    assert_eq!(f.language(&editor), Language::Json);
    assert_eq!(f.grammar(&editor), "json");
    assert_eq!(f.context_mode().as_deref(), Some("json"));
    assert!(f.squiggles(&editor).is_empty());
}

#[gpui::test]
fn json_is_checked_against_the_cluster_schema_like_yaml(cx: &mut TestAppContext) {
    let mut f = setup(cx, true);
    let editor = f.open_editor();
    let json =
        "{\n  \"apiVersion\": \"v1\",\n  \"kind\": \"ConfigMap\",\n  \"data\": {\"a\": 1}\n}\n";
    f.load(&editor, json);
    f.advance(VALIDATION_DEBOUNCE);
    let value = json.rfind('1').unwrap();
    assert_eq!(f.squiggles(&editor), vec![value..value + 1]);
    let problems = f.vcx.read(|cx| editor.read(cx).model().problems());
    assert_eq!((problems.errors, problems.warnings), (1, 0));
}

#[gpui::test]
fn a_json_list_is_checked_item_by_item(cx: &mut TestAppContext) {
    let mut f = setup(cx, true);
    let editor = f.open_editor();
    let json = "{\n  \"apiVersion\": \"v1\",\n  \"kind\": \"List\",\n  \"items\": [\n    {\"apiVersion\": \"v1\", \"kind\": \"ConfigMap\", \"data\": {\"ok\": \"yes\"}},\n    {\"apiVersion\": \"v1\", \"kind\": \"ConfigMap\", \"data\": {\"bad\": 2}}\n  ]\n}\n";
    f.load(&editor, json);
    f.advance(VALIDATION_DEBOUNCE);
    let bad = json.find("\"bad\": 2").unwrap() + "\"bad\": ".len();
    assert_eq!(f.squiggles(&editor), vec![bad..bad + 1]);
    assert_eq!(
        f.port.recorded_calls().len(),
        1,
        "one schema for both items"
    );
}

#[gpui::test]
fn the_conversion_commands_exist_for_the_editor_only(cx: &mut TestAppContext) {
    let _f = setup(cx, false);
    for id in [
        CommandId::EDITOR_CONVERT_TO_JSON,
        CommandId::EDITOR_CONVERT_TO_YAML,
        CommandId::EDITOR_USE_JSON,
        CommandId::EDITOR_USE_YAML,
    ] {
        let meta = lookup(id).unwrap_or_else(|| panic!("{id} is declared"));
        assert_eq!(meta.availability.views, [ViewContext::Editor], "{id}");
        assert!(
            !meta.availability.requires_writable,
            "{id} never touches a cluster"
        );
    }
}

#[gpui::test]
fn undo_after_a_conversion_does_not_reach_earlier_edits(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    f.type_text("a: 1\n");
    f.advance(Duration::from_millis(1000));
    f.vcx.simulate_keystrokes(&convert_to_json_key());
    f.vcx.run_until_parked();
    assert_eq!(f.text(&editor), "{\n  \"a\": 1\n}\n");
    f.vcx.simulate_keystrokes(&undo_key());
    f.vcx.run_until_parked();
    assert_eq!(
        f.text(&editor),
        "a: 1\n",
        "exactly the conversion is undone"
    );
}

#[gpui::test]
fn converting_text_that_is_already_in_the_target_form_adds_no_undo_step(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    f.vcx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.use_language(Language::Yaml, window, cx)
        });
    });
    f.load(&editor, DEPLOYMENT_JSON);
    assert_eq!(f.language(&editor), Language::Yaml, "the user said YAML");
    let version = f.vcx.read(|cx| editor.read(cx).editor().read(cx).version());

    f.vcx.simulate_keystrokes(&convert_to_json_key());
    f.vcx.run_until_parked();
    assert_eq!(f.text(&editor), DEPLOYMENT_JSON);
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).editor().read(cx).version()),
        version,
        "the buffer was not touched"
    );
    assert_eq!(f.language(&editor), Language::Json);
    let (level, text) = f.notice(&editor).expect("a notice");
    assert_eq!(level, NoticeLevel::Info);
    assert_eq!(text, "Already written as JSON.");
    // The notice stays until the next edit, and the next edit clears it.
    f.type_text(" ");
    assert_eq!(f.notice(&editor), None);
}

#[gpui::test]
fn converting_text_already_in_the_target_form_checks_it_again(cx: &mut TestAppContext) {
    let mut f = setup(cx, true);
    let editor = f.open_editor();
    let json = "{\n  \"apiVersion\": \"v1\",\n  \"kind\": \"ConfigMap\",\n  \"data\": {\n    \"a\": 1\n  }\n}\n";
    f.vcx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.use_language(Language::Yaml, window, cx)
        });
    });
    f.load(&editor, json);
    f.advance(VALIDATION_DEBOUNCE);
    let value = json.rfind('1').unwrap();
    assert_eq!(f.squiggles(&editor), vec![value..value + 1]);

    f.vcx.simulate_keystrokes(&convert_to_json_key());
    f.vcx.run_until_parked();
    f.advance(VALIDATION_DEBOUNCE);
    assert_eq!(f.text(&editor), json, "canonical JSON converts to itself");
    assert_eq!(f.language(&editor), Language::Json);
    let (problems, shown) = f.vcx.read(|cx| {
        let model = editor.read(cx).model();
        (model.problems(), model.shown_version())
    });
    assert_eq!((problems.errors, problems.warnings), (1, 0));
    assert!(shown.is_some(), "the text was checked as JSON");
    assert_eq!(f.squiggles(&editor), vec![value..value + 1]);
}

#[gpui::test]
fn converting_right_after_a_paste_does_not_wait_for_the_detection(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    let code = f.vcx.read(|cx| editor.read(cx).editor().clone());
    f.vcx.update(|window, cx| {
        code.update(cx, |code, cx| {
            code.api(window, cx).replace_text(DEPLOYMENT_JSON)
        });
    });
    // The debounce has not run, so the editor has not yet noticed it holds JSON.
    f.vcx.run_until_parked();
    assert_eq!(f.language(&editor), Language::Yaml);
    f.vcx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.convert_to(Language::Yaml, window, cx)
        });
    });
    f.vcx.run_until_parked();
    let notice = f.notice(&editor);
    assert!(
        f.text(&editor).starts_with("apiVersion: apps/v1\n"),
        "{notice:?}\n{}",
        f.text(&editor)
    );
    assert_eq!(f.language(&editor), Language::Yaml);
}

#[gpui::test]
fn converting_to_the_language_a_buffer_is_in_says_so(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    f.load(&editor, DEPLOYMENT_YAML);
    f.vcx.simulate_keystrokes(&convert_to_yaml_key());
    f.vcx.run_until_parked();
    assert_eq!(f.text(&editor), DEPLOYMENT_YAML);
    let (level, text) = f.notice(&editor).expect("a notice");
    assert_eq!(level, NoticeLevel::Info);
    assert_eq!(text, "Already YAML.");
}
