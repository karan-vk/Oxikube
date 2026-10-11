//! IngressClass actions (E12-S14): the one mutation of the section, making an IngressClass
//! the cluster's default.
//!
//! | File | Holds |
//! |---|---|
//! | `handler` | [`register_commands`]: the `ingressclass::SetDefault` handler, the only code that writes the default-class annotation, through the guard's `Mutation` |
//!
//! `ingressclass::SetDefault` is a low-risk mutation (a simple confirmation): blocked on a
//! read-only cluster for every initiator, a server dry run on request, one audit record. It merge
//! patches `ingressclass.kubernetes.io/is-default-class: "true"` onto the class and, unless the
//! command says otherwise, removes the annotation from the class that carried it (the API server
//! refuses an Ingress without a class while two defaults exist). Everything else in the Network
//! section is read-only.

mod handler;

#[cfg(test)]
mod tests;

pub use handler::register_commands;
