# ADR 0006: Hybrid table feeds: typed/metadata reflectors for core kinds, server Table API for CRDs

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/

## Context

kube-rs has no Table API support. The server Table API (Accept: as=Table) gives kubectl-identical columns incl. CRD additionalPrinterColumns but is not a typed reflector (prior art sofka/kubetui re-poll). Typed reflectors give true watch semantics and computed columns (CPU, restarts).

## Decision

One `ColumnProvider` trait with two implementations: core kinds use kube-runtime reflectors (or metadata-only feeds) plus our column definitions; CRDs and unknown kinds use a hand-rolled Table list+watch feed (~150 lines over Client::request) with JSON fallback when the Accept header is ignored. `ResourceStore` hides the feed type from the UI.

## Consequences

Core views get live updates, sorting on computed columns and metrics columns; every CRD renders like kubectl. Two feed mechanisms to maintain; the watch budget treats them uniformly.

## Amendment (E12-S12, 2026-10-11)

Two kinds change feed in the per-kind table (`oxikube_app::store::policy`). **Leases** move from a metadata-only feed to a full reflector: a Lease holds no secrets, its holder is a list column (Freelens and k9s show it), and the event count is unchanged (a renewal bumps the resource version, so the metadata feed delivered the same events); only the payload grows, by a few hundred bytes per Lease, and the feed still starts only when the user opens Leases. The **VerticalPodAutoscaler** (an optional CRD) gets a full reflector instead of the Table feed so its Mode and Conditions columns come from our catalogue; the sidebar entry is gated by discovery, so nothing opens that feed where the CRD is absent.
