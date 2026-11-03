//! The frame of the settings page: navigation on the left, the search field and the selected
//! section (or the search results) on the right.

use gpui::{
    AnyElement, Context, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledText, Window, div, prelude::FluentBuilder as _, px,
};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::SettingsPage;
use crate::page::search::SearchHit;

/// Width of the navigation column, unscaled.
const NAV_WIDTH: f32 = 220.0;

impl Render for SettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let nav = self.navigation(cx);
        let content = if self.query_text.is_empty() {
            self.section_content(cx)
        } else {
            self.results(cx)
        };
        h_flex()
            .id("settings-page")
            .debug_selector(|| "settings-page".into())
            .key_context("SettingsPage")
            .track_focus(&self.focus)
            .size_full()
            .items_start()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(nav)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        div()
                            .debug_selector(|| "settings-search".into())
                            .flex_none()
                            .px(u(tokens.spacing.xl))
                            .py(u(tokens.spacing.lg))
                            .border_b_1()
                            .border_color(colors.border_variant)
                            .child(Input::new(&self.query).small().cleanable(true)),
                    )
                    .child(
                        v_flex()
                            .id("settings-content")
                            .debug_selector(|| "settings-content".into())
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.content_scroll)
                            .px(u(tokens.spacing.xl))
                            .pb(u(tokens.spacing.xl))
                            .children(content),
                    ),
            )
    }
}

impl SettingsPage {
    /// The list of sections; while searching, each shows how many results it holds.
    fn navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let searching = !self.query_text.is_empty();
        let rows = self
            .model
            .sections()
            .iter()
            .enumerate()
            .map(|(ix, section)| {
                let key = section.key.clone();
                let matches = self.hits.iter().filter(|hit| hit.section == ix).count();
                let selected = !searching && ix == self.selected;
                let dimmed = searching && matches == 0;
                h_flex()
                    .id(("settings-nav", ix))
                    .debug_selector(move || format!("settings-nav-{key}"))
                    .gap(u(tokens.spacing.md))
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.sm))
                    .rounded(u(tokens.radius.md))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(colors.element_selected))
                    .hover(|style| style.bg(colors.element_hover))
                    .text_color(if dimmed {
                        colors.text_disabled
                    } else {
                        colors.text
                    })
                    .child(div().flex_1().truncate().child(section.title.clone()))
                    .when(searching && matches > 0, |row| {
                        row.child(
                            div()
                                .text_color(colors.text_muted)
                                .child(matches.to_string()),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !this.query_text.is_empty() {
                            this.query
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            this.query_text.clear();
                            this.hits.clear();
                        }
                        this.select_section(ix, cx);
                    }))
            });
        v_flex()
            .id("settings-nav")
            .debug_selector(|| "settings-nav".into())
            .flex_none()
            .w(u(px(NAV_WIDTH)))
            .h_full()
            .overflow_y_scroll()
            .gap(u(tokens.spacing.xs))
            .p(u(tokens.spacing.md))
            .border_r_1()
            .border_color(colors.border_variant)
            .bg(colors.surface)
            .child(
                div()
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.md))
                    .text_size(u(tokens.font.heading))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .children(rows)
            .into_any_element()
    }

    /// The selected section: its header, then one row per field.
    fn section_content(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Some(section) = self.model.section(self.selected) else {
            return vec![
                div()
                    .py(u(tokens.spacing.xl))
                    .text_color(colors.text_muted)
                    .child("No settings are registered.")
                    .into_any_element(),
            ];
        };
        let header = v_flex()
            .debug_selector(|| "settings-section-header".into())
            .gap(u(tokens.spacing.xs))
            .pt(u(tokens.spacing.xl))
            .pb(u(tokens.spacing.md))
            .child(
                div()
                    .text_size(u(tokens.font.heading))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(section.title.clone()),
            )
            .children(
                section
                    .description
                    .clone()
                    .map(|text| div().text_color(colors.text_muted).child(text)),
            )
            .into_any_element();
        let mut children = Vec::with_capacity(section.fields.len() + 1);
        children.push(header);
        for (ix, field) in section.fields.iter().enumerate() {
            let highlighted = self.highlighted == Some(ix);
            children.push(self.field_row(field, highlighted, cx));
        }
        children
    }

    /// The results of the query: section and field title (the match highlighted) and the start
    /// of the description. A click opens the field.
    fn results(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let summary = match self.hits.len() {
            0 => format!("No settings match \"{}\".", self.query_text),
            1 => "1 result".to_owned(),
            n => format!("{n} results"),
        };
        let mut children = Vec::with_capacity(self.hits.len() + 1);
        children.push(
            div()
                .debug_selector(|| "settings-results-summary".into())
                .pt(u(tokens.spacing.lg))
                .pb(u(tokens.spacing.md))
                .text_color(colors.text_muted)
                .child(summary)
                .into_any_element(),
        );
        let highlight = HighlightStyle {
            color: Some(colors.accent),
            font_weight: Some(FontWeight::BOLD),
            ..HighlightStyle::default()
        };
        for (ix, hit) in self.hits.iter().enumerate() {
            let Some((trail, title, description)) = self.hit_texts(hit) else {
                continue;
            };
            let ranges = hit
                .title_ranges
                .iter()
                .map(|range| (range.clone(), highlight));
            let title = StyledText::new(title).with_highlights(ranges);
            children.push(
                v_flex()
                    .id(("settings-hit", ix))
                    .debug_selector(move || format!("settings-hit-{ix}"))
                    .gap(u(tokens.spacing.xs))
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.md))
                    .rounded(u(tokens.radius.md))
                    .cursor_pointer()
                    .hover(|style| style.bg(colors.element_hover))
                    .child(
                        h_flex()
                            .gap(u(tokens.spacing.md))
                            .child(div().text_color(colors.text_muted).child(trail))
                            .child(div().font_weight(FontWeight::MEDIUM).child(title)),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(u(tokens.font.small))
                            .text_color(colors.text_muted)
                            .child(description),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.open_hit(ix, window, cx)))
                    .into_any_element(),
            );
        }
        children
    }

    /// What a result row shows: where it is ("Terminal ›", or "Section"), its title and its
    /// description.
    fn hit_texts(&self, hit: &SearchHit) -> Option<(SharedString, SharedString, SharedString)> {
        let section = self.model.section(hit.section)?;
        match hit.field {
            Some(field) => {
                let field = section.fields.get(field)?;
                Some((
                    format!("{} \u{203a}", section.title).into(),
                    field.title.clone(),
                    field.description.clone(),
                ))
            }
            None => Some((
                "Section".into(),
                section.title.clone(),
                section.description.clone().unwrap_or_default(),
            )),
        }
    }
}
