//! [`NotificationCentre`]: the window's view of the app's `NotificationService`, the model the
//! panel, the status-bar badge and the toasts read. It derives the rows once per change, off the
//! render path, and redraws at most once a frame (`notify_coalesced`). A row whose notification
//! did not change is reused (the service replaces a changed notification's `Arc`, and the cache
//! holds one, so a pointer compare is exact): a burst formats only the rows it touched.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{Context, EventEmitter, SharedString, Task};
use oxikube_app::ClusterSessionManager;
use oxikube_app::notifications::{Notification, NotificationId, NotificationService};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_ports::NotificationLevel;
use oxikube_runtime::NotifyCoalescedExt as _;

/// One row as the panel and a toast show it, made when the list changes.
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationRow {
    /// The notification's id.
    pub id: NotificationId,
    /// How urgent it is.
    pub level: NotificationLevel,
    /// The headline.
    pub title: SharedString,
    /// The latest detail.
    pub body: SharedString,
    /// The cluster it concerns.
    pub cluster: ClusterId,
    /// What the cluster is called (its display name, else its context).
    pub cluster_label: SharedString,
    /// The count and time line: `×12 · 14:03:07`.
    pub meta: SharedString,
    /// Whether it was read.
    pub read: bool,
    /// The object a click opens.
    pub target: Option<ResourceRef>,
}

impl NotificationRow {
    fn of(note: &Notification, sessions: &ClusterSessionManager, tz: &jiff::tz::TimeZone) -> Self {
        let time = note
            .updated
            .to_zoned(tz.clone())
            .strftime("%H:%M:%S")
            .to_string();
        let meta = if note.count > 1 {
            format!("\u{d7}{} \u{b7} {time}", note.count)
        } else {
            time
        };
        Self {
            id: note.id,
            level: note.level,
            title: note.title.clone().into(),
            body: note.body.clone().into(),
            cluster: note.cluster.clone(),
            cluster_label: label_of(sessions, &note.cluster).into(),
            meta: meta.into(),
            read: note.read,
            target: note.target.clone(),
        }
    }

    /// What opening the row dispatches, in order: mark it read, select its cluster, open its
    /// object (the same commands a click on the OS notification runs).
    pub fn open_commands(&self) -> Vec<Command> {
        let mut commands = vec![Command::NotificationsMarkRead { id: self.id.0 }];
        commands.extend(
            oxikube_app::ClickAction {
                cluster: Some(self.cluster.clone()),
                open: self.target.clone(),
            }
            .commands(),
        );
        commands
    }
}

/// A cluster's display name while its session is open, else its id.
fn label_of(sessions: &ClusterSessionManager, cluster: &ClusterId) -> String {
    sessions
        .get(cluster)
        .map(|session| session.title().to_owned())
        .unwrap_or_else(|| cluster.to_string())
}

/// What the centre tells the window besides redrawing.
#[derive(Clone, Debug, PartialEq)]
pub enum CentreEvent {
    /// These rows should toast now (oldest first); `more` were raised besides them.
    Raised {
        /// The rows to toast.
        rows: Vec<NotificationRow>,
        /// How many more were raised in the same poll.
        more: usize,
    },
}

/// The window's model of the notification centre. See the [module docs](self).
pub struct NotificationCentre {
    service: NotificationService,
    sessions: ClusterSessionManager,
    rows: Rc<[NotificationRow]>,
    /// The rows of the last refresh by id, with the notification each was made from.
    cache: HashMap<NotificationId, (Arc<Notification>, NotificationRow)>,
    unread: usize,
    muted: Vec<ClusterId>,
    _pump: Task<()>,
}

impl EventEmitter<CentreEvent> for NotificationCentre {}

impl NotificationCentre {
    /// The centre over `service`, naming clusters through `sessions`.
    pub fn new(
        service: NotificationService,
        sessions: ClusterSessionManager,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut changes = service.subscribe();
        // Stored, never dropped from inside: it ends with the entity.
        let pump = cx.spawn(async move |this, cx| {
            while let Some(change) = changes.next().await {
                let alive = this.update(cx, |this, cx| {
                    if change.changed {
                        this.refresh();
                        cx.notify_coalesced();
                    }
                    if !change.raised.is_empty() {
                        let tz = jiff::tz::TimeZone::system();
                        let rows = change
                            .raised
                            .iter()
                            .map(|note| NotificationRow::of(note, &this.sessions, &tz))
                            .collect();
                        cx.emit(CentreEvent::Raised {
                            rows,
                            more: change.more_raised,
                        });
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        });
        let mut centre = Self {
            service,
            sessions,
            rows: Rc::from(Vec::new()),
            cache: HashMap::new(),
            unread: 0,
            muted: Vec::new(),
            _pump: pump,
        };
        centre.refresh();
        centre
    }

    fn refresh(&mut self) {
        let notes = self.service.list();
        let tz = jiff::tz::TimeZone::system();
        let mut cache = HashMap::with_capacity(notes.len());
        let rows: Vec<NotificationRow> = notes
            .into_iter()
            .map(|note| {
                let row = match self.cache.remove(&note.id) {
                    Some((made_from, row)) if Arc::ptr_eq(&made_from, &note) => row,
                    _ => NotificationRow::of(&note, &self.sessions, &tz),
                };
                cache.insert(note.id, (note, row.clone()));
                row
            })
            .collect();
        self.cache = cache;
        self.rows = rows.into();
        self.unread = self.service.unread();
        self.muted = self.service.muted();
    }

    /// The rows, newest first.
    pub fn rows(&self) -> Rc<[NotificationRow]> {
        self.rows.clone()
    }

    /// How many rows are unread.
    pub fn unread(&self) -> usize {
        self.unread
    }

    /// Whether `cluster` is muted.
    pub fn is_muted(&self, cluster: &ClusterId) -> bool {
        self.muted.contains(cluster)
    }

    /// The muted clusters.
    pub fn muted(&self) -> &[ClusterId] {
        &self.muted
    }

    /// What `cluster` is called.
    pub fn cluster_label(&self, cluster: &ClusterId) -> SharedString {
        label_of(&self.sessions, cluster).into()
    }

    /// The service behind it.
    pub fn service(&self) -> &NotificationService {
        &self.service
    }
}
