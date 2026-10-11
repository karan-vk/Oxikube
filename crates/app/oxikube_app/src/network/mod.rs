//! Services and where their traffic goes (E12-S13): the questions a Service's detail asks of the
//! store, the navigation from a Service to its pods, and the hook behind "Open in browser".
//!
//! | File | Holds |
//! |---|---|
//! | `backends` | [`ServiceBackends`]: the two store queries of a Service's detail, its EndpointSlices (by `kubernetes.io/service-name`) and its pods (by the Service's selector), both with the label selector sent to the server so a large namespace is never scanned locally |
//! | `navigate` | [`plan_show_pods`]: `service::ShowPods` as the commands of a jump (`namespace::Select` when needed, `resource::OpenList`, `table::SetFilter -l <selector>`) |
//! | `browser` | [`ServicePortOpener`]: the hook the port-forward manager (E15) implements, and the handler of `service::OpenInBrowser` that is registered only when there is one |
//!
//! Nothing here starts a feed by itself: the detail of one Service subscribes to its two queries
//! when it is open and drops them when it closes, so a table of 500 Services costs nothing extra.

mod backends;
mod browser;
mod navigate;

#[cfg(test)]
mod tests;

pub use backends::{PodsPlan, ServiceBackends};
pub use browser::{OpenedServicePort, ServicePortOpener, UrlSink, register_open_in_browser};
pub use navigate::plan_show_pods;
