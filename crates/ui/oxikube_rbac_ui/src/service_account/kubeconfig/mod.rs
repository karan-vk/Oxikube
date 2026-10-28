//! "Generate kubeconfig" on the ServiceAccount panel (E17-S09).
//!
//! | File | Holds |
//! |---|---|
//! | `state` | [`Expiry`] (the lifetime choices), [`KubeconfigDeps`] (the filesystem Save writes through and the vault), [`KubeconfigState`] |
//! | `actions` | the buttons: generate (`serviceaccount::GenerateKubeconfig` through the bus), copy, save, reveal, discard, and the look at the vault |
//! | `view` | the drawing: lifetime choices, the button, the masked preview |
//!
//! # How a user gets here
//!
//! Open **Access Control / Service Accounts** in the cluster sidebar (or `:sa`), open a row, and
//! use the **Kubeconfig** section of its panel: pick how long the token lives, press **Generate
//! kubeconfig**, confirm. The same command is "Generate ServiceAccount Kubeconfig" in the
//! palette and the row menu; the result always lands in this section.
//!
//! The token is held in memory only. The preview masks it until **Show token**; **Copy** puts the
//! file on the clipboard; **Save…** asks where and writes it owner-only (mode 0600). Closing the
//! panel or pressing **Discard** drops it.

mod actions;
mod state;
mod view;

pub use state::{Expiry, KubeconfigDeps, KubeconfigState};

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};

use crate::service_account::gvk;

/// Where "Generate Kubeconfig" sits in a row's menu: after "Used By" (140).
pub const GENERATE_ORDER: u16 = 150;

/// The row action "Generate Kubeconfig" for ServiceAccounts: `serviceaccount::GenerateKubeconfig`
/// with the default lifetime (one hour) and no long-lived fallback. The guard confirms; the
/// result waits in the account's panel, which the toast says.
pub fn row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::SERVICEACCOUNT_GENERATE_KUBECONFIG, |target| {
            Command::ServiceAccountGenerateKubeconfig {
                target: target.clone(),
                expiration_seconds: Some(Expiry::OneHour.seconds()),
                audiences: Vec::new(),
                allow_legacy_token: false,
            }
        })
        .label("Generate Kubeconfig")
        .kinds(KindFilter::Matching(|kind| kind.gvk == gvk()))
        .order(GENERATE_ORDER),
    ]
}
