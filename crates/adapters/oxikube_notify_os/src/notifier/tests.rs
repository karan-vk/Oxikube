//! `OsNotifier` against a scripted backend: no OS is called.

use std::sync::mpsc;
use std::time::Duration;

use futures::executor::block_on;
use oxikube_ports::{NotificationLevel, is_unavailable};

use super::*;
use crate::Shown;

/// A backend that records what it is asked and lets a test play the user.
#[derive(Default)]
struct Scripted {
    connect_error: Mutex<Option<BackendError>>,
    show_error: Mutex<Option<BackendError>>,
    connects: AtomicUsize,
    shows: Mutex<Vec<(Payload, bool)>>,
    detached: AtomicUsize,
    /// One sender per notification that waits for the user, in show order.
    users: Mutex<Vec<mpsc::Sender<Response>>>,
}

impl Scripted {
    fn respond(&self, index: usize, response: Response) {
        self.users.lock()[index].send(response).unwrap();
    }
}

struct ScriptedShown {
    id: u32,
    user: Option<mpsc::Receiver<Response>>,
    backend: Arc<Scripted>,
}

impl Shown for ScriptedShown {
    fn id(&self) -> Option<u32> {
        Some(self.id)
    }

    fn wait(self: Box<Self>) -> Response {
        self.user
            .as_ref()
            .and_then(|user| user.recv_timeout(Duration::from_secs(5)).ok())
            .unwrap_or(Response::Closed)
    }

    fn detach(self: Box<Self>) {
        self.backend.detached.fetch_add(1, Ordering::SeqCst);
    }
}

impl Backend for Arc<Scripted> {
    fn connect(&self) -> Result<(), BackendError> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        self.connect_error.lock().clone().map_or(Ok(()), Err)
    }

    fn show(&self, payload: &Payload, clickable: bool) -> Result<Box<dyn Shown>, BackendError> {
        if let Some(error) = self.show_error.lock().clone() {
            return Err(error);
        }
        let mut shows = self.shows.lock();
        shows.push((payload.clone(), clickable));
        let user = clickable.then(|| {
            let (tx, rx) = mpsc::channel();
            self.users.lock().push(tx);
            rx
        });
        Ok(Box::new(ScriptedShown {
            // Like a freedesktop server: a replacement keeps the id it replaces.
            id: payload.replaces.unwrap_or(100 + shows.len() as u32),
            user,
            backend: self.clone(),
        }))
    }
}

fn setup() -> (OsNotifier, Arc<Scripted>) {
    let backend = Arc::new(Scripted::default());
    (OsNotifier::with_backend(Arc::new(backend.clone())), backend)
}

fn warning(tag: &str) -> Notification {
    Notification::new(
        "Deployment api",
        "Back-off restarting",
        NotificationLevel::Warning,
    )
    .with_tag(tag)
}

fn target() -> oxikube_domain::ids::ResourceRef {
    "3f2a9c01b7d4e865/apps/v1/Deployment/prod/api"
        .parse()
        .unwrap()
}

#[test]
fn off_by_default_touches_nothing() {
    let (notifier, backend) = setup();
    assert_eq!(notifier.status(), NotifierStatus::Off);
    block_on(notifier.notify(&warning("a"))).unwrap();
    assert_eq!(
        backend.connects.load(Ordering::SeqCst),
        0,
        "no OS contact before opt-in"
    );
    assert!(backend.shows.lock().is_empty());
}

#[test]
fn turning_on_connects_once_and_turning_off_silences() {
    let (notifier, backend) = setup();
    assert_eq!(
        block_on(notifier.set_enabled(true)).unwrap(),
        NotifierStatus::Ready
    );
    assert_eq!(
        block_on(notifier.set_enabled(true)).unwrap(),
        NotifierStatus::Ready
    );
    assert_eq!(backend.connects.load(Ordering::SeqCst), 1);

    block_on(notifier.notify(&warning("a"))).unwrap();
    assert_eq!(backend.shows.lock().len(), 1);

    assert_eq!(
        block_on(notifier.set_enabled(false)).unwrap(),
        NotifierStatus::Off
    );
    block_on(notifier.notify(&warning("a"))).unwrap();
    assert_eq!(
        backend.shows.lock().len(),
        1,
        "off again: nothing more is shown"
    );
}

#[test]
fn no_service_is_reported_not_raised() {
    let (notifier, backend) = setup();
    *backend.connect_error.lock() = Some(BackendError::Unavailable("no daemon".into()));
    let status = block_on(notifier.set_enabled(true)).unwrap();
    assert_eq!(status, NotifierStatus::Unavailable("no daemon".into()));
    let error = block_on(notifier.notify(&warning("a"))).unwrap_err();
    assert!(is_unavailable(&error), "{error}");
    assert!(backend.shows.lock().is_empty());

    // The user can try again once a daemon is running.
    *backend.connect_error.lock() = None;
    assert_eq!(
        block_on(notifier.set_enabled(true)).unwrap(),
        NotifierStatus::Ready
    );
}

#[test]
fn a_service_that_goes_away_flips_the_status() {
    let (notifier, backend) = setup();
    block_on(notifier.set_enabled(true)).unwrap();
    *backend.show_error.lock() = Some(BackendError::Unavailable("bus closed".into()));
    assert!(is_unavailable(
        &block_on(notifier.notify(&warning("a"))).unwrap_err()
    ));
    assert_eq!(
        notifier.status(),
        NotifierStatus::Unavailable("bus closed".into())
    );

    let (notifier, backend) = setup();
    block_on(notifier.set_enabled(true)).unwrap();
    *backend.show_error.lock() = Some(BackendError::Failed("refused".into()));
    let error = block_on(notifier.notify(&warning("a"))).unwrap_err();
    assert!(!is_unavailable(&error));
    assert_eq!(notifier.status(), NotifierStatus::Ready);
}

