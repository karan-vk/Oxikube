//! Secret values: decoded in memory, edited as plaintext, written back as base64 (E12-S11).
//!
//! A Secret's `data` is base64 on the wire, which nobody should read or type by hand. This module
//! is the pure half of working with the plaintext:
//!
//! | File | Holds |
//! |---|---|
//! | `text` | [`SecretText`]: a value that prints as `***` in `Debug` and `Display` and is zeroised on drop |
//! | `kind` | [`SecretType`]: what `type` says the keys mean (Opaque, TLS, registry, ...) |
//! | `decoded` | [`DecodedSecret`]: `data` read as text; binary values kept as their original base64 |
//! | `edit` | [`to_decoded`] / [`from_decoded`]: the object the editor shows for "edit decoded", and the object that is applied |
//! | `docker` | [`docker::registries`] reads a registry login without its password; [`docker::config_json`] writes one |
//!
//! Decoded values live in memory only: the types here have no `Serialize`, their `Debug` shows
//! keys and `***`, and nothing writes them anywhere. The service that decides when a value may be
//! decoded and shown is `oxikube_app::SecretService`.

pub mod docker;

mod decoded;
mod edit;
mod kind;
mod line;
mod text;

pub use decoded::{
    BINARY_PREFIX, DecodedEntry, DecodedSecret, DecodedValue, MASKED_VALUE, SecretError,
    is_displayable, is_placeholder, is_secret, placeholder,
};
pub use edit::{LAST_APPLIED, from_decoded, to_decoded};
pub use kind::SecretType;
pub use text::{STARS, SecretText};
