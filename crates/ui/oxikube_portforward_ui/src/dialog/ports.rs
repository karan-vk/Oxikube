//! [`PortChoice`]: a port of the target the dialog offers, read from the loaded object.

use oxikube_domain::Resource;
use oxikube_domain::portforward::ForwardPort;
use serde_json::Value;

/// One forwardable (TCP) port of a Pod or a Service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortChoice {
    /// The port number (a pod's `containerPort`, a Service's `port`).
    pub number: u16,
    /// The name it was given (`http`).
    pub name: Option<String>,
    /// The container it belongs to (pods only).
    pub container: Option<String>,
}

impl PortChoice {
    /// `http 8080` or `8080`, plus the container for a pod: the chip's text.
    pub fn label(&self) -> String {
        let base = match &self.name {
            Some(name) => format!("{name} {}", self.number),
            None => self.number.to_string(),
        };
        match &self.container {
            Some(container) => format!("{base} ({container})"),
            None => base,
        }
    }

    /// The remote port this choice forwards.
    pub fn forward_port(&self) -> ForwardPort {
        ForwardPort::Number(self.number)
    }
}

/// The TCP ports of a Pod (every container) or a Service, in the object's order. Anything else
/// has none.
pub fn ports_of(resource: &Resource) -> Vec<PortChoice> {
    let json = resource.json().to_value();
    let spec = json.get("spec");
    match &*resource.kind.kind {
        "Pod" => spec
            .and_then(|s| s.get("containers"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|container| {
                let owner = container.get("name").and_then(Value::as_str);
                container
                    .get("ports")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(move |p| choice(p, "containerPort", owner))
            })
            .collect(),
        "Service" => spec
            .and_then(|s| s.get("ports"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| choice(p, "port", None))
            .collect(),
        _ => Vec::new(),
    }
}

fn choice(port: &Value, number_key: &str, container: Option<&str>) -> Option<PortChoice> {
    let protocol = port
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or("TCP");
    if !protocol.eq_ignore_ascii_case("TCP") {
        return None;
    }
    let number = port.get(number_key)?.as_u64()?;
    Some(PortChoice {
        number: u16::try_from(number).ok().filter(|n| *n != 0)?,
        name: port.get("name").and_then(Value::as_str).map(str::to_owned),
        container: container.map(str::to_owned),
    })
}
