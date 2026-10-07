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

## Amendment: the exec class (E09-S08)

A shell, attach or exec into a pod changes no object and takes no confirmation, but it is as powerful as the container's user, so it is neither a mutation nor a plain read. `pod::Shell`, `pod::Attach` and `pod::Exec` form the *exec class* (`CommandMeta::exec`): the guard blocks them on a read-only cluster unless that cluster's `exec_in_read_only` setting (default off) allows them, writes one audit record per open (initiator, pod, and a `detail` naming the session kind, container and program; never input, output or an exec's arguments) and fails closed when it cannot, and their MCP tool stubs are `risk: high`, `unsafe`, `interactive` and hidden from agents until the user enables them. A pod session is only started by its command, so a split, a retry or a layout restore never opens one around the guard (pod terminals are not restored).
