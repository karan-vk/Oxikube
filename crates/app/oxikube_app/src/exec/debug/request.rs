//! What a debug container is asked for ([`DebugRequest`]), and what the pod makes of it
//! ([`plan_debug`] -> [`DebugPlan`]): pure functions over the pod object, no I/O.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::command::{Command, DEFAULT_DEBUG_COMMAND, DEFAULT_DEBUG_IMAGE};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::{ContainerKind, ContainerSummary};
use oxikube_domain::{OxiError, OxiResult, Resource};

use crate::exec::containers::PodContainers;

/// How long to wait for a debug container to start when nobody chose: long enough for an image
/// pull, short enough that a rejected or stuck one is reported.
pub const DEFAULT_DEBUG_START_TIMEOUT: Duration = Duration::from_secs(60);

/// The longest a container name may be (a DNS label).
const MAX_NAME_LEN: usize = 63;

/// A debug container as the user (or the `pod::Debug` command) asks for it. Blank fields mean "the
/// default": see [`plan_debug`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugRequest {
    /// The pod to debug (namespaced).
    pub pod: ResourceRef,
    /// The image to run; blank is [`DEFAULT_DEBUG_IMAGE`] for a request built from a command, an
    /// error for [`plan_debug`] (a dialog never sends one).
    pub image: String,
    /// The container whose process namespace to share; `None` is the pod's default container.
    pub target_container: Option<String>,
    /// The program and arguments; empty runs [`DEFAULT_DEBUG_COMMAND`].
    pub command: Vec<String>,
    /// The new container's name; `None` generates `debugger-xxxxx`.
    pub name: Option<String>,
    /// How long to wait for it to run.
    pub start_timeout: Duration,
}

impl DebugRequest {
    /// A request for `image` in `pod` with every default.
    pub fn new(pod: ResourceRef, image: impl Into<String>) -> Self {
        Self {
            pod,
            image: image.into(),
            target_container: None,
            command: Vec::new(),
            name: None,
            start_timeout: DEFAULT_DEBUG_START_TIMEOUT,
        }
    }

    /// The request a `pod::Debug` command makes (blank image, container name and program mean the
    /// defaults).
    ///
    /// # Errors
    ///
    /// `Validation` for a command that is not `pod::Debug`.
    pub fn from_command(command: &Command) -> OxiResult<Self> {
        let Command::PodDebug {
            target,
            image,
            target_container,
            command,
            name,
        } = command
        else {
            return Err(OxiError::validation("not a pod::Debug command"));
        };
        let blank = |text: &Option<String>| text.clone().filter(|t| !t.trim().is_empty());
        let mut request = Self::new(
            target.clone(),
            if image.trim().is_empty() {
                DEFAULT_DEBUG_IMAGE
            } else {
                image
            },
        );
        request.target_container = blank(target_container);
        request.name = blank(name);
        request.command.clone_from(command);
        Ok(request)
    }

    /// The `pod::Debug` command that makes this request.
    pub fn to_command(&self) -> Command {
        Command::PodDebug {
            target: self.pod.clone(),
            image: self.image.trim().to_owned(),
            target_container: self.target_container.clone(),
            command: self.command.clone(),
            name: self.name.clone(),
        }
    }

    /// The checks that need no cluster: the pod is namespaced, the image is one word, the program
    /// is not blank, the name (when given) is a DNS label. A dialog runs this on every submit,
    /// before anything is sent.
    ///
    /// # Errors
    ///
    /// `Validation` with a message that says which field.
    pub fn check_fields(&self) -> OxiResult<()> {
        if self.pod.namespace().is_none() {
            return Err(OxiError::validation(
                "a debug container needs a namespaced pod",
            ));
        }
        let image = self.image.trim();
        if image.is_empty() {
            return Err(OxiError::validation("the image is empty"));
        }
        if image.chars().any(char::is_whitespace) {
            return Err(OxiError::validation(
                "the image must be one word, like busybox",
            ));
        }
        if self
            .command
            .first()
            .is_some_and(|program| program.trim().is_empty())
        {
            return Err(OxiError::validation(
                "the command starts with a blank program",
            ));
        }
        if let Some(name) = &self.name {
            check_name(name)?;
        }
        Ok(())
    }
}

/// What the pod makes of a [`DebugRequest`]: every default filled in and every check passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugPlan {
    /// The new container's name, unused in the pod.
    pub name: String,
    /// The image to run.
    pub image: String,
    /// The container whose processes the debug container shares.
    pub target: Arc<str>,
    /// The program and arguments (never empty).
    pub command: Vec<String>,
}

impl DebugPlan {
    /// What the terminal's first line says: the container, its image and what it targets.
    pub(crate) fn notice(&self, pod: &ResourceRef) -> String {
        format!(
            "debug container {} ({}) in {}, sharing the processes of {}",
            self.name, self.image, pod.name, self.target
        )
    }
}

