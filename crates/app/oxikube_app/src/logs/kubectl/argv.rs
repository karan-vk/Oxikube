//! [`KubectlTail`]: the argv of `kubectl logs -f` for one pod or one label selector.

use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{LogOptions, LogSince};

/// What `kubectl logs` reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailTarget {
    /// One pod (`kubectl logs <pod>`).
    Pod(String),
    /// Every pod a label selector matches (`kubectl logs -l <selector> --prefix`), all their
    /// containers unless [`KubectlTail::options`] names one.
    Selector(String),
}

/// A `kubectl logs` command for what a log view shows. [`argv`](Self::argv) is the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubectlTail {
    /// The kubeconfig context (`--context`).
    pub context: String,
    /// The namespace of the pod or pods (`--namespace`).
    pub namespace: String,
    /// The pod, or the selector of the pods.
    pub target: TailTarget,
    /// What the view reads: container, previous instance, tail length, since, byte limit. The
    /// view's own request, so the terminal shows the lines the viewer would.
    pub options: LogOptions,
    /// Prefix each line with its timestamp (`--timestamps`): the view's toggle, not the
    /// `timestamps` of [`options`](Self::options), which the viewer always asks the server for.
    pub timestamps: bool,
    /// Most pods read at once for a selector (`--max-log-requests`); kubectl's own default is 5.
    pub max_log_requests: usize,
}

impl KubectlTail {
    /// The arguments after the program: `logs -f --context=... --namespace=... ...`.
    ///
    /// `-f` is present when the view follows; a range that stops (the head, the previous
    /// instance) has none, because kubectl refuses `--follow` with `--previous` and a read that
    /// ends has nothing to follow. A selector always gets a `--tail`, as kubectl's default for a
    /// selector is 10 lines.
    ///
    /// # Errors
    ///
    /// A validation error for an empty context, namespace, pod name or selector, and for a pod
    /// name that starts with `-` (it would be read as a flag).
    pub fn argv(&self) -> OxiResult<Vec<String>> {
        require("context", &self.context)?;
        require("namespace", &self.namespace)?;
        let mut args = vec!["logs".to_owned()];
        if self.options.follow {
            args.push("-f".to_owned());
        }
        args.push(format!("--context={}", self.context));
        args.push(format!("--namespace={}", self.namespace));
        if let Some(container) = &self.options.container {
            require("container", container)?;
            args.push(format!("--container={container}"));
        } else if matches!(self.target, TailTarget::Selector(_)) {
            args.push("--all-containers=true".to_owned());
        }
        if self.options.previous {
            args.push("--previous".to_owned());
        }
        if self.timestamps {
            args.push("--timestamps".to_owned());
        }
        match self.options.since {
            Some(LogSince::Seconds(seconds)) => args.push(format!("--since={seconds}s")),
            Some(LogSince::Time(time)) => args.push(format!("--since-time={time}")),
            None => {}
        }
        match (self.options.tail_lines, &self.target) {
            (Some(lines), _) => args.push(format!("--tail={lines}")),
            (None, TailTarget::Selector(_)) => args.push("--tail=-1".to_owned()),
            (None, TailTarget::Pod(_)) => {}
        }
        if let Some(bytes) = self.options.limit_bytes {
            args.push(format!("--limit-bytes={bytes}"));
        }
        match &self.target {
            TailTarget::Pod(name) => {
                require("pod name", name)?;
                if name.starts_with('-') {
                    return Err(OxiError::validation(format!(
                        "`{name}` cannot be a pod name: it would be read as an option"
                    )));
                }
                args.push(name.clone());
            }
            TailTarget::Selector(selector) => {
                require("selector", selector)?;
                args.push(format!("--selector={selector}"));
                args.push("--prefix".to_owned());
                args.push(format!(
                    "--max-log-requests={}",
                    self.max_log_requests.max(1)
                ));
            }
        }
        Ok(args)
    }
}

fn require(what: &str, value: &str) -> OxiResult<()> {
    if value.trim().is_empty() {
        return Err(OxiError::validation(format!("the {what} is empty")));
    }
    Ok(())
}
