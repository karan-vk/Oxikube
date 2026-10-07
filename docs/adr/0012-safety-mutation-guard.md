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

## Amendment: debug containers (E09-S10)

`pod::Debug` adds an ephemeral container to a running pod (`kubectl debug`) and opens a terminal in it. Unlike a shell it *does* change an object (it patches `pods/ephemeralcontainers`, and the container can be neither removed nor edited until the pod is deleted), so it is a mutation, not exec-class: `Risk::Low`, a simple confirmation whose text names the pod, image and target and says the container is permanent, blocked on a read-only cluster for every initiator (`exec_in_read_only` is for shells and does not apply), and audited with a `detail` of `session=debug image=... target=... program=...` (the program only, never arguments). It needs `exec` as well as `mutate`; its tool stub is `unsafe`, `interactive` and hidden from agents like the exec tools (`CommandMeta::interactive`). The only door to the patch is `ExecService::open_debug(&Mutation, ..)`, which takes the guard's permit, and re-checks read-only just before the call as the guard's writer does for a delete. A dry-run dispatch validates against the pod it reads and stops: the `ExecPort` has no server-side dry run of the subresource patch.
