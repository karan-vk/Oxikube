//! Node tests (E12-S09) over `oxikube_testkit` fakes: cordon and uncordon through the real
//! `CommandBus` and `MutationGuard` (`commands`), the node table's columns (`columns`), and the
//! field-selector fallback for the pods on a node (`pods`).

mod columns;
mod commands;
mod pods;
