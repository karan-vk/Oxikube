//! Tests of the RBAC model and normaliser. The comparison with recorded
//! `kubectl auth can-i --list` output lives with the subject index in `oxikube_app`.

mod escalation;
mod matrix;
mod model;
mod normalise;
mod props;
