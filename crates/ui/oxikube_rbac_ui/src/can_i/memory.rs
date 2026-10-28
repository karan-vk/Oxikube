//! [`SessionMemory`]: what the dialog remembers between openings.

use std::cell::RefCell;
use std::rc::Rc;

use super::dialog::SubjectKind;

#[derive(Debug, Default)]
struct Inner {
    verb: Option<String>,
    subject_kind: SubjectKind,
    subject: String,
}

/// The last verb and subject asked, kept in memory for the session. Nothing here reaches a file,
/// the state store or a log: an impersonation target is never persisted. Cheap to clone; clones
/// share it.
#[derive(Debug, Clone, Default)]
pub struct SessionMemory(Rc<RefCell<Inner>>);

impl SessionMemory {
    /// An empty memory.
    pub fn new() -> Self {
        Self::default()
    }

    /// The last verb asked.
    pub fn verb(&self) -> Option<String> {
        self.0.borrow().verb.clone()
    }

    /// The last subject kind and name asked.
    pub fn subject(&self) -> (SubjectKind, String) {
        let inner = self.0.borrow();
        (inner.subject_kind, inner.subject.clone())
    }

    pub(super) fn remember(&self, verb: &str, kind: SubjectKind, subject: &str) {
        let mut inner = self.0.borrow_mut();
        inner.verb = Some(verb.to_owned());
        inner.subject_kind = kind;
        inner.subject = subject.to_owned();
    }
}
