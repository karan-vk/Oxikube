//! [`GatewaySummary`]: a Gateway with its listeners and addresses.

use std::sync::Arc;

use super::super::super::{
    Condition, ConditionStatus, ViewError, arc_of, arr_of, check_kind, i64_of, str_of, sub,
};
use super::{GATEWAY_API_GROUP, ObjectRef, conditions_of, port_of, status_of, string_of};
use crate::json::JsonRef;
use crate::resource::Resource;

/// One address of a Gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayAddress {
    /// `IPAddress`, `Hostname` or an implementation-specific type; `IPAddress` when unset.
    pub kind: String,
    /// The address.
    pub value: String,
}

/// One listener of a Gateway, joined with its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerSummary {
    /// `name`.
    pub name: String,
    /// `HTTP`, `HTTPS`, `TLS`, `TCP`, `UDP` or an implementation-specific protocol.
    pub protocol: String,
    /// `port`.
    pub port: Option<u16>,
    /// `hostname`; `None` matches every host.
    pub hostname: Option<String>,
    /// `tls.mode`: `Terminate` (the default) or `Passthrough`; `None` for a listener without TLS.
    pub tls_mode: Option<String>,
    /// The Secrets named by `tls.certificateRefs`.
    pub certificates: Vec<String>,
    /// Where routes may attach from: `Same namespace` (the default), `All namespaces`, or
    /// `Selected namespaces`.
    pub allowed_namespaces: String,
    /// The route kinds the listener accepts, when it restricts them.
    pub allowed_kinds: Vec<String>,
    /// `status.listeners[].attachedRoutes`, when the controller reported it.
    pub attached_routes: Option<i64>,
    /// The conditions the controller reported for the listener.
    pub conditions: Vec<Condition>,
}

impl ListenerSummary {
    /// `HTTPS :443 api.example.com`.
    pub fn title(&self) -> String {
        let mut text = self.protocol.clone();
        if let Some(port) = self.port {
            text.push_str(&format!(" :{port}"));
        }
        if let Some(host) = &self.hostname {
            text.push(' ');
            text.push_str(host);
        }
        text
    }

    fn from_json(listener: JsonRef<'_>, status: Option<JsonRef<'_>>) -> Option<Self> {
        let name = str_of(listener, "name")?;
        let tls = sub(listener, "tls");
        let allowed = sub(listener, "allowedRoutes");
        let namespaces = sub(allowed, "namespaces");
        Some(Self {
            name: name.to_owned(),
            protocol: str_of(listener, "protocol").unwrap_or_default().to_owned(),
            port: port_of(listener, "port"),
            hostname: string_of(listener, "hostname"),
            tls_mode: tls
                .is_object()
                .then(|| str_of(tls, "mode").unwrap_or("Terminate").to_owned()),
            certificates: arr_of(tls, "certificateRefs")
                .iter()
                .filter_map(|c| str_of(c, "name"))
                .map(str::to_owned)
                .collect(),
            allowed_namespaces: match str_of(namespaces, "from") {
                Some("All") => "All namespaces",
                Some("Selector") => "Selected namespaces",
                _ => "Same namespace",
            }
            .to_owned(),
            allowed_kinds: arr_of(allowed, "kinds")
                .iter()
                .filter_map(|k| str_of(k, "kind"))
                .map(str::to_owned)
                .collect(),
            attached_routes: status.and_then(|s| i64_of(s, "attachedRoutes")),
            conditions: status.map(conditions_of).unwrap_or_default(),
        })
    }
}

/// A Gateway: its class, listeners and addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewaySummary {
    /// `spec.gatewayClassName`.
    pub class: Option<Arc<str>>,
    /// `spec.listeners`, each with its status entry.
    pub listeners: Vec<ListenerSummary>,
    /// `status.addresses`: where the Gateway is reachable. When the controller reported none,
    /// the addresses requested in `spec.addresses` (with [`addresses_requested`](Self::addresses_requested) set).
    pub addresses: Vec<GatewayAddress>,
    /// Whether `addresses` are the ones asked for in the spec, not assigned ones.
    pub addresses_requested: bool,
    /// `status.conditions` (`Accepted`, `Programmed`), with their reasons.
    pub conditions: Vec<Condition>,
}

impl GatewaySummary {
    /// The summary of a Gateway API Gateway.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] for any other kind.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "Gateway", &[(GATEWAY_API_GROUP, "Gateway")])?;
        let spec = sub(res.json(), "spec");
        let status = sub(res.json(), "status");
        let listener_status = arr_of(status, "listeners");
        let listeners = arr_of(spec, "listeners")
            .iter()
            .filter_map(|listener| {
                let name = str_of(listener, "name");
                let entry = listener_status
                    .iter()
                    .find(|s| name.is_some() && str_of(*s, "name") == name);
                ListenerSummary::from_json(listener, entry)
            })
            .collect();
        let address = |a: JsonRef<'_>| {
            Some(GatewayAddress {
                kind: str_of(a, "type").unwrap_or("IPAddress").to_owned(),
                value: str_of(a, "value")?.to_owned(),
            })
        };
        let mut addresses: Vec<GatewayAddress> = arr_of(status, "addresses")
            .iter()
            .filter_map(address)
            .collect();
        let addresses_requested = addresses.is_empty();
        if addresses_requested {
            addresses = arr_of(spec, "addresses")
                .iter()
                .filter_map(address)
                .collect();
        }
        Ok(Self {
            class: arc_of(spec, "gatewayClassName"),
            listeners,
            addresses_requested: addresses_requested && !addresses.is_empty(),
            addresses,
            conditions: conditions_of(status),
        })
    }

    /// The `Accepted` condition's status.
    pub fn accepted(&self) -> ConditionStatus {
        status_of(&self.conditions, "Accepted")
    }

    /// The `Programmed` condition's status: whether the data plane is configured.
    pub fn programmed(&self) -> ConditionStatus {
        status_of(&self.conditions, "Programmed")
    }

    /// The reference to the Gateway's class, for the link to it.
    pub fn class_ref(&self) -> Option<ObjectRef> {
        self.class
            .as_deref()
            .map(|name| ObjectRef::new(GATEWAY_API_GROUP, "GatewayClass", None, name))
    }

    /// The addresses as one line, comma-separated.
    pub fn addresses_text(&self) -> String {
        self.addresses
            .iter()
            .map(|a| a.value.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The listeners as one line: `HTTP :80, HTTPS :443 api.example.com`.
    pub fn listeners_text(&self) -> String {
        self.listeners
            .iter()
            .map(ListenerSummary::title)
            .collect::<Vec<_>>()
            .join(", ")
    }
}
