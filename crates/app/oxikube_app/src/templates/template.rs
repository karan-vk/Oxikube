//! [`Template`]: one manifest the "Create resource" picker offers.

use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::session::NamespaceSelection;
use serde_json::Value;

use super::prefill::{Prefill, prefill};

/// Where a template comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateOrigin {
    /// Shipped with Oxikube.
    Embedded,
    /// A `*.yaml` file of the user's `templates` directory.
    User,
}

/// A manifest to start a new resource from: one or more YAML documents with a name for the
/// picker. Cheap to clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// What the picker lists and `resource::Create` names ("Deployment"). Matched without regard
    /// to case, so a user template overrides the embedded one of the same name.
    pub name: Arc<str>,
    /// One line under the name ("A Deployment of one nginx container"); empty when none.
    pub summary: Arc<str>,
    /// Embedded or the user's.
    pub origin: TemplateOrigin,
    /// The kind of the first document, when it names one.
    pub gvk: Option<Gvk>,
    /// How many documents the text holds (empty ones do not count).
    pub documents: usize,
    /// The YAML text, with `namespace: default` where the selection's namespace goes.
    pub body: Arc<str>,
}

impl Template {
    /// A template shipped with Oxikube.
    pub fn embedded(name: &str, summary: &str, body: &str) -> Self {
        Self::build(name, summary, TemplateOrigin::Embedded, body)
    }

    /// The template a user file makes: `file_stem` names it unless a leading `# name:` comment
    /// does; a `# description:` comment is its summary.
    ///
    /// # Errors
    ///
    /// A generic reason (never a quote of the text) when the file is not a template: not YAML,
    /// or no document that is an object.
    pub fn from_user_file(file_stem: &str, text: &str) -> Result<Self, &'static str> {
        let documents = read_documents(text).ok_or("not valid YAML")?;
        if documents.is_empty() {
            return Err("holds no manifest");
        }
        if !documents.iter().all(Value::is_object) {
            return Err("holds a document that is not an object");
        }
        let header = Header::of(text);
        let name = header
            .name
            .filter(|name| !name.is_empty())
            .unwrap_or(file_stem);
        Ok(Self::build(
            name,
            header.description.unwrap_or_default(),
            TemplateOrigin::User,
            text,
        ))
    }

    fn build(name: &str, summary: &str, origin: TemplateOrigin, body: &str) -> Self {
        let documents = read_documents(body).unwrap_or_default();
        let gvk = documents.first().and_then(gvk_of);
        Self {
            name: name.trim().into(),
            summary: summary.trim().into(),
            origin,
            gvk,
            documents: documents.len(),
            body: body.into(),
        }
    }

    /// The text a new editor opens with: `body` with the namespace of `selection` in the
    /// placeholders, and what to tell the user when the selection named no single namespace.
    pub fn prefill(&self, selection: &NamespaceSelection) -> Prefill {
        prefill(&self.body, selection)
    }

    /// Whether `name` names this template (case does not matter).
    pub fn is_named(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name.trim())
    }
}

/// The non-empty documents of `text` as values; `None` when it is not YAML.
fn read_documents(text: &str) -> Option<Vec<Value>> {
    let documents: Vec<Value> = serde_saphyr::from_multiple(text).ok()?;
    Some(documents.into_iter().filter(|d| !d.is_null()).collect())
}

fn gvk_of(document: &Value) -> Option<Gvk> {
    let api_version = document.get("apiVersion")?.as_str()?;
    let kind = document.get("kind")?.as_str()?;
    Some(Gvk::from_api_version(api_version, kind))
}

/// The `# name:` and `# description:` comments at the top of a file.
struct Header<'a> {
    name: Option<&'a str>,
    description: Option<&'a str>,
}

impl<'a> Header<'a> {
    fn of(text: &'a str) -> Self {
        let mut header = Self {
            name: None,
            description: None,
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some(comment) = line.strip_prefix('#') else {
                break;
            };
            let comment = comment.trim();
            if let Some(value) = comment.strip_prefix("name:") {
                header.name.get_or_insert(value.trim());
            } else if let Some(value) = comment.strip_prefix("description:") {
                header.description.get_or_insert(value.trim());
            }
        }
        header
    }
}
