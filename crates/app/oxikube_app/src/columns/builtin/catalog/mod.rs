//! The core column catalogue: ~40 kinds, table-driven.
//!
//! Column sets come from `docs/research/features-freelens-k9s.md` section A (the Freelens list
//! views, A1, and the k9s views, A2) and from the kubectl printers' `-o wide` columns. Each area
//! file names its sources above its table. Order is display order: default columns first, then
//! the `wide` ones, as `kubectl get -o wide` appends them.
//!
//! Every kind the [`FeedPolicy`](crate::store::FeedPolicy) serves from a reflector or metadata
//! feed has an entry here (a test keeps the two in step). Kinds read through the metadata feed
//! (ConfigMap, Secret, Lease) only list columns a metadata-only object can fill: their `data`,
//! `type` and `spec` are never cached (non-negotiable 5).

mod cluster;
mod network;
mod rbac;
mod storage;
mod workloads;

use super::def::KindDef;

/// Every area's kinds.
pub(super) const AREAS: &[&[KindDef]] = &[
    workloads::KINDS,
    cluster::KINDS,
    network::KINDS,
    storage::KINDS,
    rbac::KINDS,
];
