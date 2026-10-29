//! The commands that open the panel: on the bus with tool stubs, one panel per tab, the editor
//! hand-off with the source line, and the keyboard.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{self, Capabilities, Command, CommandId};

use super::{CM_A, CM_B, Rig, seed};
use crate::table::tests::fixture::cluster;

#[test]
fn the_commands_are_declared_read_class_and_kustomize_needs_its_capability() {
    let directory = command::lookup(CommandId::APPLY_DIRECTORY).expect("declared");
    let kustomize = command::lookup(CommandId::APPLY_KUSTOMIZE).expect("declared");
    assert!(
        !directory.mutating && !kustomize.mutating,
        "opening the panel changes nothing"
    );
    assert!(directory.needs.is_empty());
    assert_eq!(kustomize.needs, Capabilities::KUSTOMIZE);
    assert_eq!(
        CommandId::APPLY_DIRECTORY.tool_name(),
        "k8s.apply_directory"
    );
    assert_eq!(
        CommandId::APPLY_KUSTOMIZE.tool_name(),
        "k8s.apply_kustomize"
    );
}

#[gpui::test]
fn the_commands_are_on_the_bus_with_tool_stubs_and_open_the_panel_in_the_tab(
    cx: &mut TestAppContext,
) {
    let mut rig = Rig::new(cx, None);
    seed(&rig, "/m", &[("a.yaml", CM_A)]);
    for (id, tool) in [
        (CommandId::APPLY_DIRECTORY, "k8s.apply_directory"),
        (CommandId::APPLY_KUSTOMIZE, "k8s.apply_kustomize"),
    ] {
        assert_eq!(
            rig.bus.owner(id),
            Some("oxikube_resources_ui::apply"),
            "{id}"
        );
        assert_eq!(
            rig.bus.tool(id).map(|t| t.name.as_str()),
            Some(tool),
            "{id}"
        );
    }
    block_on(rig.bus.dispatch(
        Command::ApplyDirectory {
            cluster: cluster(),
            path: Some("/m".into()),
            namespace: Some("shop".into()),
        },
        DispatchContext::new(Initiator::Ui, "me"),
    ))
    .expect("dispatched");
    rig.settle();
    assert_eq!(rig.words(), ["created"]);
    assert_eq!(
        rig.read(|v| v.plan().unwrap().target.namespace.clone())
            .as_deref(),
        Some("shop")
    );
    let ws = rig.workspace();
    let panels = rig
        .f
        .vcx
        .read(|cx| ws.read(cx).items_of_type::<crate::apply::ApplyView>().len());
    assert_eq!(panels, 1);
    // Asking again shows the open panel and loads into it.
    block_on(rig.bus.dispatch(
        Command::ApplyDirectory {
            cluster: cluster(),
            path: Some("/m".into()),
            namespace: None,
        },
        DispatchContext::new(Initiator::Ui, "me"),
    ))
    .expect("dispatched");
    rig.settle();
    let panels = rig
        .f
        .vcx
        .read(|cx| ws.read(cx).items_of_type::<crate::apply::ApplyView>().len());
    assert_eq!(panels, 1, "one panel per tab");
}

#[gpui::test]
fn a_command_without_a_path_opens_the_file_picker_and_loads_the_choice(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    seed(&rig, "/m", &[("a.yaml", CM_A)]);
    rig.request(crate::apply::SourceKind::Manifests, None);
    assert!(
        cx.did_prompt_for_paths(),
        "the platform picker was asked for"
    );
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && options.directories, "a file or a folder");
        assert!(!options.multiple);
        Some(vec![std::path::PathBuf::from("/m")])
    });
    rig.settle();
    assert_eq!(rig.words(), ["created"]);
}

#[gpui::test]
fn open_in_editor_hands_over_the_file_text_at_the_documents_line(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    let two = format!("{CM_A}---\n{CM_B}");
    seed(&rig, "/m", &[("both.yaml", &two)]);
    rig.open("/m");
    let lines: Vec<usize> = rig.read(|v| v.model().rows().iter().map(|r| r.line).collect());
    assert_eq!(
        lines,
        [1, 8],
        "the second document starts at line 8 of the file"
    );
    rig.click("apply-open-1");
    let opened = rig.opened.borrow().clone();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].title, "both.yaml");
    assert_eq!(opened[0].line, 8);
    assert_eq!(&*opened[0].text, two.as_str());
    assert_eq!(opened[0].cluster, cluster());
    let debug = format!("{:?}", opened[0]);
    assert!(
        !debug.contains("kind: ConfigMap"),
        "the text is not in Debug: {debug}"
    );
}

#[gpui::test]
fn the_keys_move_expand_filter_and_open(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    seed(&rig, "/m", &[("a.yaml", CM_A), ("b.yaml", CM_B)]);
    rig.f
        .ports()
        .resources
        .script()
        .patch
        .push_err(oxikube_domain::OxiError::validation(
            "ConfigMap \"a\" is invalid",
        ));
    rig.open("/m");
    let focus = rig.read(|v| v.focus.clone());
    rig.f.vcx.update(|window, cx| window.focus(&focus, cx));
    rig.settle();
    rig.f.vcx.simulate_keystrokes("j");
    rig.settle();
    assert_eq!(rig.read(|v| v.model().selected()), Some(0));
    rig.f.vcx.simulate_keystrokes("j");
    assert_eq!(rig.read(|v| v.model().selected()), Some(1));
    rig.f.vcx.simulate_keystrokes("k");
    assert_eq!(rig.read(|v| v.model().selected()), Some(0));
    rig.f.vcx.simulate_keystrokes("enter");
    rig.settle();
    assert!(
        rig.read(|v| v.model().is_expanded(0)),
        "enter opens the failed row"
    );
    rig.f.vcx.simulate_keystrokes("e");
    rig.settle();
    assert!(rig.read(|v| v.model().errors_only()));
    rig.f.vcx.simulate_keystrokes("o");
    rig.settle();
    assert_eq!(
        rig.opened.borrow().len(),
        1,
        "o opens the selected row in the editor"
    );
}
