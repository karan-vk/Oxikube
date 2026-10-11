//! `suspend_patch`: the body that suspends or resumes a Job or a CronJob.

use serde_json::{Value, json};

/// The JSON merge patch (RFC 7386) that sets `spec.suspend`: `true` suspends, `false` resumes.
///
/// Both `batch` kinds take it unchanged: a CronJob stops creating Jobs while suspended, and a
/// Job's controller deletes its running pods until it is resumed (its start time is reset).
pub fn suspend_patch(suspend: bool) -> Value {
    json!({"spec": {"suspend": suspend}})
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::suspend_patch;

    #[test]
    fn the_patch_sets_only_spec_suspend() {
        assert_eq!(suspend_patch(true), json!({"spec": {"suspend": true}}));
        assert_eq!(suspend_patch(false), json!({"spec": {"suspend": false}}));
    }
}
