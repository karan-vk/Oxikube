//! [`BackendDescriptor`]: what a terminal runs, enough to start it again and nothing more.
//!
//! It is the terminal tab's whole persisted state (layout persistence, reopen-closed): the kind
//! of backend, the program, the directory, and the cluster or pod it belongs to. Never the
//! scrollback, never an environment value, never a credential (non-negotiable 5): a restored
//! terminal is a fresh process started from this, and a cluster shell gets its kubeconfig written
//! again at start.

use std::path::{Path, PathBuf};

use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_ui::IconName;
use serde::{Deserialize, Serialize};

use crate::backend::local::resolve_shell;

/// The version of the saved shape ([`BackendDescriptor::to_state`]). A saved tab of another
/// version is not restored.
const DESCRIPTOR_VERSION: u32 = 1;

/// Longest tab title, in characters; a longer process title is cut with an ellipsis.
const MAX_TITLE_CHARS: usize = 48;

/// What a terminal runs. See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackendDescriptor {
    /// A shell on this machine (`LocalPty`).
    Local {
        /// The cluster whose kubeconfig, context and namespace go into the shell's environment
        /// (written again when the shell starts); `None` for a plain shell.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cluster: Option<ClusterId>,
        /// The namespace the cluster shell starts in (`OXIKUBE_NAMESPACE`); `None` uses the
        /// context's.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        /// The program; `None` follows the `terminal.shell` setting.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<String>,
        /// Its arguments when `shell` is set (`terminal.shell_args` otherwise).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        /// The directory it starts in; `None` for the user's home. A split or a saved tab
        /// carries the shell's directory at that moment
        /// ([`TerminalView::live_descriptor`](super::TerminalView::live_descriptor)).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
        /// The tab's title until the process sets one; `None` names the program. A command run
        /// instead of a shell (`kubectl logs -f ...`) says what it is for here.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// A command run in a container (`kubectl exec -it`).
    Exec {
        /// The pod (its cluster and namespace included).
        pod: ResourceRef,
        /// The container; `None` for the pod's default one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        container: Option<String>,
        /// argv, no shell.
        command: Vec<String>,
    },
    /// The main process of a container, attached to (`kubectl attach -it`).
    Attach {
        /// The pod (its cluster and namespace included).
        pod: ResourceRef,
        /// The container; `None` for the pod's default one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        container: Option<String>,
    },
}

/// The saved form: the descriptor under a version.
#[derive(Serialize, Deserialize)]
struct Saved {
    v: u32,
    backend: BackendDescriptor,
}

impl BackendDescriptor {
    /// A local shell (the `terminal.shell` setting), for `cluster` when given.
    pub fn local(cluster: Option<ClusterId>) -> Self {
        Self::Local {
            cluster,
            namespace: None,
            shell: None,
            args: Vec::new(),
            cwd: None,
            title: None,
        }
    }

    /// Starts a local cluster shell in `namespace` (no effect on other kinds).
    #[must_use]
    pub fn in_namespace(mut self, ns: Option<String>) -> Self {
        if let Self::Local { namespace, .. } = &mut self {
            *namespace = ns;
        }
        self
    }

