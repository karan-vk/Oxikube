//! [`RoleDraft`]: the dialog's model. What the user typed, as a Role or ClusterRole manifest,
//! and what is wrong with it. No gpui: the dialog reads its inputs into one of these.

use std::collections::BTreeSet;

use oxikube_domain::rbac::{PolicyRule, Role, RoleKind};
use serde_json::{Map, Value, json};

/// The verb checkboxes, in the order the dialog lists them. `*` is every verb.
pub const VERBS: [&str; 9] = [
    "get",
    "list",
    "watch",
    "create",
    "update",
    "patch",
    "delete",
    "deletecollection",
    "*",
];

/// Verbs `resourceNames` cannot narrow: the request names no object (create, list, watch, and
/// deletecollection) or covers many.
pub const NAMELESS_VERBS: [&str; 4] = ["create", "list", "watch", "deletecollection"];

/// The longest object name the API server accepts.
const MAX_NAME: usize = 253;

/// One rule as the dialog holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleDraft {
    /// `apiGroups`; `""` is the core group.
    pub api_groups: Vec<String>,
    /// `resources`, subresources (`pods/log`) and `*`.
    pub resources: Vec<String>,
    /// `verbs`, the checked boxes first, then any verb the boxes do not offer (`bind`, `use`).
    pub verbs: Vec<String>,
    /// `resourceNames`.
    pub resource_names: Vec<String>,
    /// `nonResourceURLs`, ClusterRoles only.
    pub non_resource_urls: Vec<String>,
}

impl RuleDraft {
    /// The draft of a rule as written.
    pub fn from_policy(rule: &PolicyRule) -> Self {
        Self {
            api_groups: rule.api_groups.clone(),
            resources: rule.resources.clone(),
            verbs: rule.verbs.clone(),
            resource_names: rule.resource_names.clone(),
            non_resource_urls: rule.non_resource_urls.clone(),
        }
    }

    /// A new rule with nothing in it.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Whether the rule is about URLs rather than resources.
    pub fn is_non_resource(&self) -> bool {
        !self.non_resource_urls.is_empty()
    }

    /// Whether `verb` is in the rule.
    pub fn has_verb(&self, verb: &str) -> bool {
        self.verbs.iter().any(|own| own == verb)
    }

    /// Checks or unchecks `verb`, keeping the order the boxes list them in and the verbs the
    /// boxes do not offer after them.
    pub fn set_verb(&mut self, verb: &str, on: bool) {
        let present = self.has_verb(verb);
        if on && !present {
            self.verbs.push(verb.to_owned());
        } else if !on && present {
            self.verbs.retain(|own| own != verb);
        }
        let rank = |verb: &String| {
            VERBS
                .iter()
                .position(|own| own == verb)
                .unwrap_or(VERBS.len())
        };
        self.verbs.sort_by_key(rank);
    }

    /// The verbs the boxes do not offer, as typed in "other verbs".
    pub fn other_verbs(&self) -> Vec<String> {
        self.verbs
            .iter()
            .filter(|verb| !VERBS.contains(&verb.as_str()))
            .cloned()
            .collect()
    }

    /// Replaces the verbs the boxes do not offer.
    pub fn set_other_verbs(&mut self, other: Vec<String>) {
        self.verbs.retain(|verb| VERBS.contains(&verb.as_str()));
        for verb in other {
            if !self.has_verb(&verb) {
                self.verbs.push(verb);
            }
        }
    }

    /// The rule as the API takes it: only the lists that are not empty.
    pub fn to_json(&self) -> Value {
        let mut rule = Map::new();
        for (key, values) in [
            ("apiGroups", &self.api_groups),
            ("resources", &self.resources),
            ("resourceNames", &self.resource_names),
            ("nonResourceURLs", &self.non_resource_urls),
            ("verbs", &self.verbs),
        ] {
            if !values.is_empty() {
                rule.insert(key.to_owned(), json!(values));
            }
        }
        Value::Object(rule)
    }
}

/// How bad a [`Problem`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// The server would refuse it: saving is blocked.
    Error,
    /// Allowed, but probably not what was meant.
    Warning,
}

