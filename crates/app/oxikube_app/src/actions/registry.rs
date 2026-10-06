//! [`RowActionRegistry`]: which commands are row actions, and for which kinds.

use oxikube_domain::command::{Command, CommandId, Propagation};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::kinds::{ResourceKind, Verb};

/// Which kinds a row action applies to.
#[derive(Clone, Copy, Debug)]
pub enum KindFilter {
    /// Every kind.
    Any,
    /// Kinds whose server supports this verb (delete is offered for kinds that can be deleted).
    Verb(Verb),
    /// Kinds the function accepts (a per-kind action: scale for workloads with a scale
    /// subresource, cordon for nodes).
    Matching(fn(&ResourceKind) -> bool),
}

impl KindFilter {
    /// Whether the action applies to `kind`.
    pub fn accepts(&self, kind: &ResourceKind) -> bool {
        match self {
            KindFilter::Any => true,
            KindFilter::Verb(verb) => kind.supports(*verb),
            KindFilter::Matching(accepts) => accepts(kind),
        }
    }
}

/// One row action: a command, the kinds it applies to, and how an object becomes the command.
#[derive(Clone, Copy, Debug)]
pub struct RowActionSpec {
    /// The command the action dispatches. It must be registered on the bus to be offered.
    pub command: CommandId,
    /// The menu label; the command's title when `None`.
    pub label: Option<&'static str>,
    /// The kinds the action applies to.
    pub kinds: KindFilter,
    /// Builds the command for one object.
    pub build: fn(&ResourceRef) -> Command,
    /// Whether the action also applies to a selection of several objects.
    pub bulk: bool,
    /// Where the action sits in a menu: lower first, ties in registration order.
    pub order: u16,
}

impl RowActionSpec {
    /// A spec for `command`, built by `build`, for every kind, one object at a time, last.
    pub fn new(command: CommandId, build: fn(&ResourceRef) -> Command) -> Self {
        Self {
            command,
            label: None,
            kinds: KindFilter::Any,
            build,
            bulk: false,
            order: u16::MAX,
        }
    }

    /// Sets the menu label.
    #[must_use]
    pub fn label(mut self, label: &'static str) -> Self {
        self.label = Some(label);
        self
    }

    /// Sets the kinds the action applies to.
    #[must_use]
    pub fn kinds(mut self, kinds: KindFilter) -> Self {
        self.kinds = kinds;
        self
    }

    /// Offers the action for a selection of several objects too.
    #[must_use]
    pub fn bulk(mut self) -> Self {
        self.bulk = true;
        self
    }

    /// Sets the menu position.
    #[must_use]
    pub fn order(mut self, order: u16) -> Self {
        self.order = order;
        self
    }
}

/// Two specs for one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{0} is already a row action")]
pub struct DuplicateAction(pub CommandId);

/// The row actions of the app, before they are joined with the bus. See the
/// [module docs](super).
///
/// Each crate that owns per-kind actions adds its specs at start-up (E12: scale, restart, cordon,
/// drain, exec, logs); this crate only knows delete.
#[derive(Clone, Debug, Default)]
pub struct RowActionRegistry {
    specs: Vec<RowActionSpec>,
}

impl RowActionRegistry {
    /// No actions.
    pub fn new() -> Self {
        Self::default()
    }

    /// The actions every table has: delete, for any kind the server can delete, also for a
    /// selection.
    pub fn core() -> Self {
        let mut registry = Self::new();
        registry
            .register(
                RowActionSpec::new(CommandId::RESOURCE_DELETE, |target| {
                    Command::ResourceDelete {
                        target: target.clone(),
                        propagation: Propagation::default(),
                    }
                })
                .label("Delete")
                .kinds(KindFilter::Verb(Verb::Delete))
                .bulk()
                .order(900),
            )
            .expect("the core registry has one delete");
        registry
    }

    /// Adds `spec`.
    ///
    /// # Errors
    ///
    /// [`DuplicateAction`] when its command is a row action already.
    pub fn register(&mut self, spec: RowActionSpec) -> Result<(), DuplicateAction> {
        if self.specs.iter().any(|s| s.command == spec.command) {
            return Err(DuplicateAction(spec.command));
        }
        self.specs.push(spec);
        Ok(())
    }

    /// The registered specs, in registration order.
    pub fn specs(&self) -> &[RowActionSpec] {
        &self.specs
    }
}
