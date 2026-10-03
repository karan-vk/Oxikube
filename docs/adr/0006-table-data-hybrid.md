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
