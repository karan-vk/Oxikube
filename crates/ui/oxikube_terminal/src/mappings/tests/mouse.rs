//! The reports a click, drag and wheel turn into.

use gpui::Modifiers;

use crate::grid::TerminalModes;
use crate::mappings::mouse::{MouseButton, MouseEvent, MouseKind, WheelDirection};
use crate::mappings::{encode_mouse, should_report, wheel_arrows};

const SGR: TerminalModes = TerminalModes::MOUSE_REPORT_CLICK.union(TerminalModes::SGR_MOUSE);

fn event(kind: MouseKind, column: usize, row: usize) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: Modifiers::none(),
    }
}

fn encode(event: MouseEvent, modes: TerminalModes) -> Option<Vec<u8>> {
    encode_mouse(&event, modes).map(|bytes| bytes.as_bytes().to_vec())
}

#[test]
fn sgr_click_press_and_release() {
    let press = event(MouseKind::Press(MouseButton::Left), 0, 0);
    assert_eq!(encode(press, SGR).unwrap(), b"\x1b[<0;1;1M");
    let release = event(MouseKind::Release(MouseButton::Left), 9, 4);
    assert_eq!(encode(release, SGR).unwrap(), b"\x1b[<0;10;5m");
    let middle = event(MouseKind::Press(MouseButton::Middle), 1, 1);
    assert_eq!(encode(middle, SGR).unwrap(), b"\x1b[<1;2;2M");
    let right = event(MouseKind::Press(MouseButton::Right), 2, 3);
    assert_eq!(encode(right, SGR).unwrap(), b"\x1b[<2;3;4M");
}

#[test]
fn sgr_has_no_coordinate_limit() {
    let far = event(MouseKind::Press(MouseButton::Left), 1999, 499);
    assert_eq!(encode(far, SGR).unwrap(), b"\x1b[<0;2000;500M");
}

#[test]
fn sgr_drag_motion_and_modifiers() {
    let modes = TerminalModes::MOUSE_DRAG | TerminalModes::SGR_MOUSE;
    let drag = event(MouseKind::Drag(MouseButton::Left), 5, 2);
    assert_eq!(encode(drag, modes).unwrap(), b"\x1b[<32;6;3M");
    let all = TerminalModes::MOUSE_MOTION | TerminalModes::SGR_MOUSE;
    let motion = event(MouseKind::Motion, 5, 2);
    assert_eq!(encode(motion, all).unwrap(), b"\x1b[<35;6;3M");
    let mut press = event(MouseKind::Press(MouseButton::Left), 0, 0);
    press.modifiers = Modifiers {
        shift: true,
        alt: true,
        control: true,
        ..Modifiers::none()
    };
    assert_eq!(encode(press, SGR).unwrap(), b"\x1b[<28;1;1M");
}

#[test]
fn sgr_wheel() {
    for (direction, code) in [
        (WheelDirection::Up, 64),
        (WheelDirection::Down, 65),
        (WheelDirection::Left, 66),
        (WheelDirection::Right, 67),
    ] {
        let wheel = event(MouseKind::Wheel(direction), 3, 7);
        assert_eq!(
            encode(wheel, SGR).unwrap(),
            format!("\x1b[<{code};4;8M").into_bytes()
        );
    }
}

#[test]
fn the_legacy_encoding_is_one_byte_per_value_and_limited() {
    let modes = TerminalModes::MOUSE_REPORT_CLICK;
    let press = event(MouseKind::Press(MouseButton::Left), 9, 4);
    assert_eq!(
        encode(press, modes).unwrap(),
        [0x1b, b'[', b'M', 32, 42, 37]
    );
    // The legacy form cannot say which button came up.
    let release = event(MouseKind::Release(MouseButton::Right), 0, 0);
    assert_eq!(
        encode(release, modes).unwrap(),
        [0x1b, b'[', b'M', 35, 33, 33]
    );
    let edge = event(MouseKind::Press(MouseButton::Left), 222, 0);
    assert!(encode(edge, modes).is_some());
    let beyond = event(MouseKind::Press(MouseButton::Left), 223, 0);
    assert_eq!(encode(beyond, modes), None, "does not fit one byte");
}

#[test]
fn the_utf8_encoding_reaches_further() {
    let modes = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::UTF8_MOUSE;
    let far = event(MouseKind::Press(MouseButton::Left), 300, 0);
    let bytes = encode(far, modes).unwrap();
    assert_eq!(&bytes[..3], b"\x1b[M");
    assert_eq!(std::str::from_utf8(&bytes[3..]).unwrap(), "\u{20}\u{14d}!");
    let beyond = event(MouseKind::Press(MouseButton::Left), 3000, 0);
    assert_eq!(encode(beyond, modes), None);
}

#[test]
fn only_the_events_the_process_asked_for_are_reported() {
    let press = MouseKind::Press(MouseButton::Left);
    let drag = MouseKind::Drag(MouseButton::Left);
    let wheel = MouseKind::Wheel(WheelDirection::Up);
    assert!(
        !should_report(press, TerminalModes::empty()),
        "mouse mode off"
    );
    let click = TerminalModes::MOUSE_REPORT_CLICK;
    assert!(should_report(press, click) && should_report(wheel, click));
    assert!(!should_report(drag, click) && !should_report(MouseKind::Motion, click));
    let dragging = TerminalModes::MOUSE_DRAG;
    assert!(should_report(drag, dragging) && !should_report(MouseKind::Motion, dragging));
    let motion = TerminalModes::MOUSE_MOTION;
    assert!(should_report(drag, motion) && should_report(MouseKind::Motion, motion));
    assert_eq!(encode(event(drag, 0, 0), click), None);
}

#[test]
fn alternate_scroll_sends_cursor_keys() {
    let none = TerminalModes::empty();
    assert_eq!(wheel_arrows(true, none), "\x1b[A");
    assert_eq!(wheel_arrows(false, none), "\x1b[B");
    assert_eq!(wheel_arrows(true, TerminalModes::APP_CURSOR), "\x1bOA");
    assert_eq!(wheel_arrows(false, TerminalModes::APP_CURSOR), "\x1bOB");
}
