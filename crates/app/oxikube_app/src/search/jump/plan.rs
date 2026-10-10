//! From a parsed line to navigation: [`plan`] resolves a [`JumpCommand`] against the cluster
//! ([`JumpEnv`]) and returns the [`Command`]s that do it.
//!
//! Nothing here touches a window or a cluster: a line becomes data, and the bar sends each
//! command through the `CommandBus` (non-negotiable 4), so a jump is guarded, audited and
//! available to an agent exactly like a click.
//!
//! | Line | Commands |
//! |---|---|
//! | `:pods` | `resource::OpenList { gvk: Pod }` |
//! | `:deploy kube-system` | `namespace::Select { [kube-system] }`, `resource::OpenList { Deployment }` |
//! | `:pod /re app=x` | `resource::OpenList { Pod }`, `table::SetFilter { "re -l app=x" }` |
//! | `:pod @prod` | `cluster::Select { prod }` first (or `cluster::Connect`, then the rest once connected) |
//! | `:ctx` | `view::Open { catalog }` |
//! | `:ctx prod` | `cluster::Select { prod }` (or `cluster::Connect`) |
//! | `:ns` | `resource::OpenList { Namespace }` |
//! | `:ns kube-system` | `namespace::Select { [kube-system] }` |
//! | `:q` | `app::Quit` (which asks first while operations are running) |
//! | `-`, `[`, `]` | `jump::Last`, `jump::Back`, `jump::Forward` |
//!
//! A user alias that expands to a command line (`fred: pod fred app=blee`) is expanded and
//! planned again, up to [`MAX_EXPANSIONS`] levels deep.

use oxikube_domain::AliasTarget;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk};

use super::ast::{HistoryStep, JumpCommand, ResourceJump};
use super::env::{JumpContext, JumpEnv};
use super::error::{ParseError, ParseErrorKind};
use super::lookup::{find_context, namespace_names, no_cluster, unknown_alias};
use super::parse::parse;
use super::span::{Span, Spanned};
use crate::search::aliases::Resolution;

/// The view id of the cluster catalog home (`oxikube_catalog_ui::catalog::CATALOG_VIEW`), where
/// the contexts are listed.
pub const CATALOG_VIEW_ID: &str = "catalog";

/// How many times an alias may expand into another before the planner gives up.
const MAX_EXPANSIONS: usize = 4;

/// What a line does, as `Command`s.
#[derive(Debug, Clone, PartialEq)]
pub struct JumpPlan {
    /// Commands to send now, in order.
    pub commands: Vec<Command>,
    /// Commands to send once a cluster is connected: a jump to a context whose session is not
    /// open connects it (in `commands`) and does the rest after.
    pub after_connect: Option<AfterConnect>,
    /// The canonical line for the history; `None` for a line that is not a navigation worth
    /// going back to (`:q`, a history step).
    pub record: Option<String>,
}

/// Commands that wait for a cluster to connect.
#[derive(Debug, Clone, PartialEq)]
pub struct AfterConnect {
    /// The cluster that is connecting.
    pub cluster: ClusterId,
    /// What to send once it is.
    pub commands: Vec<Command>,
}

impl JumpPlan {
    fn now(commands: Vec<Command>) -> Self {
        Self {
            commands,
            after_connect: None,
            record: None,
        }
    }
}

/// Parses `line` and plans it. The two steps are separate for callers that want to underline
/// while typing ([`parse`]) and run on Enter ([`plan`]).
///
/// # Errors
///
/// A [`ParseError`] for a line that is not a command or that names something the cluster does
/// not have (an unknown alias, namespace or context), with the span to underline and
/// suggestions.
pub fn plan(line: &str, env: &dyn JumpEnv) -> Result<JumpPlan, ParseError> {
    resolve(&parse(line)?, env)
}

/// Plans an already parsed line.
///
/// # Errors
///
/// See [`plan`].
pub fn resolve(command: &JumpCommand, env: &dyn JumpEnv) -> Result<JumpPlan, ParseError> {
    resolve_at(command, env, 0)
}

fn resolve_at(
    command: &JumpCommand,
    env: &dyn JumpEnv,
    depth: usize,
) -> Result<JumpPlan, ParseError> {
    let mut plan = match command {
        JumpCommand::Resource(resource) => resource_plan(resource, env, depth)?,
        JumpCommand::Ctx(name) => ctx_plan(name.as_ref(), env)?,
        JumpCommand::Ns(name) => ns_plan(name.as_ref(), env)?,
        JumpCommand::Quit => JumpPlan::now(vec![Command::AppQuit]),
        JumpCommand::History(step) => JumpPlan::now(vec![match step {
            HistoryStep::Last => Command::JumpLast,
            HistoryStep::Back => Command::JumpBack,
            HistoryStep::Forward => Command::JumpForward,
        }]),
    };
    if command.is_recorded() {
        plan.record = Some(command.to_string());
    }
    Ok(plan)
}

