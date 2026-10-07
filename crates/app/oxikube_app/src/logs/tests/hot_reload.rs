//! `logs.buffer_lines`: the bound, its clamp, and how a change reaches open sessions.

use std::sync::Arc;

use super::{Harness, burst};
use crate::logs::{
    DEFAULT_BUFFER_LINES, LogConfig, MAX_BUFFER_LINES, MIN_BUFFER_LINES, clamp_buffer_lines,
};

fn with_lines(lines: usize) -> Harness {
    Harness::with_config(LogConfig {
        buffer_lines: lines,
        ..LogConfig::default()
    })
}

#[test]
fn the_default_bound_is_the_documented_one() {
    let h = Harness::new();
    assert_eq!(h.service.buffer_lines(), DEFAULT_BUFFER_LINES);
}

#[test]
fn the_bound_is_clamped() {
    assert_eq!(clamp_buffer_lines(0), MIN_BUFFER_LINES);
    assert_eq!(clamp_buffer_lines(5), MIN_BUFFER_LINES);
    assert_eq!(clamp_buffer_lines(1_000), 1_000);
    assert_eq!(clamp_buffer_lines(usize::MAX), MAX_BUFFER_LINES);
    assert_eq!(with_lines(3).service.buffer_lines(), MIN_BUFFER_LINES);
}

#[test]
fn a_session_keeps_at_most_the_bound() {
    let mut h = with_lines(500);
    let session = h.follow_flushed(burst(1_200).keep_open());
    assert_eq!(session.len(), 500);
    session.read(|buffer, _| {
        assert!(buffer.is_truncated());
        assert_eq!(buffer.dropped(), 700);
        assert_eq!(buffer.first_seq(), 700);
    });
}

#[test]
fn a_smaller_bound_trims_open_sessions_at_once() {
    let mut h = with_lines(1_000);
    let session = h.follow_flushed(burst(800).keep_open());
    assert_eq!(session.len(), 800);
    let mut deltas = session.deltas();

    h.service.set_buffer_lines(300);
    assert_eq!(h.service.buffer_lines(), 300);
    assert_eq!(session.len(), 300);
    session.read(|buffer, _| assert_eq!(buffer.first_seq(), 500));
    // The consumer is told the front moved.
    let waker = futures::task::noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    let futures::task::Poll::Ready(Some(delta)) =
        futures::Stream::poll_next(std::pin::Pin::new(&mut deltas), &mut cx)
    else {
        panic!("no delta");
    };
    assert_eq!(delta.first_seq, 500);
}

#[test]
fn a_larger_bound_applies_to_what_arrives_next_and_to_new_sessions() {
    let mut h = with_lines(200);
    let session = h.follow_flushed(burst(200).keep_open());
    h.service.set_buffer_lines(5_000);
    assert_eq!(
        session.len(),
        200,
        "growing never brings dropped lines back"
    );
    session.read(|buffer, _| assert_eq!(buffer.capacity(), 5_000));
    let next = h.follow_flushed(burst(1_000).keep_open());
    assert_eq!(next.len(), 1_000);
}

#[test]
fn a_setting_below_the_floor_is_clamped_when_applied() {
    let h = with_lines(1_000);
    h.service.set_buffer_lines(1);
    assert_eq!(h.service.buffer_lines(), MIN_BUFFER_LINES);
}

#[test]
fn a_commit_never_overwrites_a_newer_bound_with_an_older_one() {
    use crate::logs::shared::Shared;
    use crate::logs::{LogEntry, LogTarget};
    use oxikube_ports::LogOptions;
    use std::sync::atomic::{AtomicUsize, Ordering};

    for _ in 0..500 {
        let shared = Arc::new(Shared::new(
            1,
            LogTarget::pod("default", "web-0"),
            LogOptions::default(),
            1_500,
        ));
        let setting = Arc::new(AtomicUsize::new(1_500));
        let committer = {
            let (shared, setting) = (shared.clone(), setting.clone());
            std::thread::spawn(move || {
                for i in 0..20 {
                    shared.commit(vec![LogEntry::new(super::line(i))], &setting);
                }
            })
        };
        // What `LogService::set_buffer_lines` does: store the setting, then apply it.
        setting.store(400, Ordering::Release);
        shared.set_capacity(400);
        committer.join().unwrap();
        // A commit after the change reads 400, one before it was trimmed: both end at 400.
        shared.read(|buffer, _| assert_eq!(buffer.capacity(), 400));
    }
}