/// Plans a debug container for `request` in `pod` (the pod as the cluster returns it).
///
/// Checks the request's fields, that the pod still runs (an ephemeral container cannot be added to
/// a finished pod), that the target container exists (default: the pod's default container) and
/// that the name is free (default: a generated `debugger-xxxxx`; a name cannot be reused in a pod,
/// even after the container exited).
///
/// # Errors
///
/// `Validation` for a bad field, `NotFound` for a target the pod does not have, `Conflict` for a
/// name that is taken or a pod that has finished.
pub fn plan_debug(request: &DebugRequest, pod: &Resource) -> OxiResult<DebugPlan> {
    request.check_fields()?;
    if matches!(
        pod.get_str("/status/phase"),
        Some("Succeeded") | Some("Failed")
    ) {
        return Err(OxiError::conflict(format!(
            "pod {} has finished, so a debug container cannot be added to it",
            request.pod.name
        )));
    }
    let taken: HashSet<Arc<str>> = ContainerSummary::list_from_resource(pod)
        .map_err(|_| OxiError::validation("the object is not a pod"))?
        .into_iter()
        .map(|container| container.name)
        .collect();
    let target = target_of(request, pod)?;
    let name = match &request.name {
        Some(name) => {
            let name = name.trim();
            if taken.contains(name) {
                return Err(OxiError::conflict(format!(
                    "the pod already has a container named {name}; a container name cannot be reused"
                )));
            }
            name.to_owned()
        }
        None => generated_name(&taken),
    };
    let command = if request.command.is_empty() {
        vec![DEFAULT_DEBUG_COMMAND.to_owned()]
    } else {
        request.command.clone()
    };
    Ok(DebugPlan {
        name,
        image: request.image.trim().to_owned(),
        target,
        command,
    })
}

/// The container to share processes with: the one asked for, else the pod's default.
fn target_of(request: &DebugRequest, pod: &Resource) -> OxiResult<Arc<str>> {
    let containers = PodContainers::of(pod);
    // An ephemeral container is no target: it cannot be shared with.
    let candidates = || {
        containers
            .containers()
            .iter()
            .filter(|c| c.kind != ContainerKind::Ephemeral)
    };
    if let Some(name) = request.target_container.as_deref().map(str::trim) {
        return candidates()
            .find(|c| &*c.name == name)
            .map(|c| c.name.clone())
            .ok_or_else(|| {
                OxiError::not_found(format!(
                    "container {name} not found in pod {}",
                    request.pod.name
                ))
            });
    }
    containers
        .default_container()
        .filter(|c| c.kind != ContainerKind::Ephemeral)
        .or_else(|| candidates().next())
        .map(|c| c.name.clone())
        .ok_or_else(|| OxiError::validation("the pod has no container to share processes with"))
}

/// `debugger-` and five random lowercase letters or digits, not in `taken`.
fn generated_name(taken: &HashSet<Arc<str>>) -> String {
    loop {
        let name = format!("debugger-{}", random_suffix());
        if !taken.contains(name.as_str()) {
            return name;
        }
    }
}

/// Five characters of `a-z0-9` from the standard library's randomly seeded hasher.
fn random_suffix() -> String {
    use std::hash::{BuildHasher as _, Hasher as _, RandomState};
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut bits = RandomState::new().build_hasher().finish();
    (0..5)
        .map(|_| {
            let index = (bits % ALPHABET.len() as u64) as usize;
            bits /= ALPHABET.len() as u64;
            char::from(ALPHABET[index])
        })
        .collect()
}

/// Whether `name` is a valid container name (an RFC 1123 label).
///
/// # Errors
///
/// `Validation` saying what is wrong.
pub fn check_name(name: &str) -> OxiResult<()> {
    let name = name.trim();
    let bad = |why: &str| Err(OxiError::validation(format!("the container name {why}")));
    if name.is_empty() {
        return bad("is empty");
    }
    if name.len() > MAX_NAME_LEN {
        return bad("is longer than 63 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return bad("may only hold lowercase letters, digits and dashes");
    }
    if name.starts_with('-') || name.ends_with('-') {
        return bad("must start and end with a letter or a digit");
    }
    Ok(())
}

/// Splits what a user typed in the command field into a program and its arguments: whitespace
/// separates, single or double quotes group (`sh -c "ls -l /"`), a backslash escapes the next
/// character outside single quotes.
///
/// # Errors
///
/// `Validation` for an unclosed quote or a trailing backslash.
pub fn split_command(text: &str) -> OxiResult<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('\''), c) => word.push(c),
            (_, '\\') => match chars.next() {
                Some(next) => {
                    word.push(next);
                    started = true;
                }
                None => return Err(OxiError::validation("the command ends with a backslash")),
            },
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(OxiError::validation("the command has an unclosed quote"));
    }
    if started {
        words.push(word);
    }
    Ok(words)
}