/// A field of a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleField {
    /// The API groups.
    ApiGroups,
    /// The resources.
    Resources,
    /// The verbs.
    Verbs,
    /// The resource names.
    ResourceNames,
    /// The non-resource URLs.
    NonResourceUrls,
}

/// Where a [`Problem`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The role's name.
    Name,
    /// The Role's namespace.
    Namespace,
    /// The rules as a whole.
    Rules,
    /// One field of rule `index`.
    Rule(usize, RuleField),
}

/// Something wrong with the draft, at a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Where.
    pub at: Field,
    /// Blocks saving or not.
    pub level: Level,
    /// What to tell the user.
    pub message: String,
}

impl Problem {
    fn error(at: Field, message: impl Into<String>) -> Self {
        Self {
            at,
            level: Level::Error,
            message: message.into(),
        }
    }

    fn warning(at: Field, message: impl Into<String>) -> Self {
        Self {
            at,
            level: Level::Warning,
            message: message.into(),
        }
    }
}

/// A Role or ClusterRole being made or edited.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleDraft {
    /// Role or ClusterRole.
    pub kind: RoleKind,
    /// The Role's namespace; unused for a ClusterRole.
    pub namespace: String,
    /// The object's name.
    pub name: String,
    /// The rules.
    pub rules: Vec<RuleDraft>,
    /// The live object when editing: its labels, annotations, aggregation rule and
    /// `resourceVersion` are kept, and the save is conditional on that version.
    pub base: Option<Value>,
}

impl RoleDraft {
    /// A new, empty Role in `namespace`, or ClusterRole.
    pub fn new(kind: RoleKind, namespace: &str) -> Self {
        Self {
            kind,
            namespace: namespace.to_owned(),
            name: String::new(),
            rules: vec![RuleDraft::empty()],
            base: None,
        }
    }

    /// The draft of the existing `role`, built from its object `live`.
    pub fn edit(role: &Role, live: Value) -> Self {
        Self {
            kind: role.id.kind,
            namespace: role.id.namespace.as_deref().unwrap_or_default().to_owned(),
            name: role.id.name.to_string(),
            rules: role.rules.iter().map(RuleDraft::from_policy).collect(),
            base: Some(live),
        }
    }

    /// Whether this edits an existing role.
    pub fn is_edit(&self) -> bool {
        self.base.is_some()
    }

    /// Whether the control plane owns the rules: an aggregated ClusterRole's `rules` are
    /// overwritten from the ClusterRoles its selectors pick, so an edit would be lost.
    pub fn rules_locked(&self) -> bool {
        self.base
            .as_ref()
            .is_some_and(|base| base.get("aggregationRule").is_some_and(Value::is_object))
    }

