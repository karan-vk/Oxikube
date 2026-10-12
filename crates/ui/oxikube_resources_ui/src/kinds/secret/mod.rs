//! Secrets (E12-S11): masked by default, revealed on purpose, decoded as text, type-aware.
//!
//! | File | Holds |
//! |---|---|
//! | `panel` | [`SecretPanel`]: the kind's panel above the Overview. A header (the type, the key count, "Reveal" and "Edit decoded"), the type's facts, and the keys as a virtualised list of uniform rows: name, size, and the value as dots until it is revealed, as its first line after; a binary value is its size either way |
//! | `facts` | what a type puts on display without anyone revealing a value: the registries and usernames of a registry login (never the password or `auth`), a TLS certificate's subject, names and expiry (never the key), a service-account token's account |
//! | `rows` | the stateless drawing of one key |
//! | `actions` | [`secret_row_actions`]: "Reveal values" (`secret::Reveal`, `x`) and "Edit decoded" (`secret::EditDecoded`, `shift-e`) in a Secret's row menu and the palette's list |
//!
//! A Secret's values never reach this panel by default: the detail's full read removes them and
//! keeps a [`SecretSummary`](oxikube_app::SecretSummary) (names, sizes, the type's public facts).
//! Revealing is `secret::Reveal`, which reads the Secret once more and holds it decoded in the
//! detail view's memory ([`DetailView::revealed_value`](crate::detail::DetailView::revealed_value)) until
//! it is hidden, the tab closes or [`REVEAL_TIMEOUT`](crate::detail::REVEAL_TIMEOUT) passes. The
//! panel reads it from there; it keeps no value of its own.
//!
//! Creating a Secret is `secret::Create` (the table's "New" button on this kind, the palette): the
//! editor's "Create secret" dialog, in `oxikube_editor`.

mod actions;
mod facts;
mod panel;
mod rows;

use gpui::{App, AppContext as _};
use oxikube_app::{KindMeta, Section};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, Scope};

pub use actions::{EDIT_DECODED_ORDER, REVEAL_ORDER, is_secret_kind, secret_row_actions};
pub use panel::SecretPanel;

use super::detail::DetailRenderer;
use super::spec::KindSpec;
use super::specs::KindSpecs;

/// Where Secrets sit in the Config section: after ConfigMaps.
pub(super) const RANK: u32 = 20;

/// The Secrets kind: the generic table, this panel in the detail, and its own actions.
pub fn spec() -> KindSpec {
    let meta = KindMeta::new(
        Gvk::new("", "v1", "Secret"),
        "secrets",
        "Secrets",
        Scope::Namespaced,
    )
    .in_section(Section::CONFIG, RANK)
    .with_actions([
        CommandId::SECRET_REVEAL,
        CommandId::SECRET_EDIT_DECODED,
        CommandId::SECRET_CREATE,
        CommandId::RESOURCE_SHOW_USED_BY,
    ]);
    KindSpec::new(meta).with_detail(DetailRenderer::panel(|detail, _, cx| {
        cx.new(|cx| SecretPanel::new(detail, cx)).into()
    }))
}

/// Registers the Secrets kind.
pub fn init(cx: &mut App) {
    if let Err(error) = KindSpecs::register(cx, spec()) {
        tracing::error!(%error, "the Secret kind was not registered");
    }
}
