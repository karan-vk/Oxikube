//! What start-up does *not* do: the services started on first use (E05-S13).
//!
//! The rule every epic follows: an `init(cx)` in the [init order](super) only registers (actions,
//! settings, item builders, globals). Anything that loads an engine, reads more than a small
//! config file, spawns a process or touches the network is an
//! [`oxikube_runtime::LazyService`] whose `ensure_init` its first real use calls, and its heavy
//! part then runs on `spawn_kube` or the background executor. [`DEFERRED`] lists the services
//! planned so far, which story makes each lazy, and the first use that starts it; the start-up
//! tests check that the first frame is drawn with none of them started
//! ([`oxikube_runtime::LazyServices::started`] is empty).

/// A service start-up leaves for later.
#[derive(Debug, Clone, Copy)]
pub struct Deferred {
    /// The service ([`oxikube_runtime::LazyService::name`] once it exists).
    pub name: &'static str,
    /// The epic that builds it.
    pub owner: &'static str,
    /// The first use that calls its `ensure_init`.
    pub started_by: &'static str,
}

/// The deferred services, in the order they are likely to be needed.
pub const DEFERRED: &[Deferred] = &[
    Deferred {
        name: "kubeconfig_sources",
        owner: "E03 / E06 / E07-S00",
        started_by: "the catalog home's first read, after the first frame: \
                     `kube_ports::LazyKubeSources` builds the adapter (environment, files, \
                     watcher) on `spawn_kube`, never on the UI thread",
    },
    Deferred {
        name: "api_discovery",
        owner: "E03",
        started_by: "connecting a cluster (per cluster, on `spawn_kube`)",
    },
    Deferred {
        name: "cloud_discovery",
        owner: "E18",
        started_by: "opening the cloud sources page or a refresh the user asked for",
    },
    Deferred {
        name: "prometheus_detection",
        owner: "E13",
        started_by: "the first metrics view of a connected cluster",
    },
    Deferred {
        name: "extension_host",
        owner: "E23",
        started_by: "the first extension command, view or the extensions page (wasmtime engine \
                     and compile cache on the background executor)",
    },
    Deferred {
        name: "agent_registry",
        owner: "E26 / E27",
        started_by: "opening the agent panel or the first MCP client connecting",
    },
    Deferred {
        name: "update_checker",
        owner: "E24",
        started_by: "a timer started after the first frame, or the user's \"check for updates\"",
    },
];
