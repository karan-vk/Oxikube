//! [`ThemeDelegate`]: the picker delegate behind `theme::Select`.
//!
//! The picker supplies the query field, the keyboard selection and the virtualised list. The
//! delegate lists the registry's themes in two groups (the appearance in effect first), previews
//! the selected one, writes the setting on confirm and reverts on any other way out.

use gpui::{
    AnyElement, App, Context, DismissEvent, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, Styled as _, Task, Window, div,
};
use oxikube_theme::{
    ActiveTheme, Appearance, SystemAppearance, ThemeChoice, ThemeMeta, ThemeRegistry,
    clear_preview, commit_theme, current_choice, preview_theme,
};
use oxikube_ui::{ActiveTokens as _, u};

use super::render;
use crate::picker::fuzzy::{self, INLINE_MATCH_LIMIT, StringMatch, StringMatchCandidate};
use crate::picker::{Picker, PickerDelegate};

/// One line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A group heading; cannot be selected.
    Header {
        /// The appearance the group holds.
        appearance: Appearance,
        /// How many themes it lists for the current query.
        count: usize,
    },
    /// A theme.
    Theme {
        /// Its index in [`ThemeDelegate::themes`].
        theme: usize,
        /// Byte offsets of the query characters in its name.
        positions: Vec<usize>,
    },
}

/// The theme picker's delegate. See the module docs.
pub struct ThemeDelegate {
    themes: Vec<ThemeMeta>,
    names: Vec<StringMatchCandidate>,
    rows: Vec<Row>,
    selected: usize,
    /// The theme the window showed when the picker opened (marked "Current").
    current: String,
    /// The theme on screen because of this picker (the previewed one, or `current`).
    shown: String,
    choice: ThemeChoice,
    /// The appearance listed first.
    first: Appearance,
    confirmed: bool,
}

impl ThemeDelegate {
    /// A delegate over the installed registry and the current setting. Needs
    /// `oxikube_theme::init` to have run.
    pub fn new(cx: &App) -> Self {
        let themes = ThemeRegistry::global(cx).list();
        let names = themes
            .iter()
            .enumerate()
            .map(|(ix, meta)| StringMatchCandidate::new(ix, meta.name.clone()))
            .collect();
        let current = ActiveTheme::get(cx).name.clone();
        let choice = current_choice(cx);
        let first = match choice.slot {
            oxikube_theme::ThemeSlot::Light => Appearance::Light,
            oxikube_theme::ThemeSlot::Dark => Appearance::Dark,
            oxikube_theme::ThemeSlot::Any => SystemAppearance::get(cx),
        };
        let mut delegate = Self {
            themes,
            names,
            rows: Vec::new(),
            selected: 0,
            shown: current.clone(),
            current,
            choice,
            first,
            confirmed: false,
        };
        let matches = fuzzy::match_strings(&delegate.names, "", usize::MAX);
        delegate.set_rows("", &matches);
        delegate
    }

    /// The registered themes, in registry order.
    pub fn themes(&self) -> &[ThemeMeta] {
        &self.themes
    }

    /// The lines of the current query.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The name of the selected theme; `None` when nothing matches.
    pub fn selected_name(&self) -> Option<&str> {
        match self.rows.get(self.selected)? {
            Row::Theme { theme, .. } => Some(&self.themes[*theme].name),
            Row::Header { .. } => None,
        }
    }

    /// Whether a pick has been confirmed (the picker is closing with a write under way).
    pub fn confirmed(&self) -> bool {
        self.confirmed
    }

    fn set_rows(&mut self, query: &str, matches: &[StringMatch]) {
        self.rows.clear();
        let other = match self.first {
            Appearance::Dark => Appearance::Light,
            Appearance::Light => Appearance::Dark,
        };
        for appearance in [self.first, other] {
            let group: Vec<&StringMatch> = matches
                .iter()
                .filter(|found| self.themes[found.candidate_id].appearance == appearance)
                .collect();
            if group.is_empty() {
                continue;
            }
            self.rows.push(Row::Header {
                appearance,
                count: group.len(),
            });
            self.rows.extend(group.into_iter().map(|found| Row::Theme {
                theme: found.candidate_id,
                positions: found.positions.clone(),
            }));
        }
        let current_row = if query.trim().is_empty() {
            self.rows.iter().position(|row| {
                matches!(row, Row::Theme { theme, .. } if self.themes[*theme].name == self.current)
            })
        } else {
            None
        };
        self.selected = current_row
            .or_else(|| {
                self.rows
                    .iter()
                    .position(|row| matches!(row, Row::Theme { .. }))
            })
            .unwrap_or(0);
    }

