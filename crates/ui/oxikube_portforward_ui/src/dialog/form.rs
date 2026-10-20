//! [`StartForm`]: the dialog's fields and what makes them valid. Pure: no gpui, no I/O.

use std::net::IpAddr;
use std::ops::RangeInclusive;

use oxikube_domain::ids::ResourceRef;
use oxikube_domain::portforward::{ForwardPort, ForwardSpec};

use super::ports::PortChoice;

/// First port that needs no privileges on Linux and macOS.
pub const FIRST_UNPRIVILEGED_PORT: u16 = 1024;

/// The fields as the user typed them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormInput {
    /// The remote port: a number, or a name from the target's ports.
    pub remote_port: String,
    /// Whether the local port is chosen automatically.
    pub local_auto: bool,
    /// The local port when it is fixed.
    pub local_port: String,
    /// The local address to bind.
    pub bind: String,
    /// The forward's name; empty is the default.
    pub name: String,
    /// Whether to keep it as a favourite.
    pub favourite: bool,
}

impl Default for FormInput {
    fn default() -> Self {
        Self {
            remote_port: String::new(),
            local_auto: true,
            local_port: String::new(),
            bind: ForwardSpec::DEFAULT_BIND.to_string(),
            name: String::new(),
            favourite: false,
        }
    }
}

/// What is wrong with each field, shown under it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormErrors {
    /// The remote port.
    pub remote_port: Option<String>,
    /// The local port.
    pub local_port: Option<String>,
    /// The bind address.
    pub bind: Option<String>,
}

impl FormErrors {
    /// Whether every field is fine.
    pub fn is_empty(&self) -> bool {
        self.remote_port.is_none() && self.local_port.is_none() && self.bind.is_none()
    }
}

/// The target, its ports and the rules the fields follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartForm {
    /// What is forwarded.
    pub target: ResourceRef,
    /// The target's ports, when known.
    pub ports: Vec<PortChoice>,
    /// The local ports the settings allow (`portforward.port_range`; all of them until E15-S08).
    pub range: RangeInclusive<u16>,
}

impl StartForm {
    /// A form for `target` with `ports` to choose from.
    pub fn new(target: ResourceRef, ports: Vec<PortChoice>) -> Self {
        Self {
            target,
            ports,
            range: 1..=u16::MAX,
        }
    }

    /// Limits the local port to `range`.
    #[must_use]
    pub fn with_range(mut self, range: RangeInclusive<u16>) -> Self {
        self.range = range;
        self
    }

    /// `pod/web:8080`: the name a forward gets when the field is empty.
    pub fn default_name(&self, remote_port: &str) -> String {
        format!(
            "{}/{}:{}",
            self.target.gvk.kind.to_ascii_lowercase(),
            self.target.name,
            remote_port.trim()
        )
    }

    /// The remote port as typed: a number, or a port name the target has.
    fn remote(&self, text: &str) -> Result<ForwardPort, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Choose the port to forward".to_owned());
        }
        if let Ok(number) = text.parse::<u16>() {
            return if number == 0 {
                Err("Ports run from 1 to 65535".to_owned())
            } else {
                Ok(ForwardPort::Number(number))
            };
        }
        if text.chars().all(|c| c.is_ascii_digit()) {
            return Err("Ports run from 1 to 65535".to_owned());
        }
        match self.ports.iter().find(|p| p.name.as_deref() == Some(text)) {
            Some(port) => Ok(port.forward_port()),
            None => Err(format!("{} has no port named {text}", self.target.name)),
        }
    }

    fn local(&self, input: &FormInput) -> Result<u16, String> {
        if input.local_auto {
            return Ok(0);
        }
        let text = input.local_port.trim();
        let number: u32 = text
            .parse()
            .map_err(|_| "Enter a port from 1 to 65535, or choose Auto".to_owned())?;
        let number = u16::try_from(number)
            .ok()
            .filter(|n| *n != 0)
            .ok_or_else(|| "Ports run from 1 to 65535".to_owned())?;
        if self.range.contains(&number) {
            Ok(number)
        } else {
            Err(format!(
                "Local ports are limited to {} to {}",
                self.range.start(),
                self.range.end()
            ))
        }
    }

    /// What is wrong with `input`, field by field. An empty remote port is not an error yet (the
    /// user has not typed): [`build`](Self::build) reports it.
    pub fn errors(&self, input: &FormInput) -> FormErrors {
        FormErrors {
            remote_port: (!input.remote_port.trim().is_empty())
                .then(|| self.remote(&input.remote_port).err())
                .flatten(),
            local_port: (!input.local_auto && !input.local_port.trim().is_empty())
                .then(|| self.local(input).err())
                .flatten(),
            bind: input
                .bind
                .trim()
                .parse::<IpAddr>()
                .err()
                .map(|_| "Enter an IP address such as 127.0.0.1".to_owned()),
        }
    }

    /// A non-blocking warning about the bind address: a non-loopback address exposes the cluster
    /// endpoint to the network.
    pub fn bind_warning(&self, input: &FormInput) -> Option<String> {
        let ip = input.bind.trim().parse::<IpAddr>().ok()?;
        (!ip.is_loopback()).then(|| {
            format!("{ip} is reachable from other machines: anyone who can connect gets this cluster endpoint")
        })
    }

    /// A non-blocking hint about the local port: a privileged port usually cannot be bound.
    pub fn local_warning(&self, input: &FormInput) -> Option<String> {
        if input.local_auto {
            return None;
        }
        let number = self.local(input).ok()?;
        (number < FIRST_UNPRIVILEGED_PORT)
            .then(|| format!("Ports below {FIRST_UNPRIVILEGED_PORT} need privileges: use {FIRST_UNPRIVILEGED_PORT} or higher"))
    }

    /// Whether Forward can be pressed.
    pub fn can_submit(&self, input: &FormInput) -> bool {
        self.build(input).is_ok()
    }

    /// The forward to start.
    ///
    /// # Errors
    ///
    /// The fields that are wrong, with a message each.
    pub fn build(&self, input: &FormInput) -> Result<ForwardSpec, FormErrors> {
        let remote = self.remote(&input.remote_port);
        let local = self.local(input);
        let bind = input.bind.trim().parse::<IpAddr>();
        let (Ok(remote), Ok(local), Ok(bind)) = (remote.clone(), local.clone(), bind) else {
            return Err(FormErrors {
                remote_port: remote.err(),
                local_port: local.err(),
                bind: input
                    .bind
                    .trim()
                    .parse::<IpAddr>()
                    .err()
                    .map(|_| "Enter an IP address such as 127.0.0.1".to_owned()),
            });
        };
        let mut spec = ForwardSpec::new(self.target.clone(), remote)
            .with_local_port(local)
            .with_bind(bind)
            .with_favourite(input.favourite);
        let name = input.name.trim();
        if !name.is_empty() {
            spec = spec.with_name(name);
        }
        Ok(spec)
    }
}
