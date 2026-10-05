//! Unit tests: a scripted in-process API server behind a real `kube::Client`. Each test pins
//! the request the adapter sends (path, method, content type, query, body) and the mapping of
//! the server's answer.

mod evict;
mod harness;
mod patches;
mod pod_patches;
mod scale;
mod subresources;
