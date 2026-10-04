use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use oxikube_domain::ErrorKind;

use super::*;

fn quick() -> LivenessConfig {
    LivenessConfig {
        interval: Duration::from_secs(30),
        probe_timeout: Duration::from_secs(10),
        failure_threshold: 3,
        ..Default::default()
    }
}

fn ok() -> OxiResult<String> {
    Ok("v1.35.0".into())
}

fn transient() -> OxiResult<String> {
    Err(OxiError::auth("token expired", true))
}

/// A probe that pops scripted results (then repeats the last) and records when it ran.
fn scripted(
    results: Vec<OxiResult<String>>,
) -> (
    impl FnMut() -> std::future::Ready<OxiResult<String>> + Send + 'static,
    Arc<std::sync::Mutex<Vec<Instant>>>,
) {
    let times = Arc::new(std::sync::Mutex::new(Vec::new()));
    let t = times.clone();
    let mut i = 0;
    let probe = move || {
        t.lock().unwrap().push(Instant::now());
        let idx = i.min(results.len() - 1);
        i += 1;
        let r = match &results[idx] {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(OxiError::new(e.kind(), e.message()).with_retryable(e.is_retryable())),
        };
        std::future::ready(r)
    };
    (probe, times)
}

async fn next(rx: &mut mpsc::Receiver<HealthEvent>) -> HealthEvent {
    rx.recv().await.expect("event")
}

#[tokio::test(start_paused = true)]
async fn healthy_probes_repeat_every_interval() {
    let (probe, times) = scripted(vec![ok()]);
    let (_live, mut rx) = Liveness::spawn(quick(), probe);
    for _ in 0..3 {
        assert!(matches!(next(&mut rx).await, HealthEvent::Healthy { .. }));
    }
    let t = times.lock().unwrap();
    assert_eq!(t[1] - t[0], Duration::from_secs(30));
    assert_eq!(t[2] - t[1], Duration::from_secs(30));
}

#[tokio::test(start_paused = true)]
async fn failures_back_off_then_fail_and_the_loop_ends() {
    let (probe, times) = scripted(vec![transient()]);
    let (live, mut rx) = Liveness::spawn(quick(), probe);
    assert!(matches!(
        next(&mut rx).await,
        HealthEvent::Unhealthy {
            consecutive_failures: 1,
            ..
        }
    ));
    assert!(matches!(
        next(&mut rx).await,
        HealthEvent::Unhealthy {
            consecutive_failures: 2,
            ..
        }
    ));
    let failed = next(&mut rx).await;
    assert!(matches!(failed, HealthEvent::Failed { .. }), "{failed:?}");
    assert!(rx.recv().await.is_none(), "channel closes after Failed");
    assert!(live.is_finished());
    let t = times.lock().unwrap();
    assert_eq!(t.len(), 3);
    // backoff 2s then 4s, not the 30s interval
    assert_eq!(t[1] - t[0], Duration::from_secs(2));
    assert_eq!(t[2] - t[1], Duration::from_secs(4));
}