    /// Starts a local shell in `dir` (no effect on other kinds).
    #[must_use]
    pub fn in_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        if let Self::Local { cwd, .. } = &mut self {
            *cwd = Some(dir.into());
        }
        self
    }

    /// [`in_dir`](Self::in_dir) when `dir` is known; unchanged otherwise.
    #[must_use]
    pub fn in_dir_if_known(self, dir: Option<PathBuf>) -> Self {
        match dir {
            Some(dir) => self.in_dir(dir),
            None => self,
        }
    }

    /// Runs `program` with `program_args` instead of the configured shell (no effect on other
    /// kinds).
    #[must_use]
    pub fn with_shell(mut self, program: impl Into<String>, program_args: Vec<String>) -> Self {
        if let Self::Local { shell, args, .. } = &mut self {
            *shell = Some(program.into());
            *args = program_args;
        }
        self
    }

    /// Names the tab until the process sets its own title (no effect on other kinds).
    #[must_use]
    pub fn titled(mut self, name: impl Into<String>) -> Self {
        if let Self::Local { title, .. } = &mut self {
            *title = Some(name.into());
        }
        self
    }

    /// The cluster the terminal belongs to: the local shell's, or the pod's.
    pub fn cluster(&self) -> Option<&ClusterId> {
        match self {
            Self::Local { cluster, .. } => cluster.as_ref(),
            Self::Exec { pod, .. } | Self::Attach { pod, .. } => Some(&pod.cluster),
        }
    }

    /// The working directory of a local shell.
    pub fn cwd(&self) -> Option<&Path> {
        match self {
            Self::Local { cwd, .. } => cwd.as_deref(),
            Self::Exec { .. } | Self::Attach { .. } => None,
        }
    }

    /// Whether the process runs on this machine (its paths are local files).
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local { .. })
    }

    /// The bus command that opens this pod terminal again (`pod::Shell` for an exec without a
    /// command, `pod::Exec`, `pod::Attach`); `None` for a local shell.
    ///
    /// A pod session is only ever started by its command, so the guard applies the read-only
    /// policy and the audit record: a split, a reconnect or a reopen sends this instead of starting
    /// another process itself.
    pub fn pod_command(&self) -> Option<Command> {
        match self {
            Self::Local { .. } => None,
            Self::Exec {
                pod,
                container,
                command,
            } => {
                let (target, container) = (pod.clone(), container.clone());
                Some(if command.is_empty() {
                    Command::PodShell { target, container }
                } else {
                    Command::PodExec {
                        target,
                        container,
                        command: command.clone(),
                    }
                })
            }
            Self::Attach { pod, container } => Some(Command::PodAttach {
                target: pod.clone(),
                container: container.clone(),
            }),
        }
    }

    /// The tab title before the process sets one: the program's name for a local shell
    /// (`setting_shell` is the `terminal.shell` setting, used when no shell is named), the pod
    /// (and container) for a pod terminal.
    pub fn default_title(&self, setting_shell: Option<&str>) -> String {
        match self {
            Self::Local {
                title: Some(title), ..
            } => tab_title(title),
            Self::Local { shell, .. } => {
                let env = std::env::var("SHELL").ok();
                let program = resolve_shell(shell.as_deref().or(setting_shell), env.as_deref());
                let name = Path::new(&program)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or(program);
                tab_title(&name)
            }
            Self::Exec { pod, container, .. } | Self::Attach { pod, container } => {
                let name = match container {
                    Some(container) => format!("{}/{container}", pod.name),
                    None => pod.name.to_string(),
                };
                tab_title(&name)
            }
        }
    }

    /// The tab icon: a terminal for a local shell, a container for a pod's.
    pub fn icon(&self) -> IconName {
        match self {
            Self::Local { .. } => IconName::Terminal,
            Self::Exec { .. } | Self::Attach { .. } => IconName::Container,
        }
    }

    /// The saved state of a terminal tab: `{ "v": 1, "backend": { "kind": ... } }`.
    pub fn to_state(&self) -> serde_json::Value {
        serde_json::to_value(Saved {
            v: DESCRIPTOR_VERSION,
            backend: self.clone(),
        })
        // Plain data: serialisation cannot fail.
        .unwrap_or(serde_json::Value::Null)
    }

    /// Reads [`to_state`](Self::to_state)'s output; `None` for another version or shape.
    pub fn from_state(state: &serde_json::Value) -> Option<Self> {
        let saved: Saved = serde_json::from_value(state.clone()).ok()?;
        (saved.v == DESCRIPTOR_VERSION).then_some(saved.backend)
    }
}

