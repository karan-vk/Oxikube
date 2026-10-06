//! The notify path under churn (E07-S09): however many deltas the feed delivers inside one frame,
//! the table applies them in one update and redraws once.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::{Timeline, pod};

use super::fixture::Fixture;

const PODS: usize = 200;
const DELTAS: usize = 1_000;

fn pod_at(i: usize, version: usize) -> Resource {
    let mut r = pod()
        .namespace("load")
        .name(format!("pod-{i:05}"))
        .restarts(u32::try_from(version).unwrap_or(0))
        .build();
    r.meta.resource_version = Some(version.to_string().into());
    r
}

#[gpui::test]
fn a_thousand_deltas_inside_one_frame_render_once(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    // The relist, then a thousand watch batches of one event each, all inside one frame: 900
    // modifies, 50 deletes and 50 new pods.
    let relist = DeltaBatch::from_deltas(vec![Delta::Restarted(
        (0..PODS).map(|i| pod_at(i, 1)).collect(),
    )]);
    let burst = Duration::from_secs(1);
    let timeline = (0..DELTAS).fold(
        Timeline::new().ok_at(Duration::ZERO, relist),
        |timeline, k| {
            let delta = match k % 20 {
                0 => Delta::Deleted(pod_at(k / 20, 1)),
                1 => Delta::Applied(pod_at(PODS + k / 20, 2)),
                _ => Delta::Applied(pod_at(PODS / 2 + k % (PODS / 2), k + 2)),
            };
            timeline.ok_at(burst, DeltaBatch::from_deltas(vec![delta]))
        },
    );
    f.ports()
        .resources
        .script()
        .watch
        .push_ok(timeline.keep_open());
    f.connect_with([]);
    let table = f.open_pods();
    let rows = |f: &mut Fixture| {
        f.vcx
            .update(|_, cx| table.read(cx).read_rows(cx, |d| d.rows().len()))
    };
    assert_eq!(rows(&mut f), PODS);

    let renders_before = f.vcx.update(|_, cx| table.read(cx).renders);
    let notified = Rc::new(Cell::new(0));
    let count = notified.clone();
    let _observer = f
        .vcx
        .update(|_, cx| cx.observe(&table, move |_, _| count.set(count.get() + 1)));
    // The feed delivers the burst; the coalesced notify is still a frame away.
    f.ports().resources.clock().advance(burst);
    f.vcx.run_until_parked();
    let pending = f.vcx.update(|_, cx| {
        oxikube_runtime::notify_pending(cx, table.entity_id()).then(|| table.read(cx).renders)
    });
    assert_eq!(
        pending,
        Some(renders_before),
        "the burst was applied and its redraw is waiting for the frame"
    );
    // The frame.
    f.vcx
        .executor()
        .advance_clock(oxikube_runtime::FRAME_INTERVAL);
    f.vcx.run_until_parked();
    let renders = f.vcx.update(|_, cx| table.read(cx).renders) - renders_before;
    assert_eq!(renders, 1, "{DELTAS} deltas in one frame: one render");
    assert_eq!(notified.get(), 1, "and one notify");
    assert_eq!(rows(&mut f), PODS, "50 deleted, 50 added");
    let names = f.names(&table);
    assert!(names.iter().any(|n| n == &format!("pod-{:05}", PODS + 49)));
    assert!(!names.iter().any(|n| n == "pod-00000"));
}