#[test]
fn a_click_reports_the_target_and_a_dismissal_does_not() {
    let (notifier, backend) = setup();
    let (clicks, seen) = mpsc::channel();
    notifier.on_activated(Arc::new(move |activation| {
        clicks.send(activation).unwrap();
    }));
    block_on(notifier.set_enabled(true)).unwrap();

    let first = warning("a").with_action_target(target());
    block_on(notifier.notify(&first)).unwrap();
    block_on(notifier.notify(&warning("b"))).unwrap();
    backend.respond(1, Response::Closed);
    assert!(
        seen.recv_timeout(Duration::from_millis(200)).is_err(),
        "a dismissal is not a click"
    );
    backend.respond(0, Response::Clicked);
    let activation = seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(activation, first.activation());
    assert_eq!(activation.action_target, Some(target()));
    assert_eq!(activation.cluster, Some(target().cluster));
}

#[test]
fn the_same_tag_replaces_until_cleared() {
    let (notifier, backend) = setup();
    block_on(notifier.set_enabled(true)).unwrap();
    block_on(notifier.notify(&warning("a"))).unwrap();
    block_on(notifier.notify(&warning("a"))).unwrap();
    block_on(notifier.notify(&warning("b"))).unwrap();
    block_on(notifier.clear("a")).unwrap();
    block_on(notifier.notify(&warning("a"))).unwrap();
    let replaces: Vec<_> = backend
        .shows
        .lock()
        .iter()
        .map(|(p, _)| p.replaces)
        .collect();
    assert_eq!(replaces, vec![None, Some(101), None, None]);
}

#[test]
fn a_replacement_shares_the_waiter_so_one_click_activates_once() {
    let (notifier, backend) = setup();
    let (clicks, seen) = mpsc::channel();
    notifier.on_activated(Arc::new(move |activation| {
        clicks.send(activation).unwrap();
    }));
    block_on(notifier.set_enabled(true)).unwrap();

    let first = warning("x").with_action_target(target());
    let newer = warning("x").with_action_target(
        "3f2a9c01b7d4e865/apps/v1/Deployment/prod/web"
            .parse()
            .unwrap(),
    );
    block_on(notifier.notify(&first)).unwrap();
    block_on(notifier.notify(&newer)).unwrap();
    {
        let shows = backend.shows.lock();
        assert_eq!(shows[1].0.replaces, Some(101), "replaced in place");
        assert!(shows[1].1, "the replacement still carries the click action");
    }
    assert_eq!(
        notifier.inner.waiting.load(Ordering::SeqCst),
        1,
        "one waiter, one slot, for the one notification on screen"
    );

    // Every thread waiting on the id would see this click.
    let senders: Vec<_> = backend.users.lock().clone();
    for user in senders {
        // The replacement's own channel has no reader: only a second waiter would answer.
        let _ = user.send(Response::Clicked);
    }
    let activation = seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(activation, newer.activation(), "the notification on screen");
    assert!(
        seen.recv_timeout(Duration::from_millis(200)).is_err(),
        "activated once"
    );
}

#[test]
fn re_sending_a_tag_does_not_use_up_the_waiting_slots() {
    let (notifier, backend) = setup();
    block_on(notifier.set_enabled(true)).unwrap();
    for _ in 0..MAX_WAITING * 2 {
        block_on(notifier.notify(&warning("same"))).unwrap();
    }
    assert_eq!(notifier.inner.waiting.load(Ordering::SeqCst), 1);
    block_on(notifier.notify(&warning("other"))).unwrap();
    assert!(
        backend.shows.lock().last().unwrap().1,
        "a different notification is still clickable"
    );
}

#[test]
fn what_leaves_the_process_is_redacted_and_capped() {
    let (notifier, backend) = setup();
    block_on(notifier.set_enabled(true)).unwrap();
    let noisy = Notification::new(
        "x".repeat(300),
        format!("token=abcdef1234567890abcdef0123456789 {}", "y".repeat(600)),
        NotificationLevel::Error,
    );
    block_on(notifier.notify(&noisy)).unwrap();
    let (payload, _) = backend.shows.lock()[0].clone();
    assert!(payload.title.chars().count() <= oxikube_ports::MAX_TITLE_CHARS);
    assert!(payload.body.chars().count() <= oxikube_ports::MAX_BODY_CHARS);
    assert!(!payload.body.contains("abcdef1234567890abcdef0123456789"));
}

#[test]
fn waiting_threads_are_bounded() {
    let (notifier, backend) = setup();
    block_on(notifier.set_enabled(true)).unwrap();
    for i in 0..MAX_WAITING + 3 {
        block_on(notifier.notify(&warning(&format!("n{i}")))).unwrap();
    }
    let clickable = backend.shows.lock().iter().filter(|(_, c)| *c).count();
    assert_eq!(clickable, MAX_WAITING);
    // The detaching threads run after the call returned: give them a moment.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while backend.detached.load(Ordering::SeqCst) < 3 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(backend.detached.load(Ordering::SeqCst), 3);

    // Freeing slots lets later notifications be clickable again.
    for i in 0..MAX_WAITING {
        backend.respond(i, Response::Closed);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while notifier.inner.waiting.load(Ordering::SeqCst) > 0 && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    block_on(notifier.notify(&warning("again"))).unwrap();
    assert!(backend.shows.lock().last().unwrap().1);
}
