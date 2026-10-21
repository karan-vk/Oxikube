//! The gate state: one notification per real change, the native reader flag, the global.

use std::cell::Cell;
use std::rc::Rc;

use gpui::TestAppContext;
use oxikube_domain::helm::{GateReason, HelmCapabilities, HelmFeature, HelmGate, HelmVersion};

use super::full;
use crate::{HelmGateChanged, HelmGateState};

#[gpui::test]
fn nothing_detected_is_unavailable_and_not_configured(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert_eq!(HelmGateState::current(cx), HelmGate::default());
        let state = HelmGateState::install(cx);
        assert_eq!(
            state.read(cx).gate(),
            &HelmGate::Unavailable {
                reason: GateReason::NotConfigured
            }
        );
        assert!(HelmFeature::ALL.iter().all(|f| !state.read(cx).allows(*f)));
        // Installing twice hands back the same state.
        assert_eq!(HelmGateState::install(cx), state);
    });
}

#[gpui::test]
fn detection_results_map_onto_the_gate(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let state = HelmGateState::install(cx);
        state.update(cx, |s, cx| s.set_native_read(true, cx));
        let cases = [
            (full(), HelmGate::Full),
            (
                HelmCapabilities::none(),
                HelmGate::NativeOnly {
                    reason: GateReason::BinaryMissing,
                },
            ),
            (
                HelmCapabilities {
                    cli_version: Some(HelmVersion::new(3, 8, 1)),
                    ..HelmCapabilities::none()
                },
                HelmGate::NativeOnly {
                    reason: GateReason::VersionTooOld {
                        found: "3.8.1".into(),
                        needed: "3.10.0".into(),
                    },
                },
            ),
        ];
        for (cli, expected) in cases {
            state.update(cx, |s, cx| s.set_cli_capabilities(cli, cx));
            assert_eq!(HelmGateState::current(cx), expected);
        }
    });
}

#[gpui::test]
fn without_a_native_reader_no_binary_leaves_nothing(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let state = HelmGateState::install(cx);
        state.update(cx, |s, cx| {
            s.set_cli_capabilities(HelmCapabilities::none(), cx)
        });
        assert_eq!(
            HelmGateState::current(cx),
            HelmGate::Unavailable {
                reason: GateReason::BinaryMissing
            }
        );
        state.update(cx, |s, cx| s.set_native_read(true, cx));
        assert!(matches!(
            HelmGateState::current(cx),
            HelmGate::NativeOnly { .. }
        ));
    });
}

#[gpui::test]
fn only_a_real_change_notifies_and_a_recheck_shows_while_it_runs(cx: &mut TestAppContext) {
    let notified = Rc::new(Cell::new(0));
    let changed = Rc::new(Cell::new(0));
    let state = cx.update(HelmGateState::install);
    let (n, c) = (notified.clone(), changed.clone());
    cx.update(|cx| {
        cx.observe(&state, move |_, _| n.set(n.get() + 1)).detach();
        cx.subscribe(&state, move |_, _: &HelmGateChanged, _| c.set(c.get() + 1))
            .detach();
    });
    let set = |cx: &mut TestAppContext, caps: HelmCapabilities| {
        cx.update(|cx| state.update(cx, |s, cx| s.set_cli_capabilities(caps, cx)));
    };
    set(cx, full());
    assert_eq!((notified.get(), changed.get()), (1, 1));
    set(cx, full());
    assert_eq!(
        (notified.get(), changed.get()),
        (1, 1),
        "the same answer is not a change"
    );

    cx.update(|cx| state.update(cx, |s, cx| s.begin_check(cx)));
    assert!(cx.update(|cx| state.read(cx).is_checking()));
    assert_eq!(notified.get(), 2, "the banner shows 'Checking...'");
    set(cx, full());
    assert!(!cx.update(|cx| state.read(cx).is_checking()));
    assert_eq!(notified.get(), 3, "the check ended");
    assert_eq!(changed.get(), 1, "but the gate did not change");

    set(cx, HelmCapabilities::none());
    assert_eq!(changed.get(), 2);
}