    /// Shows the selected theme: previewed, or (back on the one the window opened with) the
    /// setting's own. A no-op while it is already on screen.
    fn sync_preview(&mut self, cx: &mut App) {
        let Some(name) = self.selected_name().map(str::to_owned) else {
            return;
        };
        if name == self.shown {
            return;
        }
        if name == self.current {
            clear_preview(cx);
        } else {
            preview_theme(cx, &name);
        }
        self.shown = name;
    }
}

impl PickerDelegate for ThemeDelegate {
    type ListItem = gpui::Stateful<gpui::Div>;

    fn match_count(&self) -> usize {
        self.rows.len()
    }

    fn selected_index(&self) -> usize {
        self.selected
    }

    fn set_selected_index(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Picker<Self>>) {
        self.selected = ix;
        self.sync_preview(cx);
        cx.notify();
    }

    fn can_select(&self, ix: usize, _: &mut Window, _: &mut Context<Picker<Self>>) -> bool {
        matches!(self.rows.get(ix), Some(Row::Theme { .. }))
    }

    fn placeholder_text(&self, _: &mut Window, _: &mut App) -> SharedString {
        "Select a theme".into()
    }

    fn no_matches_text(&self, _: &mut Window, _: &mut App) -> Option<SharedString> {
        Some(if self.themes.is_empty() {
            "No themes are installed".into()
        } else {
            "No themes match".into()
        })
    }

    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        if self.names.len() <= INLINE_MATCH_LIMIT {
            let matches = fuzzy::match_strings(&self.names, &query, usize::MAX);
            self.set_rows(&query, &matches);
            self.sync_preview(cx);
            return Task::ready(());
        }
        let candidates: std::sync::Arc<[StringMatchCandidate]> = self.names.clone().into();
        cx.spawn_in(window, async move |picker, cx| {
            let executor = cx.background_executor().clone();
            let matches =
                fuzzy::match_strings_async(candidates, query.clone(), usize::MAX, &executor).await;
            picker
                .update(cx, |picker, cx| {
                    picker.delegate.set_rows(&query, &matches);
                    picker.delegate.sync_preview(cx);
                })
                .ok();
        })
    }

    fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(name) = self.selected_name().map(str::to_owned) {
            self.confirmed = true;
            // The preview stays up until the write lands (commit_theme ends it), so the committed
            // theme replaces the previewed one without a flash of the old setting.
            let write = commit_theme(cx, &name);
            // Finite (one settings write), so detaching is not a self-dropping task.
            cx.spawn(async move |_, _| {
                if let Err(err) = write.await {
                    tracing::warn!(%err, theme = %name, "could not save the theme");
                }
            })
            .detach();
        }
        cx.emit(DismissEvent);
    }

    fn dismissed(&mut self, _: &mut Window, cx: &mut Context<Picker<Self>>) {
        if !self.confirmed {
            clear_preview(cx);
        }
    }

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        Some(match self.rows.get(ix)? {
            Row::Header { appearance, count } => render::header(*appearance, *count, ix, cx),
            Row::Theme { theme, positions } => {
                let name = SharedString::from(self.themes[*theme].name.clone());
                let current = *name == self.current;
                render::entry(&name, positions, current, selected, ix, cx)
            }
        })
    }

    fn render_header(&self, _: &mut Window, cx: &mut Context<Picker<Self>>) -> Option<AnyElement> {
        let tokens = cx.tokens();
        Some(
            div()
                .debug_selector(|| "theme-slot".to_owned())
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.sm))
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text_muted)
                .child(render::slot_caption(self.choice))
                .into_any_element(),
        )
    }

    fn render_footer(&self, _: &mut Window, cx: &mut Context<Picker<Self>>) -> Option<AnyElement> {
        let tokens = cx.tokens();
        Some(
            div()
                .debug_selector(|| "theme-footer".to_owned())
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.sm))
                .border_t_1()
                .border_color(tokens.colors.border_variant)
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text_muted)
                .child("Up and down to preview, Enter to keep, Esc to go back.")
                .into_any_element(),
        )
    }
}
