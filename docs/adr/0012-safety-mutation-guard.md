# ADR 0012: Every mutation passes through MutationGuard; read-only clusters; audit log

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/

## Context

Oxikube will be pointed at production clusters and will host agents that can request mutations. Lens and k9s offer at most a read-only flag; neither audits.

## Decision

`oxikube_app::MutationGuard` is the only path to mutating port methods (newtype wrapper + lint): per-cluster read-only mode (also blocks MCP mutation tools), confirmation tiers scaled to blast radius ('type the name' for namespaces/nodes/PVs/cascading deletes), server-side dry-run diff before apply/edit, and an append-only local audit log (who/what/where/when/outcome/initiator ui|command|agent|plugin). Cluster tabs are colour-coded. Secrets are redacted everywhere.

## Consequences

Some friction for power users (configurable tiers); agents cannot bypass the UI's safety. Every action story must state its guard behaviour.
