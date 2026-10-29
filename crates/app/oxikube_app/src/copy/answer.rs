//! The confirmation answers a copy gives the guard: the dry run's and the review's.

use std::sync::Arc;

use async_trait::async_trait;

use crate::guard::{ConfirmationProvider, ConfirmationRequest, Decision};

/// Answers the confirmation of a copy's dry run: the click, and the expected name for a
/// type-the-name tier. The dry run changes nothing.
pub(super) struct DryRunAnswer;

#[async_trait]
impl ConfirmationProvider for DryRunAnswer {
    async fn confirm(&self, request: &ConfirmationRequest) -> Decision {
        match &request.expected_name {
            Some(name) => Decision::typed(name.clone()),
            None => Decision::confirm(),
        }
    }
}

/// Answers the guard's confirmation of the real copy from the review: the click, and for a
/// type-the-name tier what the user typed (the guard compares it).
#[derive(Debug, Clone, Default)]
pub struct ReviewAnswer {
    typed: Option<String>,
}

impl ReviewAnswer {
    /// The review's click.
    pub fn click() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The click and the name the user typed.
    pub fn typed(name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            typed: Some(name.into()),
        })
    }
}

#[async_trait]
impl ConfirmationProvider for ReviewAnswer {
    async fn confirm(&self, request: &ConfirmationRequest) -> Decision {
        match (&request.expected_name, &self.typed) {
            (Some(_), Some(typed)) => Decision::typed(typed.clone()),
            (Some(_), None) => Decision::Decline,
            (None, _) => Decision::confirm(),
        }
    }
}
