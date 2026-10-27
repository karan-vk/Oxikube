//! [`BindingDraft`]: the binding dialog's model. The manifest it builds and the problems it
//! finds; no gpui.

use oxikube_domain::rbac::{
    Binding, BindingKind, Escalation, RBAC_GROUP, RoleKind, RoleRef, Subject,
};
use serde_json::{Value, json};

use crate::role::Level;

/// The longest name the API accepts.
const MAX_NAME: usize = 253;

/// Which kind a subject is: the dialog's select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubjectKind {
    /// A user, typed.
    #[default]
    User,
    /// A group, typed.
    Group,
    /// A ServiceAccount, picked: it needs a namespace.
    ServiceAccount,
}

impl SubjectKind {
    /// The three, in the select's order.
    pub const ALL: [SubjectKind; 3] = [
        SubjectKind::User,
        SubjectKind::Group,
        SubjectKind::ServiceAccount,
    ];

    /// The `kind` the API takes.
    pub fn as_str(self) -> &'static str {
        match self {
            SubjectKind::User => "User",
            SubjectKind::Group => "Group",
            SubjectKind::ServiceAccount => "ServiceAccount",
        }
    }
}

/// One subject as typed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubjectDraft {
    /// User, Group or ServiceAccount.
    pub kind: SubjectKind,
    /// The name.
    pub name: String,
    /// A ServiceAccount's namespace; unused for the others.
    pub namespace: String,
}

impl SubjectDraft {
    /// The draft of an existing `subject`.
    pub fn of(subject: &Subject) -> Self {
        match subject {
            Subject::User(name) => Self {
                kind: SubjectKind::User,
                name: name.to_string(),
                namespace: String::new(),
            },
            Subject::Group(name) => Self {
                kind: SubjectKind::Group,
                name: name.to_string(),
                namespace: String::new(),
            },
            Subject::ServiceAccount { namespace, name } => Self {
                kind: SubjectKind::ServiceAccount,
                name: name.to_string(),
                namespace: namespace.to_string(),
            },
        }
    }

    /// The domain subject, once the fields are filled.
    pub fn subject(&self) -> Option<Subject> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        match self.kind {
            SubjectKind::User => Some(Subject::User(name.into())),
            SubjectKind::Group => Some(Subject::Group(name.into())),
            SubjectKind::ServiceAccount => {
                let namespace = self.namespace.trim();
                (!namespace.is_empty()).then(|| Subject::service_account(namespace, name))
            }
        }
    }

    /// The identity the subject authenticates as, shown under the row: the user name, the group
    /// name, or `system:serviceaccount:<ns>:<name>`.
    pub fn identity(&self) -> Option<String> {
        match self.subject()? {
            Subject::Group(name) => Some(format!("group {name}")),
            other => other.username().map(|name| name.to_string()),
        }
    }

    /// The entry of `subjects` with the `apiGroup` the kind takes: the RBAC group for a User or
    /// a Group, none for a ServiceAccount.
    pub fn to_json(&self) -> Value {
        let name = self.name.trim();
        match self.kind {
            SubjectKind::ServiceAccount => json!({
                "kind": "ServiceAccount", "name": name, "namespace": self.namespace.trim(),
            }),
            kind => json!({"apiGroup": RBAC_GROUP, "kind": kind.as_str(), "name": name}),
        }
    }
}

/// A field of the dialog a [`Problem`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The binding's name.
    Name,
    /// The RoleBinding's namespace.
    Namespace,
    /// The role it grants.
    Role,
    /// The subjects as a whole.
    Subjects,
    /// The name of subject `index`.
    SubjectName(usize),
    /// The namespace of ServiceAccount subject `index`.
    SubjectNamespace(usize),
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

/// A RoleBinding or ClusterRoleBinding being made or edited.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingDraft {
    /// RoleBinding or ClusterRoleBinding.
    pub kind: BindingKind,
    /// The RoleBinding's namespace; unused for a ClusterRoleBinding.
    pub namespace: String,
    /// The object's name.
    pub name: String,
    /// The role it grants; unset until picked.
    pub role: Option<RoleRef>,
    /// Who it grants to.
    pub subjects: Vec<SubjectDraft>,
    /// The live object when editing: its labels and annotations are kept, and the save is
    /// conditional on its `resourceVersion`.
    pub base: Option<Value>,
}

impl BindingDraft {
    /// A new, empty binding.
    pub fn new(kind: BindingKind, namespace: &str) -> Self {
        Self {
            kind,
            namespace: namespace.to_owned(),
            name: String::new(),
            role: None,
            subjects: vec![SubjectDraft::default()],
            base: None,
        }
    }

    /// The draft of the existing `binding`, built from its object `live`.
    pub fn edit(binding: &Binding, live: Value) -> Self {
        Self {
            kind: binding.id.kind,
            namespace: binding
                .id
                .namespace
                .as_deref()
                .unwrap_or_default()
                .to_owned(),
            name: binding.id.name.to_string(),
            role: Some(binding.role_ref.clone()),
            subjects: binding.subjects.iter().map(SubjectDraft::of).collect(),
            base: Some(live),
        }
    }

