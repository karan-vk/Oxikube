# ADR 0002: Hexagonal workspace layout enforced by xtask

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/

## Context

Multiple agent teams build the app in parallel. Without hard boundaries, UI code grows kube-rs calls and the domain grows GPUI types, which blocks parallel work, testing with fakes, and swapping adapters.

## Decision

Crates are grouped by layer directory: crates/domain, crates/ports, crates/app, crates/adapters/*, crates/platform/*, crates/ui/*, crates/testing, bins/. Dependency direction: domain ← ports ← app ← (ui, bins); adapters implement ports and never depend on app/ui; platform depends on domain/ports; `oxikube_ui` is the only crate importing gpui-component. `cargo xtask lint-deps` encodes the table in docs/ARCHITECTURE.md and fails CI on violations.

## Consequences

More crates and some boilerplate (ports + fakes) per feature; in exchange, app logic is testable in milliseconds with `oxikube_testkit`, adapters are replaceable, and teams rarely collide on files. The lint is the contract; change the ADR and the lint together.