/// `title` cut to [`MAX_TITLE_CHARS`], control characters dropped (a process may set anything).
pub(crate) fn tab_title(title: &str) -> String {
    let clean: String = title.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    if clean.chars().count() <= MAX_TITLE_CHARS {
        return clean.to_owned();
    }
    let mut cut: String = clean.chars().take(MAX_TITLE_CHARS - 1).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::{ContextName, Gvk};
    use serde_json::json;

    use super::*;

    fn cluster() -> ClusterId {
        ClusterId::new("~/.kube/config", &ContextName::new("kind-dev"))
    }

    fn pod() -> ResourceRef {
        ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0")
    }

    #[test]
    fn a_local_shell_saves_only_what_starts_it_again() {
        let descriptor = BackendDescriptor::local(Some(cluster()))
            .in_namespace(Some("shop".into()))
            .in_dir("/work");
        let state = descriptor.to_state();
        assert_eq!(
            state,
            json!({
                "v": 1,
                "backend": {
                    "kind": "local",
                    "cluster": cluster().as_str(),
                    "namespace": "shop",
                    "cwd": "/work",
                }
            })
        );
        assert_eq!(BackendDescriptor::from_state(&state), Some(descriptor));
    }

    #[test]
    fn pod_terminals_round_trip() {
        for descriptor in [
            BackendDescriptor::Exec {
                pod: pod(),
                container: Some("app".into()),
                command: vec!["/bin/sh".into()],
            },
            BackendDescriptor::Attach {
                pod: pod(),
                container: None,
            },
        ] {
            let state = descriptor.to_state();
            assert_eq!(BackendDescriptor::from_state(&state), Some(descriptor));
        }
    }

    #[test]
    fn a_pod_terminal_is_reopened_by_its_command() {
        let shell = BackendDescriptor::Exec {
            pod: pod(),
            container: Some("app".into()),
            command: Vec::new(),
        };
        assert_eq!(
            shell.pod_command(),
            Some(Command::PodShell {
                target: pod(),
                container: Some("app".into())
            })
        );
        let exec = BackendDescriptor::Exec {
            pod: pod(),
            container: None,
            command: vec!["psql".into()],
        };
        assert!(matches!(exec.pod_command(), Some(Command::PodExec { .. })));
        let attach = BackendDescriptor::Attach {
            pod: pod(),
            container: None,
        };
        assert!(matches!(
            attach.pod_command(),
            Some(Command::PodAttach { .. })
        ));
        assert_eq!(BackendDescriptor::local(None).pod_command(), None);
    }

    #[test]
    fn another_version_or_shape_is_not_restored() {
        let mut state = BackendDescriptor::local(None).to_state();
        state["v"] = json!(2);
        assert_eq!(BackendDescriptor::from_state(&state), None);
        for garbage in [
            json!(null),
            json!("zsh"),
            json!({ "v": 1 }),
            json!({ "v": 1, "backend": { "kind": "ssh" } }),
        ] {
            assert_eq!(BackendDescriptor::from_state(&garbage), None, "{garbage}");
        }
    }

    #[test]
    fn default_titles_name_the_program_or_the_pod() {
        let shell = BackendDescriptor::local(None).with_shell("/usr/local/bin/fish", vec![]);
        assert_eq!(shell.default_title(Some("/bin/zsh")), "fish");
        assert_eq!(
            BackendDescriptor::local(None).default_title(Some("/bin/zsh")),
            "zsh",
            "the setting when no shell is named"
        );
        let exec = BackendDescriptor::Exec {
            pod: pod(),
            container: Some("app".into()),
            command: vec!["sh".into()],
        };
        assert_eq!(exec.default_title(None), "web-0/app");
        assert_eq!(
            BackendDescriptor::Attach {
                pod: pod(),
                container: None
            }
            .default_title(None),
            "web-0"
        );
        assert_eq!(exec.icon(), IconName::Container);
        assert_eq!(shell.icon(), IconName::Terminal);
        assert_eq!(exec.cluster(), Some(&cluster()));
    }

    #[test]
    fn a_command_names_its_tab_and_saves_its_program_and_arguments() {
        let descriptor = BackendDescriptor::local(Some(cluster()))
            .in_namespace(Some("shop".into()))
            .with_shell(
                "/usr/local/bin/kubectl",
                vec!["logs".into(), "-f".into(), "web-0".into()],
            )
            .titled("logs web-0");
        assert_eq!(descriptor.default_title(Some("/bin/zsh")), "logs web-0");
        let state = descriptor.to_state();
        assert_eq!(state["backend"]["title"], "logs web-0");
        assert_eq!(state["backend"]["args"], json!(["logs", "-f", "web-0"]));
        assert_eq!(BackendDescriptor::from_state(&state), Some(descriptor));
        // A shell has no title field at all.
        assert!(
            BackendDescriptor::local(None).to_state()["backend"]
                .get("title")
                .is_none()
        );
    }

    #[test]
    fn titles_are_cut_and_cleaned() {
        assert_eq!(tab_title("  vim\u{7} main.rs "), "vim main.rs");
        let long = "x".repeat(100);
        let cut = tab_title(&long);
        assert_eq!(cut.chars().count(), MAX_TITLE_CHARS);
        assert!(cut.ends_with('…'));
    }
}
