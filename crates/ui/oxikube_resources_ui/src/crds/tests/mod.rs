//! Tests of the CRD modules: reading a CRD, its versions, the schema tree and the row actions,
//! and (E12-S17) the parity of a custom resource table with the server's Table response.

pub(crate) mod fixture;
mod info;
mod parity;
mod versions;
mod views;
