//! Drawing the view: the toolbar, the path bar, the states, the status line.

use gpui::{
    AnyElement, Context, ExternalPaths, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_app::files::{ListingSource, is_secret_mount};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, Selectable as _, h_flex, v_flex};
use oxikube_ui::table::Table;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::{FilesView, LoadState};

/// `4812` as `4,812`.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl FilesView {
    fn container_buttons(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.containers.len() < 2 {
            return Vec::new();
        }
        self.containers
            .iter()
            .map(|container| {
                let name = container.name.clone();
                let selected = self.container.as_deref() == Some(&*container.name);
                let id: SharedString = format!("files-container-{}", container.name).into();
                let debug_id = id.to_string();
                div()
                    .debug_selector(move || debug_id)
                    .child(
                        Button::new(id)
                            .label(container.label())
                            .ghost()
                            .xsmall()
                            .selected(selected)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.select_container(&name, cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let show_hidden = self.table.delegate(cx).model.show_hidden();
        let has_selection = self.selected_entry(cx).is_some();
        let ready = self.state == LoadState::Ready;
        let button = |id: &'static str, label: &'static str, icon: IconName| {
            Button::new(id)
                .icon(Icon::new(icon))
                .label(label)
                .ghost()
                .xsmall()
        };
        h_flex()
            .id("files-toolbar")
            .debug_selector(|| "files-toolbar".into())
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.sm))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .children(self.container_buttons(cx))
            .child(div().flex_1())
            .child(
                div().debug_selector(|| "files-up".into()).child(
                    button("files-up", "Up", IconName::ArrowUp)
                        .disabled(self.path == "/")
                        .on_click(cx.listener(|view, _, _, cx| view.go_up(cx))),
                ),
            )
            .child(
                div().debug_selector(|| "files-hidden".into()).child(
                    button(
                        "files-hidden",
                        "Hidden",
                        if show_hidden {
                            IconName::Eye
                        } else {
                            IconName::EyeOff
                        },
                    )
                    .selected(show_hidden)
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_hidden(cx))),
                ),
            )
            .child(
                div().debug_selector(|| "files-refresh".into()).child(
                    button("files-refresh", "Refresh", IconName::RefreshCw)
                        .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                ),
            )
            .child(
                div().debug_selector(|| "files-download".into()).child(
                    button("files-download", "Download", IconName::Download)
                        .disabled(!has_selection)
                        .on_click(cx.listener(|view, _, _, cx| view.download_selected(false, cx))),
                ),
            )
            .child(
                div().debug_selector(|| "files-upload".into()).child(
                    button("files-upload", "Upload", IconName::Upload)
                        .disabled(!ready)
                        .on_click(
                            cx.listener(|view, _, window, cx| view.upload_clicked(window, cx)),
                        ),
                ),
            )
            .into_any_element()
    }

    /// The breadcrumb of the path (each part opens that directory), or the text field while the
    /// path is being edited.
    fn path_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let bar = h_flex()
            .id("files-path")
            .debug_selector(|| "files-path".into())
            .flex_none()
            .gap(u(px(2.)))
            .px(u(tokens.spacing.md))
            .py(u(px(3.)))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border_variant);
        if self.editing_path {
            return bar
                .child(div().w_full().child(Input::new(&self.path_input).xsmall()))
                .into_any_element();
        }
        let mut parts = vec![("/".to_owned(), "/".to_owned())];
        let mut so_far = String::new();
        for part in self.path.split('/').filter(|part| !part.is_empty()) {
            so_far.push('/');
            so_far.push_str(part);
            parts.push((part.to_owned(), so_far.clone()));
        }
        let crumbs = parts
            .into_iter()
            .enumerate()
            .flat_map(|(index, (label, target))| {
                let separator = (index > 1).then(|| {
                    Icon::new(IconName::ChevronRight)
                        .size(u(px(12.)))
                        .color(tokens.colors.text_muted)
                        .into_any_element()
                });
                let crumb = Button::new(SharedString::from(format!("files-crumb-{index}")))
                    .label(label)
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(move |view, _, _, cx| view.navigate(&target, cx)))
                    .into_any_element();
                separator.into_iter().chain([crumb])
            });
        bar.children(crumbs)
            .child(
                div()
                    .id("files-path-edit")
                    .debug_selector(|| "files-path-edit".into())
                    .flex_1()
                    .h(u(px(20.)))
                    .cursor_text()
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.start_editing_path(window, cx);
                    })),
            )
            .into_any_element()
    }

    fn secret_warning(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !is_secret_mount(&self.path) {
            return None;
        }
        let tokens = cx.tokens();
        Some(
            h_flex()
                .debug_selector(|| "files-secret-warning".into())
                .flex_none()
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.md))
                .py(u(px(3.)))
                .items_center()
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.warning)
                .child(Icon::new(IconName::TriangleAlert).size(u(px(13.))))
                .child("This path holds mounted secrets: what you open or download may be credentials.")
                .into_any_element(),
        )
    }

    fn message(
        &self,
        id: &'static str,
        text: SharedString,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let tokens = cx.tokens();
        v_flex()
            .id(id)
            .debug_selector(move || id.to_owned())
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.xl))
            .text_color(tokens.colors.text_muted)
            .child(div().max_w(u(px(520.))).child(text))
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.state {
            LoadState::Opening => self
                .message("files-loading", "Opening the pod…".into(), cx)
                .into_any_element(),
            LoadState::Loading => self
                .message("files-loading", "Loading…".into(), cx)
                .into_any_element(),
            LoadState::NoShell(text) => self
                .message("files-no-shell", text.clone(), cx)
                .child(Icon::new(IconName::TriangleAlert).size(u(px(20.))))
                .into_any_element(),
            LoadState::Failed(text) => {
                self.message("files-failed", text.clone(), cx)
                    .child(Button::new("files-retry").label("Retry").xsmall().on_click(
                        cx.listener(|view, _, _, cx| {
                            if view.container.is_some() {
                                view.refresh(cx);
                            } else {
                                view.open_pod(None, None, cx);
                            }
                        }),
                    ))
                    .into_any_element()
            }
            LoadState::Ready => div()
                .id("files-table")
                .debug_selector(|| "files-table".into())
                .flex_1()
                .min_h_0()
                .w_full()
                .child(Table::new(&self.table).bordered(false))
                .into_any_element(),
        }
    }

    fn status_line(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let model = self.model(cx);
        let mut text = match &self.state {
            LoadState::Ready => format!(
                "{} item{}",
                thousands(model.len()),
                if model.len() == 1 { "" } else { "s" }
            ),
            _ => String::new(),
        };
        if model.hidden() > 0 {
            text.push_str(&format!(" · {} hidden", thousands(model.hidden())));
        }
        if let Some(listing) = self
            .container
            .as_ref()
            .and_then(|c| self.cache.get(&(c.clone(), self.path.clone())))
        {
            if listing.source == ListingSource::Find {
                text.push_str(" · names only (this container has no ls)");
            }
            if listing.unreadable > 0 {
                text.push_str(&format!(" · {} unreadable", listing.unreadable));
            }
        }
        div()
            .debug_selector(|| "files-status".into())
            .flex_none()
            .px(u(tokens.spacing.md))
            .py(u(px(3.)))
            .border_t_1()
            .border_color(tokens.colors.border_variant)
            .text_size(u(tokens.font.small))
            .text_color(tokens.colors.text_muted)
            .child(SharedString::from(text))
            .into_any_element()
    }
}

impl Render for FilesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let accent = tokens.colors.accent;
        let toolbar = self.toolbar(cx);
        let path_bar = self.path_bar(cx);
        let warning = self.secret_warning(cx);
        let body = self.body(cx);
        let panel = self.transfers_panel(cx);
        let status = self.status_line(cx);
        v_flex()
            .id("files-view")
            .debug_selector(|| "files-view".into())
            .track_focus(&self.focus)
            .size_full()
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text)
            // Files dragged in from the OS are uploaded into the directory shown.
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_2().border_color(accent))
            .on_drop(cx.listener(|view, paths: &ExternalPaths, window, cx| {
                view.begin_upload(paths.paths().to_vec(), window, cx);
            }))
            .child(toolbar)
            .child(path_bar)
            .children(warning)
            .child(body)
            .children(panel)
            .child(status)
    }
}
