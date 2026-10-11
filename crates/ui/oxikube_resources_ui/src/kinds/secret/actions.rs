//! The row actions of a Secret: "Reveal values" and "Edit decoded".
//!
//! Both only read the object (a reveal reads it into the detail's memory, the editor reads it to
//! show it), so neither is a mutation and both stay available on a read-only cluster; applying
//! what the editor holds is `resource::Apply`, which is guarded.

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::Gvk;

/// Where "Reveal values" sits in a Secret's menu: after "View YAML" (10) and the editor's entries.
pub const REVEAL_ORDER: u16 = 32;
/// Where "Edit decoded" sits: right after "Reveal values".
pub const EDIT_DECODED_ORDER: u16 = 34;

/// Whether `gvk` is the core `Secret`.
pub fn is_secret_kind(gvk: &Gvk) -> bool {
    gvk.group.is_empty() && &*gvk.kind == "Secret"
}

/// The row actions of the Secrets kind, one object at a time.
pub fn secret_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::SECRET_REVEAL, |target| Command::SecretReveal {
            target: target.clone(),
            key: None,
        })
        .label("Reveal Values")
        .kinds(KindFilter::Matching(|kind| is_secret_kind(&kind.gvk)))
        .order(REVEAL_ORDER),
        RowActionSpec::new(CommandId::SECRET_EDIT_DECODED, |target| {
            Command::SecretEditDecoded {
                target: target.clone(),
            }
        })
        .label("Edit Decoded")
        .kinds(KindFilter::Matching(|kind| is_secret_kind(&kind.gvk)))
        .order(EDIT_DECODED_ORDER),
    ]
}
