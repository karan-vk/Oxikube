//! [`EventRow`]: one event as the table draws it, its cell text worked out once, and the
//! [`EventColumn`]s.

use std::sync::Arc;

use gpui::SharedString;
use jiff::Timestamp;
use oxikube_app::events::EventKey;
use oxikube_domain::age::Age;
use oxikube_domain::event::{Event, EventType};

/// Longest message a row shows, in chars: rows are one line (the table is a `uniform_list`), and
/// the whole message is in the copy and the export.
const MESSAGE_CHARS: usize = 300;

/// The columns of the events view, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventColumn {
    /// `Normal` or `Warning`.
    Type,
    /// The short reason (`BackOff`).
    Reason,
    /// The namespace the event is stored in.
    Namespace,
    /// The involved object, `Kind/name`.
    Object,
    /// The reporting component (`kubelet`).
    Source,
    /// How many times it happened.
    Count,
    /// Since it was first seen.
    Age,
    /// Since it was last seen.
    LastSeen,
    /// The message, on one line.
    Message,
}

impl EventColumn {
    /// Every column, in table order.
    pub const ALL: [EventColumn; 9] = [
        EventColumn::Type,
        EventColumn::Reason,
        EventColumn::Namespace,
        EventColumn::Object,
        EventColumn::Source,
        EventColumn::Count,
        EventColumn::Age,
        EventColumn::LastSeen,
        EventColumn::Message,
    ];

    /// The column at `ix` of the table.
    pub fn at(ix: usize) -> Option<EventColumn> {
        Self::ALL.get(ix).copied()
    }

    /// The stable key of the column.
    pub const fn key(self) -> &'static str {
        match self {
            EventColumn::Type => "type",
            EventColumn::Reason => "reason",
            EventColumn::Namespace => "namespace",
            EventColumn::Object => "object",
            EventColumn::Source => "source",
            EventColumn::Count => "count",
            EventColumn::Age => "age",
            EventColumn::LastSeen => "last_seen",
            EventColumn::Message => "message",
        }
    }

    /// The header.
    pub const fn title(self) -> &'static str {
        match self {
            EventColumn::Type => "Type",
            EventColumn::Reason => "Reason",
            EventColumn::Namespace => "Namespace",
            EventColumn::Object => "Object",
            EventColumn::Source => "Source",
            EventColumn::Count => "Count",
            EventColumn::Age => "Age",
            EventColumn::LastSeen => "Last seen",
            EventColumn::Message => "Message",
        }
    }

    /// The design-time width, in pixels.
    pub const fn width(self) -> f32 {
        match self {
            EventColumn::Type => 80.,
            EventColumn::Reason => 150.,
            EventColumn::Namespace => 130.,
            EventColumn::Object => 220.,
            EventColumn::Source => 130.,
            EventColumn::Count => 60.,
            EventColumn::Age | EventColumn::LastSeen => 80.,
            EventColumn::Message => 600.,
        }
    }

    /// Whether the column is relative to now (redrawn as time passes).
    pub const fn is_relative(self) -> bool {
        matches!(self, EventColumn::Age | EventColumn::LastSeen)
    }

    /// The slot of a fixed column in [`EventRow`]'s text.
    const fn fixed_slot(self) -> Option<usize> {
        match self {
            EventColumn::Type => Some(0),
            EventColumn::Reason => Some(1),
            EventColumn::Namespace => Some(2),
            EventColumn::Object => Some(3),
            EventColumn::Source => Some(4),
            EventColumn::Count => Some(5),
            EventColumn::Message => Some(6),
            EventColumn::Age | EventColumn::LastSeen => None,
        }
    }
}

/// One event of the table: the event, its key, its place in the arrival order and the text of
/// its cells, worked out when the row is made (off the render path). The two relative cells
/// are cached per tick of the view's clock.
#[derive(Clone, Debug)]
pub struct EventRow {
    /// Order of first arrival: the tie-break of every sort.
    pub(crate) id: u64,
    /// The event's identity in the service.
    pub key: EventKey,
    /// The event as last seen.
    pub event: Arc<Event>,
    text: [SharedString; 7],
    relative: Option<(u64, [SharedString; 2])>,
}

impl EventRow {
    /// A row for `event`, arrival number `id`.
    pub fn new(id: u64, key: EventKey, event: Arc<Event>) -> Self {
        let text = [
            SharedString::new_static(type_label(event.event_type)),
            SharedString::from(event.reason.to_string()),
            SharedString::from(event.namespace.as_deref().unwrap_or("").to_owned()),
            SharedString::from(object_label(&event)),
            SharedString::from(
                event
                    .reporting_component
                    .as_deref()
                    .unwrap_or("")
                    .to_owned(),
            ),
            SharedString::from(event.count.to_string()),
            SharedString::from(one_line(&event.message, MESSAGE_CHARS)),
        ];
        Self {
            id,
            key,
            event,
            text,
            relative: None,
        }
    }

    /// Whether the event is a `Warning`.
    pub fn is_warning(&self) -> bool {
        self.event.event_type == EventType::Warning
    }

    /// The text of `column` at `now`. `epoch` names `now`: the relative cells are worked out
    /// once per epoch, so a frame that draws the same rows again formats nothing.
    pub fn cell(&mut self, column: EventColumn, now: Timestamp, epoch: u64) -> SharedString {
        if let Some(slot) = column.fixed_slot() {
            return self.text[slot].clone();
        }
        let cells = match &mut self.relative {
            Some((at, cells)) if *at == epoch => cells,
            slot => {
                let age = since(self.event.first_seen.or(self.event.last_seen), now);
                let last = since(self.event.last_seen.or(self.event.first_seen), now);
                &slot.insert((epoch, [age, last])).1
            }
        };
        match column {
            EventColumn::Age => cells[0].clone(),
            _ => cells[1].clone(),
        }
    }

    /// The text of a fixed column (empty for the relative ones).
    pub fn fixed(&self, column: EventColumn) -> &str {
        column.fixed_slot().map_or("", |slot| &self.text[slot])
    }
}

/// `Normal`, `Warning`, or `Other`.
pub const fn type_label(event_type: EventType) -> &'static str {
    match event_type {
        EventType::Normal => "Normal",
        EventType::Warning => "Warning",
        EventType::Other => "Other",
    }
}

/// `Kind/name` of the involved object.
pub fn object_label(event: &Event) -> String {
    format!("{}/{}", event.regarding.gvk.kind, event.regarding.name)
}

/// `3m` since `at` (kubectl's style); `-` without a time.
fn since(at: Option<Timestamp>, now: Timestamp) -> SharedString {
    at.map_or(SharedString::new_static("-"), |at| {
        SharedString::from(Age::between(at, now).to_string())
    })
}

/// `text` on one line (line breaks and tabs as spaces), at most `max` chars with an ellipsis.
pub fn one_line(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max.saturating_mul(4)));
    for (n, ch) in text.chars().enumerate() {
        if n == max {
            out.push('…');
            break;
        }
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    out
}
