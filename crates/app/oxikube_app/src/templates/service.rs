//! [`TemplateService`]: the embedded templates and the user's, as the picker lists them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_ports::{EntryKind, FsPort};

use super::template::Template;

/// The largest user template read. A manifest template is a few hundred lines; a larger file is
/// not one.
const MAX_USER_TEMPLATE_BYTES: u64 = 256 * 1024;

/// A file of the user's templates directory that is not a usable template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedTemplate {
    /// The file name.
    pub file: String,
    /// Why, in words that quote nothing of its text.
    pub reason: String,
}

/// The templates the picker offers, and the files it skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateCatalog {
    templates: Vec<Template>,
    skipped: Vec<SkippedTemplate>,
}

impl TemplateCatalog {
    /// The templates: the embedded ones in their order (a user template of the same name in the
    /// place of the one it overrides), then the user's own by name.
    pub fn templates(&self) -> &[Template] {
        &self.templates
    }

    /// The template called `name` (case does not matter).
    pub fn get(&self, name: &str) -> Option<&Template> {
        self.templates.iter().find(|t| t.is_named(name))
    }

    /// The user files that were not templates.
    pub fn skipped(&self) -> &[SkippedTemplate] {
        &self.skipped
    }

    /// A catalog of `templates`, nothing skipped (the embedded ones, before the user's directory
    /// has been read).
    pub fn of(templates: Vec<Template>) -> Self {
        Self {
            templates,
            skipped: Vec::new(),
        }
    }
}

/// Lists the templates for the "Create resource" picker. Cheap to clone.
///
/// The user's directory is scanned each time [`catalog`](Self::catalog) is called, which is when
/// the picker opens, so a file added meanwhile shows up without a watcher.
#[derive(Clone)]
pub struct TemplateService {
    embedded: Arc<[Template]>,
    fs: Arc<dyn FsPort>,
    dir: PathBuf,
}

impl std::fmt::Debug for TemplateService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemplateService")
            .field("embedded", &self.embedded.len())
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl TemplateService {
    /// A service over the `embedded` templates and the `*.yaml` files in `user_dir`
    /// (`<config dir>/templates`), read through `fs`.
    pub fn new(embedded: Vec<Template>, fs: Arc<dyn FsPort>, user_dir: PathBuf) -> Self {
        Self {
            embedded: embedded.into(),
            fs,
            dir: user_dir,
        }
    }

    /// The embedded templates alone, with no read: what the picker shows while the directory is
    /// being scanned.
    pub fn embedded(&self) -> TemplateCatalog {
        TemplateCatalog::of(self.embedded.to_vec())
    }

    /// Where the user's templates are.
    pub fn user_dir(&self) -> &Path {
        &self.dir
    }

    /// The embedded templates with the user's merged in. A directory that does not exist is no
    /// user templates; a file that is not a template is skipped (named in
    /// [`TemplateCatalog::skipped`] and logged as a warning without its text), never an error.
    pub async fn catalog(&self) -> TemplateCatalog {
        let mut skipped = Vec::new();
        let mut user = Vec::new();
        match self.fs.list(&self.dir).await {
            Ok(entries) => {
                for entry in entries {
                    // A symlink (dotfiles repos keep one) is read through; a directory is not a file.
                    if !matches!(entry.kind, EntryKind::File | EntryKind::Symlink)
                        || !is_yaml(&entry.path)
                    {
                        continue;
                    }
                    let file = file_name(&entry.path);
                    match self.read(&entry.path, entry.size).await {
                        Ok(template) => user.push(template),
                        Err(reason) => {
                            tracing::warn!(file = %file, reason, "a user template was skipped");
                            skipped.push(SkippedTemplate {
                                file,
                                reason: reason.to_owned(),
                            });
                        }
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(dir = %self.dir.display(), %error, "the user templates could not be listed");
                skipped.push(SkippedTemplate {
                    file: file_name(&self.dir),
                    reason: "the templates directory could not be read".to_owned(),
                });
            }
        }
        user.sort_by_key(|t| t.name.to_lowercase());
        let mut templates: Vec<Template> = self.embedded.to_vec();
        for template in user {
            match templates.iter_mut().find(|t| t.is_named(&template.name)) {
                // The same name: the user's replaces the embedded one where it stood (and a
                // second user file of that name, in file order, replaces the first).
                Some(existing) => *existing = template,
                None => templates.push(template),
            }
        }
        TemplateCatalog { templates, skipped }
    }

    async fn read(&self, path: &Path, size: Option<u64>) -> Result<Template, &'static str> {
        if size.is_some_and(|size| size > MAX_USER_TEMPLATE_BYTES) {
            return Err("is too large for a template");
        }
        let bytes = self.fs.read(path).await.map_err(|_| "could not be read")?;
        if bytes.len() as u64 > MAX_USER_TEMPLATE_BYTES {
            return Err("is too large for a template");
        }
        let text = String::from_utf8(bytes).map_err(|_| "is not UTF-8 text")?;
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        Template::from_user_file(stem, &text)
    }
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}
