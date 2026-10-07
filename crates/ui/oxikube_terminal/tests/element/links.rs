//! cmd/ctrl-hover shows a link, cmd/ctrl-click dispatches `terminal::OpenLink`.

use gpui::{Modifiers, TestAppContext};
use oxikube_domain::command::Command;

use super::{FONT_SIZE, Harness, harness, secondary};

#[gpui::test]
fn cmd_click_on_a_url_dispatches_the_open_command(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("docs at https://kubernetes.io/docs/ now");

    // Hover without the modifier: no link.
    h.window
        .simulate_mouse_move(Harness::at(0, 12), None, Modifiers::none());
    assert_eq!(h.state.hovered_link(), None);
    // With it: the URL under the pointer.
    h.window
        .simulate_mouse_move(Harness::at(0, 12), None, secondary());
    let hovered = h.state.hovered_link().expect("a hovered link");
    assert_eq!(hovered.target, "https://kubernetes.io/docs/");
    assert_eq!(hovered.cells, [(0, 8, 34)]);

    h.window.simulate_click(Harness::at(0, 12), secondary());
    assert_eq!(
        h.commands(),
        [Command::TerminalOpenLink {
            target: "https://kubernetes.io/docs/".into()
        }]
    );

    // A plain click selects; it opens nothing.
    h.window
        .simulate_click(Harness::at(0, 12), Modifiers::none());
    assert_eq!(h.commands().len(), 1);
    // Releasing the modifier drops the hover.
    h.window.simulate_modifiers_change(Modifiers::none());
    assert_eq!(h.state.hovered_link(), None);
}

#[gpui::test]
fn osc8_and_relative_paths_open_through_the_command(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output(
        "\x1b]8;;https://example.com/run/7\x1b\\run 7\x1b]8;;\x1b\\\r\nerror at src/main.rs:12:3",
    );
    h.window.simulate_click(Harness::at(0, 1), secondary());
    h.window.simulate_click(Harness::at(1, 12), secondary());
    // Nothing there: no command.
    h.window.simulate_click(Harness::at(5, 5), secondary());
    assert_eq!(
        h.commands(),
        [
            Command::TerminalOpenLink {
                target: "https://example.com/run/7".into()
            },
            Command::TerminalOpenLink {
                target: "/work/src/main.rs:12:3".into()
            },
        ]
    );
}

#[gpui::test]
fn the_hover_is_dropped_when_its_row_changes(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("see https://kubernetes.io/docs/\r\n$ ");
    h.window
        .simulate_mouse_move(Harness::at(0, 8), None, secondary());
    assert!(h.state.hovered_link().is_some());
    // Output elsewhere (the prompt row) keeps it.
    h.output("ls");
    assert!(h.state.hovered_link().is_some(), "another row changed");
    // Clearing the screen removes the link under the pointer: so goes the hover.
    h.output("\x1b[2J\x1b[H");
    assert_eq!(h.state.hovered_link(), None);
}
