//! Unit tests. The pure parts (job template cloning, revision selection, drain filtering) are
//! tested on plain values and on the testkit's `FakeResourcePort`; the whole path through
//! [`KubeResources`](crate::KubeResources) runs against a scripted in-process API server, which
//! pins the requests (path, method, content type, query, body) and the retry timing.

mod cronjob;
mod drain_plan;
mod drain_retry;
mod drain_run;
mod drain_support;
mod harness;
mod rollout;
mod rollout_wire;
