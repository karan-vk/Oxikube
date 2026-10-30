//! Receipts for a person's own dry run of an apply (E19-S07 audit fix).
//!
//! The editor's apply runs twice through the guard: `ApplyService::preview` dispatches
//! `resource::Apply` as a dry run and shows the person the server's answer, then
//! `ApplyService::apply` dispatches it for real and answers the guard's simple confirmation
//! itself. The guard previews every `resource::Apply` before it asks (see `preview`), so without
//! a receipt that second dispatch would dry-run the apply once more (a live read, a server-side
//! apply dry run, another live read, an audit record) for a preview nobody sees.
//!
//! So when a person at the UI dry-runs a `resource::Apply` through the guard and the server
//! accepts it, the guard keeps a receipt; the first real dispatch of the same request by the same
//! person on the same cluster takes it in place of the rehearsal. Only the guard writes receipts,
//! each one is used once, and an agent's or a plugin's dry run never makes one: the person who
//! confirms an agent's apply always sees the server's diff in the dialog.

use std::collections::VecDeque;
use std::sync::Arc;

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ClusterId;
use parking_lot::Mutex;

use crate::command_bus::DispatchContext;

/// Most receipts kept (a batch preview of many documents keeps one each); the oldest is forgotten
/// first.
const MAX_RECEIPTS: usize = 32;

/// One accepted dry run.
#[derive(Debug)]
struct Receipt {
    who: Arc<str>,
    cluster: ClusterId,
    command: Command,
}

/// The receipts of accepted dry runs, newest last. See the [module docs](self).
#[derive(Debug, Default)]
pub(super) struct Receipts(Mutex<VecDeque<Receipt>>);

impl Receipts {
    /// The command to keep a receipt for if the dry-run dispatch `ctx` of `command` succeeds:
    /// `Some` for a person's dry run of a `resource::Apply` only.
    pub(super) fn eligible(command: &Command, ctx: &DispatchContext) -> Option<Command> {
        (ctx.dry_run && ctx.initiator == Initiator::Ui && command.id() == CommandId::RESOURCE_APPLY)
            .then(|| command.clone())
    }

    /// Keeps a receipt for `command`, dry-run by `who` on `cluster`.
    pub(super) fn record(&self, who: Arc<str>, cluster: ClusterId, command: Command) {
        let mut receipts = self.0.lock();
        receipts.retain(|r| {
            !(r.who == who && r.cluster == cluster && same_request(&r.command, &command))
        });
        while receipts.len() >= MAX_RECEIPTS {
            receipts.pop_front();
        }
        receipts.push_back(Receipt {
            who,
            cluster,
            command,
        });
    }

    /// Takes the receipt for the real dispatch `ctx` of `command` on `cluster`, when the same
    /// person dry-ran the same request there. Never for a dry-run dispatch or another initiator.
    pub(super) fn take(
        &self,
        command: &Command,
        ctx: &DispatchContext,
        cluster: &ClusterId,
    ) -> bool {
        if ctx.dry_run || ctx.initiator != Initiator::Ui {
            return false;
        }
        let mut receipts = self.0.lock();
        let found = receipts.iter().rposition(|r| {
            r.who == ctx.who && r.cluster == *cluster && same_request(&r.command, command)
        });
        found.and_then(|ix| receipts.remove(ix)).is_some()
    }
}

/// Whether a dry run of `dry` rehearsed what `real` sends: the same command, where a
/// `resource::Apply` may differ in what it was reviewed against (`resource_version`,
/// `reviewed_absent`). Those only make the replace fallback stricter; the server-side apply that
/// was dry-run is the same request.
fn same_request(dry: &Command, real: &Command) -> bool {
    match (dry, real) {
        (
            Command::ResourceApply {
                cluster,
                namespace,
                manifest,
                field_manager,
                force,
                ..
            },
            Command::ResourceApply {
                cluster: real_cluster,
                namespace: real_namespace,
                manifest: real_manifest,
                field_manager: real_field_manager,
                force: real_force,
                ..
            },
        ) => {
            cluster == real_cluster
                && namespace == real_namespace
                && field_manager == real_field_manager
                && force == real_force
                && manifest == real_manifest
        }
        _ => dry == real,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ctx, id};

    fn apply(manifest: &str, resource_version: Option<&str>) -> Command {
        Command::ResourceApply {
            cluster: id("a"),
            namespace: None,
            manifest: manifest.to_owned(),
            field_manager: None,
            force: false,
            resource_version: resource_version.map(str::to_owned),
            reviewed_absent: false,
        }
    }

    #[test]
    fn a_receipt_is_used_once_by_the_same_person_for_the_same_request() {
        let receipts = Receipts::default();
        let dry = ctx(Initiator::Ui).with_dry_run(true);
        let kept = Receipts::eligible(&apply("x", None), &dry).expect("a person's apply dry run");
        receipts.record(dry.who, id("a"), kept);

        let real = ctx(Initiator::Ui);
        assert!(
            !receipts.take(&apply("y", Some("7")), &real, &id("a")),
            "another manifest"
        );
        assert!(
            !receipts.take(&apply("x", Some("7")), &real, &id("b")),
            "another cluster"
        );
        assert!(
            !receipts.take(
                &apply("x", Some("7")),
                &real.clone().with_dry_run(true),
                &id("a")
            ),
            "a dry run takes none"
        );
        assert!(
            receipts.take(&apply("x", Some("7")), &real, &id("a")),
            "the reviewed apply"
        );
        assert!(
            !receipts.take(&apply("x", Some("7")), &real, &id("a")),
            "used once"
        );
    }

    #[test]
    fn agents_and_other_commands_never_get_receipts() {
        assert!(
            Receipts::eligible(&apply("x", None), &ctx(Initiator::Agent).with_dry_run(true))
                .is_none()
        );
        assert!(Receipts::eligible(&apply("x", None), &ctx(Initiator::Ui)).is_none());
        let delete = crate::testing::pod_delete("a", "web");
        assert!(Receipts::eligible(&delete, &ctx(Initiator::Ui).with_dry_run(true)).is_none());

        let receipts = Receipts::default();
        receipts.record(Arc::from("someone"), id("a"), apply("x", None));
        assert!(!receipts.take(&apply("x", None), &ctx(Initiator::Agent), &id("a")));
    }
}