    /// Whether this edits an existing binding.
    pub fn is_edit(&self) -> bool {
        self.base.is_some()
    }

    /// The `roleRef` of the live object, when editing.
    pub fn live_role(&self) -> Option<RoleRef> {
        let base = self.base.as_ref()?;
        let kind = match base.pointer("/roleRef/kind")?.as_str()? {
            "Role" => RoleKind::Role,
            "ClusterRole" => RoleKind::ClusterRole,
            _ => return None,
        };
        let name = base.pointer("/roleRef/name")?.as_str()?;
        Some(RoleRef {
            kind,
            name: name.into(),
        })
    }

    /// Whether saving changes the `roleRef` of an existing binding, which the API only allows by
    /// making a new object.
    pub fn role_changed(&self) -> bool {
        self.is_edit() && self.live_role() != self.role
    }

    /// The subjects that are filled in.
    pub fn subjects_filled(&self) -> Vec<Subject> {
        self.subjects
            .iter()
            .filter_map(SubjectDraft::subject)
            .collect()
    }

    /// The reasons this grant deserves a louder warning (`cluster-admin`, everybody).
    pub fn escalations(&self) -> Vec<Escalation> {
        match &self.role {
            Some(role) => Escalation::of(role, &self.subjects_filled()),
            None => Vec::new(),
        }
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
        object.remove("status");
        let metadata = object
            .entry("metadata".to_owned())
            .or_insert_with(|| json!({}));
        if let Value::Object(metadata) = metadata {
            metadata.remove("managedFields");
            metadata.insert("name".to_owned(), json!(self.name.trim()));
            match self.kind {
                BindingKind::RoleBinding => {
                    metadata.insert("namespace".to_owned(), json!(self.namespace.trim()));
                }
                BindingKind::ClusterRoleBinding => {
                    metadata.remove("namespace");
                }
            }
        }
        if let Some(role) = &self.role {
            object.insert(
                "roleRef".to_owned(),
                json!({"apiGroup": RBAC_GROUP, "kind": role.kind.as_str(), "name": &*role.name}),
            );
        }
        object.insert(
            "subjects".to_owned(),
            Value::Array(self.subjects.iter().map(SubjectDraft::to_json).collect()),
        );
        Value::Object(object)
    }

    /// Everything wrong with the draft, errors and warnings, in field order.
    pub fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        let name = self.name.trim();
        if name.is_empty() {
            problems.push(Problem::error(Field::Name, "Give the binding a name"));
        } else if name.len() > MAX_NAME || name.contains(char::is_whitespace) || name.contains('/')
        {
            problems.push(Problem::error(
                Field::Name,
                "A name has no spaces or slashes and at most 253 characters",
            ));
        }
        if self.kind == BindingKind::RoleBinding && self.namespace.trim().is_empty() {
            problems.push(Problem::error(
                Field::Namespace,
                "A RoleBinding lives in a namespace",
            ));
        }
        match &self.role {
            None => problems.push(Problem::error(Field::Role, "Pick the role to grant")),
            Some(role)
                if self.kind == BindingKind::ClusterRoleBinding && role.kind == RoleKind::Role =>
            {
                problems.push(Problem::error(
                    Field::Role,
                    "A ClusterRoleBinding can only grant a ClusterRole",
                ));
            }
            Some(_) => {}
        }
        if self.subjects.is_empty() {
            problems.push(Problem::warning(
                Field::Subjects,
                "No subjects: the binding grants nothing to anyone",
            ));
        }
        for (index, subject) in self.subjects.iter().enumerate() {
            self.subject_problems(index, subject, &mut problems);
        }
        problems
    }

    fn subject_problems(&self, index: usize, subject: &SubjectDraft, out: &mut Vec<Problem>) {
        let name = subject.name.trim();
        if name.is_empty() {
            out.push(Problem::error(
                Field::SubjectName(index),
                "Give the subject a name",
            ));
        } else if name.contains(char::is_whitespace) {
            out.push(Problem::error(
                Field::SubjectName(index),
                "A subject name has no spaces",
            ));
        } else if subject.kind == SubjectKind::ServiceAccount && name.contains(':') {
            out.push(Problem::error(
                Field::SubjectName(index),
                "A ServiceAccount name has no colon; pick it from the list",
            ));
        } else if subject.kind != SubjectKind::ServiceAccount
            && matches!(Subject::from_username(name), Subject::ServiceAccount { .. })
        {
            out.push(Problem::warning(
                Field::SubjectName(index),
                "That is a ServiceAccount's user name: choose the ServiceAccount kind instead",
            ));
        }
        if subject.kind == SubjectKind::ServiceAccount && subject.namespace.trim().is_empty() {
            out.push(Problem::error(
                Field::SubjectNamespace(index),
                "A ServiceAccount subject needs its namespace",
            ));
        }
        if let Some(own) = subject.subject()
            && self.subjects[..index]
                .iter()
                .any(|before| before.subject().as_ref() == Some(&own))
        {
            out.push(Problem::warning(
                Field::SubjectName(index),
                "This subject is listed twice",
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
