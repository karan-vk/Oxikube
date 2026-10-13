//! [`Indexes`]: the secondary indexes of an [`EventBuffer`](super::EventBuffer), kept in step
//! with its ring on every insert and removal.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use oxikube_domain::event::Event;

use crate::events::key::Order;

/// The events about one object: their positions (newest last) and how many are warnings.
#[derive(Debug, Default)]
pub(crate) struct ObjectEvents {
    pub(crate) orders: BTreeSet<Order>,
    pub(crate) warnings: usize,
}

/// Positions by involved-object uid, by namespace, and of the warnings. A set never stays
/// behind empty: removing its last position removes it, so no index holds a stale key.
#[derive(Debug, Default)]
pub(crate) struct Indexes {
    pub(crate) by_object: HashMap<Arc<str>, ObjectEvents>,
    pub(crate) by_namespace: HashMap<Arc<str>, BTreeSet<Order>>,
    pub(crate) warnings: BTreeSet<Order>,
}

impl Indexes {
    /// Records `event` at `order`.
    pub(crate) fn insert(&mut self, order: Order, event: &Event) {
        let warning = event.is_warning();
        if let Some(uid) = &event.regarding_uid {
            let object = self.by_object.entry(uid.clone()).or_default();
            object.orders.insert(order);
            object.warnings += usize::from(warning);
        }
        if let Some(namespace) = &event.namespace {
            self.by_namespace
                .entry(namespace.clone())
                .or_default()
                .insert(order);
        }
        if warning {
            self.warnings.insert(order);
        }
    }

    /// Forgets `event` at `order` (it must have been inserted there).
    pub(crate) fn remove(&mut self, order: Order, event: &Event) {
        let warning = event.is_warning();
        if let Some(uid) = &event.regarding_uid
            && let Some(object) = self.by_object.get_mut(uid)
        {
            object.orders.remove(&order);
            object.warnings -= usize::from(warning && object.warnings > 0);
            if object.orders.is_empty() {
                self.by_object.remove(uid);
            }
        }
        if let Some(namespace) = &event.namespace
            && let Some(orders) = self.by_namespace.get_mut(namespace)
        {
            orders.remove(&order);
            if orders.is_empty() {
                self.by_namespace.remove(namespace);
            }
        }
        if warning {
            self.warnings.remove(&order);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.by_object.clear();
        self.by_namespace.clear();
        self.warnings.clear();
    }
}
