//! [`Shared`]: a small value an adapter reads on every run and the settings can replace while
//! the app runs (hot reload).

use std::sync::{Arc, Mutex, PoisonError};

/// A cloneable handle to one value. Clones see each other's [`set`](Self::set).
#[derive(Debug)]
pub(crate) struct Shared<T>(Arc<Mutex<T>>);

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Clone> Shared<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(Arc::new(Mutex::new(value)))
    }

    /// The value now.
    pub(crate) fn get(&self) -> T {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the value; runs that started earlier keep the one they read.
    pub(crate) fn set(&self, value: T) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = value;
    }
}
