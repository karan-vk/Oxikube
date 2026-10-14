//! The toolbar's filters: typing narrows the rows in the same frame, `ctrl-z` toggles the faults,
//! a bad regex is shown while the last good rows stay, the other fields, and the filters saved
//! per cluster.

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_ports::StatePortExt as _;
use oxikube_testkit::{FakeEventFeed, FakeStatePort};

use super::{Fx, cluster, seed};
use crate::filter::{EventsFilters, TypeChoice};
use crate::persist::filters_key;
use crate::view::Field;

#[gpui::test]
fn typing_in_the_text_filter_narrows_the_rows_within_a_frame(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.open();
    assert_eq!(fx.uids(), ["w2", "n2", "w1", "n1"]);
    fx.keys("/");
    fx.vcx.simulate_input("backoff");
    // One pass of the executor: the read off the UI thread lands before the next frame.
    fx.vcx.run_until_parked();
    assert_eq!(fx.uids(), ["w1"]);
    let view = fx.view().unwrap();
    let typed = fx
        .vcx
        .update(|_, cx| view.read(cx).fields.text.read(cx).value().to_string());
    assert_eq!(typed, "backoff");
    // A regex, case-insensitive.
    fx.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.set_field(Field::Text, "^(started|pulled)$".into(), cx)
        });
    });
    fx.vcx.run_until_parked();
    assert_eq!(fx.uids(), ["n2", "n1"]);
}

#[gpui::test]
fn ctrl_z_shows_only_the_faults_and_again_every_event(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.open();
    fx.keys("ctrl-z");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::EventsToggleFaults { cluster: cluster() })
    );
    assert_eq!(fx.uids(), ["w2", "w1"]);
    let view = fx.view().unwrap();
    assert!(fx.vcx.update(|_, cx| view.read(cx).filters().faults_only));
    fx.keys("ctrl-z");
    assert_eq!(fx.uids(), ["w2", "n2", "w1", "n1"]);
    // The toolbar's toggle runs the same command.
    fx.click("events-faults");
    assert_eq!(fx.uids(), ["w2", "w1"]);
}

#[gpui::test]
fn an_invalid_regex_is_shown_and_the_rows_stay(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open();
    let set = |fx: &mut Fx, text: &str| {
        let text = text.to_owned();
        fx.vcx.update(|_, cx| {
            view.update(cx, |v, cx| v.set_field(Field::Text, text, cx));
        });
        fx.settle();
    };
    set(&mut fx, "pod");
    assert_eq!(fx.uids().len(), 4, "every event is about a pod");
    set(&mut fx, "back(off");
    let error = fx
        .vcx
        .update(|_, cx| view.read(cx).filter_error().map(str::to_owned));
    assert!(error.is_some_and(|e| !e.is_empty()));
    assert!(fx.drawn("events-filter-error"));
    assert_eq!(fx.uids().len(), 4, "the last good filter's rows stay");
    set(&mut fx, "back(off)?");
    assert!(!fx.drawn("events-filter-error"));
    assert_eq!(fx.uids(), ["w1"]);
}

#[gpui::test]
fn the_toggles_still_apply_while_the_text_is_a_bad_regex(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open();
    let apply =
        |fx: &mut Fx, f: &dyn Fn(&mut crate::EventsView, &mut gpui::Context<crate::EventsView>)| {
            fx.vcx.update(|_, cx| view.update(cx, |v, cx| f(v, cx)));
            fx.settle();
        };
    apply(&mut fx, &|v, cx| v.set_field(Field::Text, "web".into(), cx));
    assert_eq!(fx.uids(), ["w1", "n1"]);
    apply(&mut fx, &|v, cx| {
        v.set_field(Field::Text, "back(".into(), cx)
    });
    assert!(fx.drawn("events-filter-error"));
    assert_eq!(fx.uids(), ["w1", "n1"], "the last good text stays in force");
    // The faults toggle the toolbar shows is the one the rows follow.
    fx.keys("ctrl-z");
    assert!(fx.vcx.update(|_, cx| view.read(cx).filters().faults_only));
    assert_eq!(fx.uids(), ["w1"]);
    fx.keys("ctrl-z");
    apply(&mut fx, &|v, cx| v.set_types(TypeChoice::Normal, cx));
    assert_eq!(fx.uids(), ["n1"]);
    assert!(fx.drawn("events-filter-error"), "the text is still bad");
    // Fixing the text applies it with the toggles.
    apply(&mut fx, &|v, cx| v.set_field(Field::Text, "dns".into(), cx));
    assert!(!fx.drawn("events-filter-error"));
    assert_eq!(fx.uids(), ["n2"]);
}

#[gpui::test]
fn type_reason_kind_and_namespace_narrow_together(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open();
    let apply =
        |fx: &mut Fx, f: &dyn Fn(&mut crate::EventsView, &mut gpui::Context<crate::EventsView>)| {
            fx.vcx.update(|_, cx| view.update(cx, |v, cx| f(v, cx)));
            fx.settle();
        };
    apply(&mut fx, &|v, cx| {
        v.set_field(Field::Namespace, "default, shop".into(), cx)
    });
    assert_eq!(fx.uids(), ["w2", "w1", "n1"]);
    apply(&mut fx, &|v, cx| v.set_types(TypeChoice::Normal, cx));
    assert_eq!(fx.uids(), ["n1"]);
    apply(&mut fx, &|v, cx| v.set_types(TypeChoice::All, cx));
    apply(&mut fx, &|v, cx| {
        v.set_field(Field::Reason, "BackOff FailedScheduling".into(), cx)
    });
    assert_eq!(fx.uids(), ["w2", "w1"]);
    apply(&mut fx, &|v, cx| {
        v.set_field(Field::Kind, "node".into(), cx)
    });
    assert!(fx.uids().is_empty());
    assert!(fx.drawn("events-table"));
}

#[gpui::test]
fn the_filters_are_saved_per_cluster_and_come_back(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    {
        let mut fx = Fx::build(cx, FakeEventFeed::new().initial(seed()), state.clone());
        let view = fx.open();
        fx.keys("ctrl-z");
        fx.vcx.update(|_, cx| {
            view.update(cx, |v, cx| v.set_field(Field::Namespace, "shop".into(), cx));
        });
        fx.settle();
        assert_eq!(fx.uids(), ["w2"]);
    }
    let key = filters_key(&cluster()).unwrap();
    let saved: serde_json::Value = futures::executor::block_on(state.kv_get_as(&key))
        .unwrap()
        .expect("saved");
    assert_eq!(saved["filters"]["faults_only"], true);
    assert_eq!(saved["filters"]["namespace"], "shop");

    let mut fx = Fx::build(cx, FakeEventFeed::new().initial(seed()), state);
    let view = fx.open();
    assert_eq!(
        fx.vcx.update(|_, cx| view.read(cx).filters().clone()),
        EventsFilters {
            faults_only: true,
            namespace: "shop".into(),
            ..EventsFilters::default()
        }
    );
    assert_eq!(fx.uids(), ["w2"]);
}

#[gpui::test]
fn open_with_a_namespace_and_text_applies_them(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.run(Command::EventsOpen {
        cluster: cluster(),
        namespace: Some("default".into()),
        filter: Some("backoff".into()),
    });
    assert_eq!(fx.uids(), ["w1"]);
    // Opening again shows the same view.
    fx.open();
    let workspace = fx.workspace.clone();
    let open = fx.vcx.update(|_, cx| {
        workspace
            .read(cx)
            .items_of_type::<crate::EventsView>()
            .len()
    });
    assert_eq!(open, 1);
}
