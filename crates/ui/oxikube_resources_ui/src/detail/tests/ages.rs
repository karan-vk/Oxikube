//! The age tick (E07-F566): a detail view that is shown redraws for ages only when an age it
//! draws moved (header chip, condition rows on the Overview, event rows on the Events tab), not
//! once a second.

use gpui::{Entity, TestAppContext};
use jiff::{SignedDuration, Timestamp};
use oxikube_domain::Resource;
use serde_json::json;

use super::fixture::{Detail, edited, pod_ref};
use crate::detail::ages::TICK;
use crate::detail::{DetailTab, DetailView};

fn ago(age: SignedDuration) -> String {
    (Timestamp::now() - age).to_string()
}

/// A pod `shop/web-0` created `created` ago whose one condition changed `condition` ago.
fn pod_aged(created: SignedDuration, condition: SignedDuration) -> Resource {
    let mut pod = edited(
        oxikube_testkit::pod()
            .namespace("shop")
            .name("web-0")
            .created(ago(created))
            .build(),
        |json| {
            json["status"]["conditions"] = json!([
                {"type": "Ready", "status": "True", "lastTransitionTime": ago(condition)}
            ]);
        },
    );
    pod.meta.resource_version = Some("1".into());
    pod
}

/// A `BackOff` event about `shop/web-0`, last seen `seen` ago.
fn event_aged(seen: SignedDuration) -> Resource {
    oxikube_testkit::resource("v1", "Event")
        .namespace("shop")
        .name("web-0.backoff")
        .field("type", json!("Warning"))
        .field("reason", json!("BackOff"))
        .field("message", json!("Back-off restarting failed container"))
        .field("lastTimestamp", json!(ago(seen)))
        .field(
            "involvedObject",
            json!({"apiVersion": "v1", "kind": "Pod", "name": "web-0",
                   "namespace": "shop", "uid": "u-web-0"}),
        )
        .build()
}

const DAYS_30: SignedDuration = SignedDuration::from_hours(30 * 24);

fn renders(d: &mut Detail, view: &Entity<DetailView>) -> usize {
    d.read(view, |v| v.renders)
}

/// One tick of the view's timer with the clock `by` further on.
fn tick(d: &mut Detail, view: &Entity<DetailView>, by: SignedDuration) {
    d.f.vcx.update(|_, cx| view.update(cx, |v, _| v.skew += by));
    d.f.vcx.executor().advance_clock(TICK);
    d.f.vcx.run_until_parked();
}

fn open(
    cx: &mut TestAppContext,
    objects: impl IntoIterator<Item = Resource>,
) -> (Detail, Entity<DetailView>) {
    let mut d = Detail::new(cx, objects);
    let view = d.open(&pod_ref("web-0"));
    d.settle();
    (d, view)
}

#[gpui::test]
fn a_still_detail_of_an_old_object_is_not_redrawn_each_second(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx, [pod_aged(DAYS_30, DAYS_30)]);
    let before = renders(&mut d, &view);
    for _ in 0..30 {
        tick(&mut d, &view, SignedDuration::from_secs(1));
    }
    assert_eq!(renders(&mut d, &view), before, "30d is still 30d");
}

#[gpui::test]
fn the_detail_redraws_once_when_the_day_rolls_over(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx, [pod_aged(DAYS_30, DAYS_30)]);
    let before = renders(&mut d, &view);
    tick(&mut d, &view, SignedDuration::from_hours(24));
    assert_eq!(renders(&mut d, &view), before + 1);
    tick(&mut d, &view, SignedDuration::from_secs(1));
    assert_eq!(renders(&mut d, &view), before + 1, "and it is still again");
}

#[gpui::test]
fn a_young_object_ticks_seconds(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx, [pod_aged(SignedDuration::from_secs(20), DAYS_30)]);
    let before = renders(&mut d, &view);
    for n in 1..=5 {
        tick(&mut d, &view, SignedDuration::from_secs(1));
        assert_eq!(renders(&mut d, &view), before + n, "second {n}");
    }
}

#[gpui::test]
fn a_young_condition_ticks_on_the_overview_only(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx, [pod_aged(DAYS_30, SignedDuration::from_secs(20))]);
    let before = renders(&mut d, &view);
    for n in 1..=3 {
        tick(&mut d, &view, SignedDuration::from_secs(1));
        assert_eq!(renders(&mut d, &view), before + n, "Overview, second {n}");
    }
    // The Events tab draws no condition: its seconds are not shown, nothing to redraw.
    d.update(&view, |v, cx| v.set_tab(DetailTab::Events, cx));
    let before = renders(&mut d, &view);
    for _ in 0..5 {
        tick(&mut d, &view, SignedDuration::from_secs(1));
    }
    assert_eq!(renders(&mut d, &view), before);
}

#[gpui::test]
fn a_young_event_ticks_on_the_events_tab_only(cx: &mut TestAppContext) {
    let (mut d, view) = open(
        cx,
        [
            pod_aged(DAYS_30, DAYS_30),
            event_aged(SignedDuration::from_secs(20)),
        ],
    );
    let before = renders(&mut d, &view);
    for _ in 0..5 {
        tick(&mut d, &view, SignedDuration::from_secs(1));
    }
    assert_eq!(
        renders(&mut d, &view),
        before,
        "no event drawn on the Overview"
    );

    d.update(&view, |v, cx| v.set_tab(DetailTab::Events, cx));
    assert_eq!(d.read(&view, |v| v.event_rows().len()), 1);
    let before = renders(&mut d, &view);
    for n in 1..=3 {
        tick(&mut d, &view, SignedDuration::from_secs(1));
        assert_eq!(renders(&mut d, &view), before + n, "Events, second {n}");
    }
}

#[gpui::test]
fn a_detail_that_is_not_shown_never_redraws_for_ages(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx, [pod_aged(SignedDuration::from_secs(20), DAYS_30)]);
    d.update(&view, |v, cx| v.set_shown(false, cx));
    let before = renders(&mut d, &view);
    for _ in 0..5 {
        tick(&mut d, &view, SignedDuration::from_secs(1));
    }
    assert_eq!(renders(&mut d, &view), before);
}

#[gpui::test]
fn a_pinned_clock_never_redraws_for_ages(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx, [pod_aged(SignedDuration::from_secs(20), DAYS_30)]);
    d.update(&view, |v, cx| v.pin_now(Timestamp::now(), cx));
    let before = renders(&mut d, &view);
    for _ in 0..5 {
        tick(&mut d, &view, SignedDuration::from_secs(1));
    }
    assert_eq!(renders(&mut d, &view), before, "screenshots stay still");
}
