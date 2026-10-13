//! The limits that keep an agent's events small and a call short.

use std::time::Duration;

/// Events `k8s.get_events` returns when `limit` is not given.
pub const DEFAULT_LIMIT: usize = 30;
/// Most events `k8s.get_events` returns; the rest are counted, not listed.
pub const MAX_LIMIT: usize = 100;
/// Longest event message in a `k8s.get_events` row, in characters (longer ones end in `…`).
pub const MAX_MESSAGE_CHARS: usize = 240;
/// Most events one `@events` mention quotes.
pub const MAX_MENTION_EVENTS: usize = 100;
/// Longest event message in an `@events` block or a sent block, in characters.
pub const MAX_BLOCK_MESSAGE_CHARS: usize = 480;
/// Most events "Send to agent" queues in one block.
pub const MAX_SENT_EVENTS: usize = 200;
/// How long a call waits for a feed it just started to list the cluster's events.
pub(super) const WARM_DEADLINE: Duration = Duration::from_secs(5);
/// How long `@events` waits for the object it names to be read.
pub(super) const READ_DEADLINE: Duration = Duration::from_secs(10);