/// `:<alias> ...`
fn resource_plan(
    jump: &ResourceJump,
    env: &dyn JumpEnv,
    depth: usize,
) -> Result<JumpPlan, ParseError> {
    let target = match &jump.context {
        Some(name) => Some(find_context(name, env)?),
        None => None,
    };
    let cluster = match &target {
        Some(context) => context.cluster.clone(),
        None => env
            .active_cluster()
            .ok_or_else(|| no_cluster(jump.alias.span))?,
    };

    let table = env.aliases(&cluster);
    let alias = &jump.alias;
    let entry = match table.resolve(&alias.value) {
        Resolution::Unknown { suggestions } => {
            return Err(unknown_alias(alias, suggestions));
        }
        resolution => resolution.target().cloned(),
    };
    let gvr = match entry {
        Some(AliasTarget::Gvr(gvr)) => gvr,
        Some(AliasTarget::Command { name, args }) => {
            return expand(jump, &name, &args, env, depth);
        }
        None => return Err(unknown_alias(alias, Vec::new())),
    };
    let gvk = table.gvk_of(&gvr).ok_or_else(|| {
        ParseError::new(
            ParseErrorKind::NotServed,
            alias.span,
            format!(
                "`{}` leads to {gvr}, which this cluster does not serve",
                alias.value
            ),
        )
    })?;

    let mut rest = Vec::new();
    if let Some(ns) = &jump.namespace {
        rest.push(Command::NamespaceSelect {
            cluster: cluster.clone(),
            namespaces: namespace_names(ns, &cluster, env)?,
        });
    }
    rest.push(Command::ResourceOpenList {
        cluster: cluster.clone(),
        gvk: gvk.clone(),
    });
    if let Some(text) = filter_text(jump) {
        rest.push(Command::TableSetFilter {
            cluster: cluster.clone(),
            gvk,
            text,
        });
    }
    Ok(switch_cluster(target, cluster, rest))
}

/// Puts the cluster switch in front of `rest`: show the tab when it is open, connect first when
/// it is not. No switch for a line without `@ctx`.
fn switch_cluster(
    target: Option<&JumpContext>,
    cluster: ClusterId,
    mut rest: Vec<Command>,
) -> JumpPlan {
    let Some(context) = target else {
        return JumpPlan::now(rest);
    };
    rest.insert(
        0,
        Command::ClusterSelect {
            cluster: cluster.clone(),
        },
    );
    if context.connected {
        return JumpPlan::now(rest);
    }
    JumpPlan {
        commands: vec![Command::ClusterConnect {
            cluster: cluster.clone(),
        }],
        after_connect: Some(AfterConnect {
            cluster,
            commands: rest,
        }),
        record: None,
    }
}

/// The text `table::SetFilter` carries: the `/filter` and the `k=v` selector in the filter bar's
/// grammar (`re`, `-l app=x`, or `re -l app=x`). `None` when the line has neither.
fn filter_text(jump: &ResourceJump) -> Option<String> {
    let filter = jump.filter.as_ref().map(|f| f.value.as_str());
    let labels = jump.labels.as_ref().map(|l| l.value.as_str());
    match (filter, labels) {
        (None, None) => None,
        (Some(filter), None) => Some(filter.to_owned()),
        (None, Some(labels)) => Some(format!("-l {labels}")),
        // Two selectors: one list.
        (Some(filter), Some(labels)) if filter.starts_with("-l ") => {
            Some(format!("{},{labels}", filter.trim_end()))
        }
        (Some(filter), Some(labels)) => Some(format!("{filter} -l {labels}")),
    }
}

/// A user alias that is a command line: planned again as the line it stands for, with the rest of
/// what the user typed appended.
fn expand(
    jump: &ResourceJump,
    name: &str,
    args: &[String],
    env: &dyn JumpEnv,
    depth: usize,
) -> Result<JumpPlan, ParseError> {
    let alias_span = jump.alias.span;
    if depth >= MAX_EXPANSIONS {
        return Err(ParseError::new(
            ParseErrorKind::AliasLoop,
            alias_span,
            format!("`{}` expands into itself", jump.alias.value),
        ));
    }
    let mut line = name.to_owned();
    for arg in args {
        line.push(' ');
        line.push_str(arg);
    }
    // What the user typed after the alias keeps its meaning.
    let mut typed = jump.clone();
    typed.alias = Spanned::new(String::new(), alias_span);
    let tail = typed.to_string();
    if !tail.trim().is_empty() {
        line.push(' ');
        line.push_str(tail.trim());
    }
    let expanded = parse(&line).map_err(|error| error.at(alias_span))?;
    resolve_at(&expanded, env, depth + 1).map_err(|error| error.at(alias_span))
}

/// `:ctx [name]`
fn ctx_plan(name: Option<&Spanned<String>>, env: &dyn JumpEnv) -> Result<JumpPlan, ParseError> {
    let Some(name) = name else {
        return Ok(JumpPlan::now(vec![Command::ViewOpen {
            view: CATALOG_VIEW_ID.to_owned(),
        }]));
    };
    let context = find_context(name, env)?;
    Ok(switch_cluster(
        Some(context),
        context.cluster.clone(),
        Vec::new(),
    ))
}

/// `:ns [name]`
fn ns_plan(name: Option<&Spanned<String>>, env: &dyn JumpEnv) -> Result<JumpPlan, ParseError> {
    let span = name.map_or(Span::default(), |n| n.span);
    let cluster = env.active_cluster().ok_or_else(|| no_cluster(span))?;
    let command = match name {
        None => Command::ResourceOpenList {
            cluster,
            gvk: Gvk::new("", "v1", "Namespace"),
        },
        Some(name) => Command::NamespaceSelect {
            namespaces: namespace_names(name, &cluster, env)?,
            cluster,
        },
    };
    Ok(JumpPlan::now(vec![command]))
}