    /// The object to send. An edit keeps everything the dialog does not show.
    pub fn manifest(&self) -> Value {
        let mut object = self
            .base
            .clone()
            .and_then(|base| match base {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .unwrap_or_default();
        object.insert(
            "apiVersion".to_owned(),
            json!("rbac.authorization.k8s.io/v1"),
        );
        object.insert("kind".to_owned(), json!(self.kind.as_str()));
        // Server-owned: sent back, they would only make noise (or a status the server ignores).
        object.remove("status");
        let metadata = object
            .entry("metadata".to_owned())
            .or_insert_with(|| json!({}));
        if let Value::Object(metadata) = metadata {
            metadata.remove("managedFields");
            metadata.insert("name".to_owned(), json!(self.name.trim()));
            match self.kind {
                RoleKind::Role => {
                    metadata.insert("namespace".to_owned(), json!(self.namespace.trim()));
                }
                RoleKind::ClusterRole => {
                    metadata.remove("namespace");
                }
            }
        }
        if !self.rules_locked() {
            object.insert(
                "rules".to_owned(),
                Value::Array(self.rules.iter().map(RuleDraft::to_json).collect()),
            );
        }
        Value::Object(object)
    }

    /// Everything wrong with the draft, errors and warnings, in field order.
    pub fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        let name = self.name.trim();
        if name.is_empty() {
            problems.push(Problem::error(Field::Name, "Give the role a name"));
        } else if name.len() > MAX_NAME || name.contains(char::is_whitespace) || name.contains('/')
        {
            problems.push(Problem::error(
                Field::Name,
                "A name has no spaces or slashes and at most 253 characters",
            ));
        }
        if self.kind == RoleKind::Role && self.namespace.trim().is_empty() {
            problems.push(Problem::error(
                Field::Namespace,
                "A Role lives in a namespace",
            ));
        }
        if self.rules_locked() {
            return problems;
        }
        if self.rules.is_empty() {
            problems.push(Problem::error(Field::Rules, "Add at least one rule"));
        }
        for (index, rule) in self.rules.iter().enumerate() {
            self.rule_problems(index, rule, &mut problems);
        }
        problems
    }

    fn rule_problems(&self, index: usize, rule: &RuleDraft, problems: &mut Vec<Problem>) {
        let at = |field| Field::Rule(index, field);
        if rule.verbs.is_empty() {
            problems.push(Problem::error(
                at(RuleField::Verbs),
                "Pick at least one verb",
            ));
        }
        if rule.is_non_resource() {
            if self.kind == RoleKind::Role {
                problems.push(Problem::error(
                    at(RuleField::NonResourceUrls),
                    "Non-resource URLs are only allowed in a ClusterRole",
                ));
            }
            if !rule.resources.is_empty() || !rule.resource_names.is_empty() {
                problems.push(Problem::error(
                    at(RuleField::NonResourceUrls),
                    "A rule is about resources or about URLs, not both",
                ));
            }
            if let Some(url) = rule
                .non_resource_urls
                .iter()
                .find(|url| !url.starts_with('/') && url.as_str() != "*")
            {
                problems.push(Problem::error(
                    at(RuleField::NonResourceUrls),
                    format!("{url} is not a path: URLs start with /"),
                ));
            }
            return;
        }
        if rule.resources.is_empty() {
            problems.push(Problem::error(
                at(RuleField::Resources),
                "Pick at least one resource",
            ));
        }
        if rule.api_groups.is_empty() {
            problems.push(Problem::error(
                at(RuleField::ApiGroups),
                "Add an API group (\"\" is the core group)",
            ));
        }
        let nameless: BTreeSet<&str> = rule
            .verbs
            .iter()
            .map(String::as_str)
            .filter(|verb| NAMELESS_VERBS.contains(verb))
            .collect();
        if !rule.resource_names.is_empty() && !nameless.is_empty() {
            let verbs: Vec<&str> = nameless.into_iter().collect();
            problems.push(Problem::warning(
                at(RuleField::ResourceNames),
                format!(
                    "Resource names do not narrow {}: those requests name no object",
                    verbs.join(", ")
                ),
            ));
        }
    }

    /// Whether any problem blocks saving.
    pub fn has_errors(&self) -> bool {
        self.problems()
            .iter()
            .any(|problem| problem.level == Level::Error)
    }
}

/// Reads a comma- or whitespace-separated list the way the dialog's inputs take it: blanks and
/// repeats dropped, order kept.
pub fn split_list(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in text.split([',', ' ', '\n', '\t']) {
        let item = item.trim();
        if !item.is_empty() && !out.iter().any(|seen| seen == item) {
            out.push(item.to_owned());
        }
    }
    out
}

/// Reads the API groups input: like [`split_list`], with `""` and `core` meaning the core group.
pub fn split_groups(text: &str) -> Vec<String> {
    split_list(text)
        .into_iter()
        .map(|group| match group.as_str() {
            "\"\"" | "''" | "core" => String::new(),
            _ => group,
        })
        .collect()
}

/// An API group as the input shows it: the core group as `""`.
pub fn show_group(group: &str) -> String {
    if group.is_empty() {
        "\"\"".to_owned()
    } else {
        group.to_owned()
    }
}

/// A list as the input shows it.
pub fn show_list(items: &[String]) -> String {
    items.join(", ")
}
