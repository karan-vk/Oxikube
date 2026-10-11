//! [`TemplatePicker`]: the "Create resource" picker. A generic
//! [`Picker`](oxikube_palette::Picker) (E11-S02) over the catalog's templates: typing filters by
//! template name or kind, Enter or a click chooses, Escape cancels (nothing is opened).

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, DismissEvent, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, Styled as _, Task, Window, div, px,
};
use oxikube_app::{Template, TemplateCatalog, TemplateOrigin};
use oxikube_palette::picker::fuzzy::{self, StringMatch, StringMatchCandidate};
use oxikube_palette::{Picker, PickerDelegate, match_label};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, u};

/// The picker over the templates.
pub type TemplatePicker = Picker<TemplatePickerDelegate>;

/// Where the chosen template goes: opens the editor on it.
pub type TemplateSink = Rc<dyn Fn(Template, &mut Window, &mut App)>;

/// The picker's width (before UI zoom): a name, its kind and a line about it.
const WIDTH: f32 = 600.;

/// What the picker lists and does. See the module docs.
pub struct TemplatePickerDelegate {
    catalog: TemplateCatalog,
    user_dir: Option<PathBuf>,
    /// "Deployment", or "web · Deployment" for a template named otherwise: what the query is
    /// matched against and what the row highlights.
    candidates: Arc<[StringMatchCandidate]>,
    matches: Vec<StringMatch>,
    selected: usize,
    choose: TemplateSink,
}

impl TemplatePickerDelegate {
    /// A delegate over `catalog`. `user_dir` is where the user's own templates go, named in the
    /// footer; `choose` receives the template that was chosen.
    pub fn new(catalog: TemplateCatalog, user_dir: Option<PathBuf>, choose: TemplateSink) -> Self {
        let candidates = catalog
            .templates()
            .iter()
            .enumerate()
            .map(|(ix, template)| StringMatchCandidate::new(ix, candidate(template)))
            .collect();
        Self {
            catalog,
            user_dir,
            candidates,
            matches: Vec::new(),
            selected: 0,
            choose,
        }
    }

    /// The picker over this delegate.
    pub fn picker(
        catalog: TemplateCatalog,
        user_dir: Option<PathBuf>,
        choose: TemplateSink,
        window: &mut Window,
        cx: &mut Context<TemplatePicker>,
    ) -> TemplatePicker {
        Picker::uniform_list(Self::new(catalog, user_dir, choose), window, cx).width(px(WIDTH))
    }

    /// The templates the current query matches, best first.
    pub fn listed(&self) -> Vec<&Template> {
        self.matches
            .iter()
            .filter_map(|m| self.catalog.templates().get(m.candidate_id))
            .collect()
    }

    /// The selected template, if the query matches any.
    pub fn selected(&self) -> Option<&Template> {
        let found = self.matches.get(self.selected)?;
        self.catalog.templates().get(found.candidate_id)
    }

    /// The catalog behind the picker.
    pub fn catalog(&self) -> &TemplateCatalog {
        &self.catalog
    }
}

/// "Deployment", or "web · Deployment" when the template's name is not its kind (so typing the
/// kind finds a user's `web` template); the name alone for a template of several documents.
fn candidate(template: &Template) -> String {
    match template.gvk.as_ref().map(|gvk| &*gvk.kind) {
        // Several documents: the name says what the template is.
        Some(kind) if template.documents == 1 && !kind.eq_ignore_ascii_case(&template.name) => {
            format!("{} · {kind}", template.name)
        }
        _ => template.name.to_string(),
    }
}

impl PickerDelegate for TemplatePickerDelegate {
    type ListItem = gpui::Div;

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn selected_index(&self) -> usize {
        self.selected
    }

    fn set_selected_index(&mut self, ix: usize, _: &mut Window, _: &mut Context<TemplatePicker>) {
        self.selected = ix;
    }

    fn placeholder_text(&self, _: &mut Window, _: &mut App) -> SharedString {
        "Create a resource from a template…".into()
    }

    fn no_matches_text(&self, _: &mut Window, _: &mut App) -> Option<SharedString> {
        Some("No template matches".into())
    }

    fn update_matches(
        &mut self,
        query: String,
        _: &mut Window,
        _: &mut Context<TemplatePicker>,
    ) -> Task<()> {
        // A few dozen templates: matched on the calling thread, well inside a frame.
        self.matches = fuzzy::match_strings(&self.candidates, &query, usize::MAX);
        self.selected = 0;
        Task::ready(())
    }

    fn confirm(&mut self, _secondary: bool, window: &mut Window, cx: &mut Context<TemplatePicker>) {
        let Some(template) = self.selected().cloned() else {
            return;
        };
        (self.choose)(template, window, cx);
        cx.emit(DismissEvent);
    }

    /// Closing without a choice opens nothing.
    fn dismissed(&mut self, _: &mut Window, _: &mut Context<TemplatePicker>) {}

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _: &mut Window,
        cx: &mut Context<TemplatePicker>,
    ) -> Option<Self::ListItem> {
        let found = self.matches.get(ix)?;
        let template = self.catalog.templates().get(found.candidate_id)?;
        let tokens = cx.tokens();
        let id = found.candidate_id;
        let origin = match template.origin {
            TemplateOrigin::Embedded => None,
            TemplateOrigin::User => Some("yours"),
        };
        Some(
            h_flex()
                .debug_selector(move || format!("template-row-{id}"))
                .gap(u(tokens.spacing.md))
                .overflow_hidden()
                .child(match_label(
                    found.string.clone(),
                    &found.positions,
                    selected,
                    cx,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(u(tokens.font.small))
                        .text_color(tokens.colors.text_muted)
                        .child(template.summary.to_string()),
                )
                .children(origin.map(|origin| {
                    div()
                        .flex_none()
                        .text_size(u(tokens.font.small))
                        .text_color(tokens.colors.accent)
                        .child(origin)
                })),
        )
    }

    fn render_footer(
        &self,
        _: &mut Window,
        cx: &mut Context<TemplatePicker>,
    ) -> Option<AnyElement> {
        let tokens = cx.tokens();
        let skipped = self.catalog.skipped();
        let mut notes: Vec<String> = Vec::new();
        if let Some(dir) = &self.user_dir {
            notes.push(format!("Your own templates: {}/*.yaml", dir.display()));
        }
        if !skipped.is_empty() {
            let files: Vec<String> = skipped
                .iter()
                .map(|s| format!("{} ({})", s.file, s.reason))
                .collect();
            notes.push(format!("Skipped: {}", files.join(", ")));
        }
        if notes.is_empty() {
            return None;
        }
        let color = if skipped.is_empty() {
            tokens.colors.text_muted
        } else {
            tokens.colors.warning
        };
        Some(
            div()
                .debug_selector(|| "template-picker-footer".into())
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.sm))
                .text_size(u(tokens.font.small))
                .text_color(color)
                .child(notes.join(" · "))
                .into_any_element(),
        )
    }
}