mod per_cluster {
    use oxikube_domain::ids::{ClusterId, ContextName};
    use oxikube_ports::LogOptions;

    use super::super::{Harness, burst};
    use super::with_lines;
    use crate::logs::{LogSession, LogTarget, MIN_BUFFER_LINES};

    fn cluster(name: &str) -> ClusterId {
        ClusterId::new("/home/me/.kube/config", &ContextName::new(name))
    }

    fn open_in(h: &mut Harness, cluster: &ClusterId, lines: usize) -> LogSession {
        h.port
            .script()
            .stream_logs
            .push_ok(burst(lines).keep_open());
        let session = h.service.open_in(
            cluster,
            h.port.clone(),
            LogTarget::pod("default", "web-0"),
            LogOptions::follow(),
        );
        h.settle();
        h.advance(crate::logs::LogConfig::default().flush_interval);
        session
    }

    #[test]
    fn a_cluster_with_its_own_bound_keeps_that_many_lines() {
        let mut h = with_lines(1_000);
        let (prod, dev) = (cluster("prod"), cluster("dev"));
        h.service.set_cluster_buffer_lines([(prod.clone(), 400)]);
        assert_eq!(h.service.buffer_lines_for(&prod), 400);
        assert_eq!(h.service.buffer_lines_for(&dev), 1_000);
        assert_eq!(h.service.buffer_lines(), 1_000);

        let on_prod = open_in(&mut h, &prod, 900);
        let on_dev = open_in(&mut h, &dev, 900);
        assert_eq!(on_prod.len(), 400);
        assert_eq!(on_dev.len(), 900);
    }

    #[test]
    fn changing_a_cluster_bound_resizes_its_open_sessions_only() {
        let mut h = with_lines(1_000);
        let (prod, dev) = (cluster("prod"), cluster("dev"));
        let on_prod = open_in(&mut h, &prod, 800);
        let on_dev = open_in(&mut h, &dev, 800);

        h.service.set_cluster_buffer_lines([(prod.clone(), 300)]);
        assert_eq!(on_prod.len(), 300, "shrunk at once");
        assert_eq!(on_dev.len(), 800, "untouched");

        // The default moves: the cluster with its own bound ignores it, the others follow.
        h.service.set_buffer_lines(500);
        assert_eq!(on_prod.len(), 300);
        assert_eq!(on_dev.len(), 500);
        on_dev.read(|buffer, _| assert_eq!(buffer.capacity(), 500));
        on_prod.read(|buffer, _| assert_eq!(buffer.capacity(), 300));
    }

    #[test]
    fn dropping_the_override_goes_back_to_the_default() {
        let mut h = with_lines(1_000);
        let prod = cluster("prod");
        h.service.set_cluster_buffer_lines([(prod.clone(), 200)]);
        let on_prod = open_in(&mut h, &prod, 100);
        on_prod.read(|buffer, _| assert_eq!(buffer.capacity(), 200));

        h.service.set_cluster_buffer_lines([]);
        assert_eq!(h.service.buffer_lines_for(&prod), 1_000);
        on_prod.read(|buffer, _| assert_eq!(buffer.capacity(), 1_000));

        // And it follows the default again.
        h.service.set_buffer_lines(700);
        on_prod.read(|buffer, _| assert_eq!(buffer.capacity(), 700));
        let later = open_in(&mut h, &prod, 10);
        later.read(|buffer, _| assert_eq!(buffer.capacity(), 700));
    }

    #[test]
    fn a_cluster_bound_is_clamped() {
        let h = with_lines(1_000);
        let prod = cluster("prod");
        h.service.set_cluster_buffer_lines([(prod.clone(), 2)]);
        assert_eq!(h.service.buffer_lines_for(&prod), MIN_BUFFER_LINES);
    }
}