#[tokio::test(start_paused = true)]
async fn failure_delay_is_capped_at_the_interval() {
    let cfg = LivenessConfig {
        interval: Duration::from_secs(1),
        failure_threshold: 3,
        ..quick()
    };
    let (probe, times) = scripted(vec![transient()]);
    let (_live, mut rx) = Liveness::spawn(cfg, probe);
    while rx.recv().await.is_some() {}
    let t = times.lock().unwrap();
    assert_eq!(t[1] - t[0], Duration::from_secs(1));
    assert_eq!(t[2] - t[1], Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn recovery_returns_to_healthy_and_resets_backoff() {
    let (probe, times) = scripted(vec![transient(), transient(), ok(), transient(), ok()]);
    let (_live, mut rx) = Liveness::spawn(quick(), probe);
    let mut seen = vec![];
    for _ in 0..5 {
        seen.push(match next(&mut rx).await {
            HealthEvent::Healthy { .. } => "healthy",
            HealthEvent::Unhealthy { .. } => "unhealthy",
            HealthEvent::Failed { .. } => "failed",
        });
    }
    assert_eq!(
        seen,
        ["unhealthy", "unhealthy", "healthy", "unhealthy", "healthy"]
    );
    let t = times.lock().unwrap();
    assert_eq!(
        t[3] - t[2],
        Duration::from_secs(30),
        "interval after success"
    );
    assert_eq!(
        t[4] - t[3],
        Duration::from_secs(2),
        "backoff restarted at the minimum"
    );
}

#[tokio::test(start_paused = true)]
async fn non_retryable_auth_is_degraded_then_error_at_once() {
    let (probe, times) = scripted(vec![Err(OxiError::auth("token revoked", false))]);
    let (_live, mut rx) = Liveness::spawn(quick(), probe);
    assert!(matches!(next(&mut rx).await, HealthEvent::Unhealthy { .. }));
    let failed = next(&mut rx).await;
    assert!(matches!(&failed, HealthEvent::Failed { error } if error.kind() == ErrorKind::Auth));
    assert_eq!(times.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_probe_that_never_answers_times_out() {
    let cfg = LivenessConfig {
        probe_timeout: Duration::from_secs(5),
        ..quick()
    };
    let (_live, mut rx) = Liveness::spawn(cfg, || std::future::pending::<OxiResult<String>>());
    let started = Instant::now();
    match next(&mut rx).await {
        HealthEvent::Unhealthy { error, .. } => {
            assert_eq!(error.kind(), ErrorKind::Timeout);
            assert!(error.is_retryable());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(started.elapsed(), Duration::from_secs(5));
}

#[tokio::test(start_paused = true)]
async fn pause_stops_probing_and_resume_probes_immediately() {
    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();
    let probe = move || {
        c.fetch_add(1, Ordering::SeqCst);
        std::future::ready(ok())
    };
    let (live, mut rx) = Liveness::spawn(quick(), probe);
    next(&mut rx).await;
    live.pause();
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1, "no probes while paused");
    live.resume();
    next(&mut rx).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn pause_during_the_wait_holds_until_resume() {
    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();
    let probe = move || {
        c.fetch_add(1, Ordering::SeqCst);
        std::future::ready(ok())
    };
    let (live, mut rx) = Liveness::spawn(quick(), probe);
    next(&mut rx).await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    live.pause();
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    live.resume();
    next(&mut rx).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn set_interval_applies_to_the_current_wait() {
    let (probe, times) = scripted(vec![ok()]);
    let (live, mut rx) = Liveness::spawn(quick(), probe);
    next(&mut rx).await;
    live.set_interval(Duration::from_secs(5));
    next(&mut rx).await;
    let t = times.lock().unwrap();
    assert_eq!(t[1] - t[0], Duration::from_secs(5));
}

#[tokio::test(start_paused = true)]
async fn probe_now_skips_the_wait() {
    let (probe, times) = scripted(vec![ok()]);
    let (live, mut rx) = Liveness::spawn(quick(), probe);
    next(&mut rx).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    live.probe_now();
    next(&mut rx).await;
    let t = times.lock().unwrap();
    assert_eq!(t[1] - t[0], Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn dropping_the_handle_aborts_the_task() {
    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();
    let probe = move || {
        c.fetch_add(1, Ordering::SeqCst);
        std::future::ready(ok())
    };
    let (live, mut rx) = Liveness::spawn(quick(), probe);
    next(&mut rx).await;
    live.stop();
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert!(rx.recv().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn dropping_the_receiver_ends_the_loop() {
    let (probe, times) = scripted(vec![ok()]);
    let (live, rx) = Liveness::spawn(quick(), probe);
    drop(rx);
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert!(live.is_finished());
    assert_eq!(times.lock().unwrap().len(), 1);
}
