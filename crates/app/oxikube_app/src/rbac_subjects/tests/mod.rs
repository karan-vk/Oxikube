//! Tests of the subject views' model over the recorded RBAC fixtures
//! (`oxikube_testkit::fixtures::rbac`, a v1.37 kind cluster's Roles, bindings and the rules
//! reviews it answered).

mod feed;
mod filter;
mod matrix;
mod summary;

use oxikube_domain::rbac::{GrantScope, Subject};
use oxikube_testkit::fixtures::rbac;

use super::{MatrixLine, PolicyHandle};

pub(super) const NS: &str = "oxi-rbac-fx";

pub(super) fn handle() -> PolicyHandle {
    PolicyHandle::fixed(&rbac::cluster())
}

pub(super) fn namespace(name: &str) -> GrantScope {
    GrantScope::Namespace(name.into())
}

pub(super) fn user(name: &str) -> Subject {
    Subject::User(name.into())
}

pub(super) fn group(name: &str) -> Subject {
    Subject::Group(name.into())
}

/// The line of `group`/`resource`, panicking with the lines there are.
pub(super) fn line<'a>(lines: &'a [MatrixLine], group: &str, resource: &str) -> &'a MatrixLine {
    lines
        .iter()
        .find(|l| l.api_group == group && l.resource == resource)
        .unwrap_or_else(|| {
            let have: Vec<String> = lines
                .iter()
                .map(|l| format!("{}/{}", l.api_group, l.resource))
                .collect();
            panic!("no line {group:?}/{resource:?} in {have:?}")
        })
}
