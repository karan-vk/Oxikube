//! Renders each curated component once with the default tokens (E05-S02 acceptance test).
//!
//! Each test mounts the component in a window and asserts on something the component itself
//! produced, so an empty render fails: a content-sized box (zero height when nothing is drawn), a
//! child the component must lay out (the dock's panel), or painted quads (the chart's bars). No
//! assertion may rest on a size the test imposed itself.

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div, px,
};
use oxikube_ui::{
    ActiveTokens as _, Icon, IconName,
    button::{Button, ButtonVariants as _},
    chart::{Chart, Point, Series, Sparkline, TimeAxis},
    dialog::{Dialog, OverlayExt as _},
    dock::{DockLayout, DockSkin, PanelBehavior, PanelEvent, panel_handle},
    input::{Input, InputState},
    layout::v_flex,
    markdown::{MarkdownState, MarkdownView, sanitize},
    menu::PopupMenu,
    root::Root,
    sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
    tabs::{Tab, TabBar},
};

gpui::actions!(components_test, [Noop]);

/// Renders `R` inside a 480 px wide, content-height div tagged with a debug selector. A zero-height
/// box therefore means the component produced no content. Never give a component's parent an
/// explicit height and then assert on that parent: such tests pass for an empty component. Those
/// that fill their parent assert on something they produce instead (see the dock and chart tests).
struct Mount<R>(R);

impl<R: Fn(&mut Window, &mut App) -> AnyElement + 'static> Render for Mount<R> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        div()
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .id("subject")
                    .debug_selector(|| "subject".into())
                    .w(px(480.))
                    .child((self.0)(window, cx)),
            )
    }
}

/// Initialises `oxikube_ui`, runs `setup` once to create state, and mounts the closure it returns.
fn mount<S, R>(cx: &mut TestAppContext, setup: S) -> &mut VisualTestContext
where
    S: FnOnce(&mut Window, &mut App) -> R + 'static,
    R: Fn(&mut Window, &mut App) -> AnyElement + 'static,
{
    cx.update(oxikube_ui::init);
    let (_view, cx) = cx.add_window_view(move |window, cx| Mount(setup(window, cx)));
    cx.run_until_parked();
    cx
}

fn assert_laid_out(cx: &mut VisualTestContext, what: &str) {
    let bounds: Bounds<Pixels> = cx
        .debug_bounds("subject")
        .unwrap_or_else(|| panic!("{what}: subject was not laid out"));
    assert!(
        bounds.size.width > px(0.) && bounds.size.height > px(0.),
        "{what}: empty box"
    );
}

#[gpui::test]
fn button_and_icon_render(cx: &mut TestAppContext) {
    let cx = mount(cx, |_, _| {
        |_, _| {
            v_flex()
                .child(Button::new("go").primary().label("Apply"))
                .child(Icon::new(IconName::Box).size(px(16.)))
                .into_any_element()
        }
    });
    assert_laid_out(cx, "button + icon");
}

#[gpui::test]
fn input_renders(cx: &mut TestAppContext) {
    let cx = mount(cx, |window, cx| {
        let input = cx.new(|cx| InputState::new(window, cx));
        move |_, _| Input::new(&input).into_any_element()
    });
    assert_laid_out(cx, "input");
}

#[gpui::test]
fn tabs_render(cx: &mut TestAppContext) {
    let cx = mount(cx, |_, _| {
        |_, _| {
            TabBar::new("tabs")
                .selected_index(0)
                .child(Tab::new().label("Pods").icon(Icon::new(IconName::Box)))
                .child(Tab::new().label("Nodes"))
                .into_any_element()
        }
    });
    assert_laid_out(cx, "tabs");
}

/// Quads painted by a sidebar holding `items` active menu entries. An active entry paints its own
/// highlight quad, which the test cannot size, so counting them observes what the sidebar drew.
fn sidebar_quads(cx: &mut TestAppContext, items: usize) -> usize {
    let cx = mount(cx, move |_, _| {
        move |_, _| {
            let mut menu = SidebarMenu::new();
            for ix in 0..items {
                menu = menu.child(
                    SidebarMenuItem::new(format!("Item {ix}"))
                        .icon(Icon::new(IconName::Layers))
                        .active(true),
                );
            }
            let sidebar = Sidebar::<SidebarMenu>::new("sidebar").child(menu);
            div().h(px(200.)).child(sidebar).into_any_element()
        }
    });
    cx.update(|window, _| window.painted_quads().len())
}

#[gpui::test]
fn sidebar_renders(cx: &mut TestAppContext) {
    // The sidebar fills a parent the test sizes, so its box proves nothing; its menu entries do.
    let empty = sidebar_quads(cx, 0);
    let four = sidebar_quads(cx, 4);
    assert!(
        four >= empty + 4,
        "4 active entries painted {four} quads, an empty sidebar {empty}: entries were not drawn"
    );
}

/// Quads painted by a bar chart of `points` bars (plus whatever the window background paints).
fn bar_chart_quads(cx: &mut TestAppContext, points: usize) -> usize {
    let cx = mount(cx, move |window, cx| {
        let chart = cx.new(|_| Chart::bar("bars").legend(false));
        let series = if points == 0 {
            Vec::new()
        } else {
            let samples: Vec<Point> = (0..points)
                .map(|i| Point::new(1_000.0 + 60.0 * i as f64, 1.0 + (i * 3 % 7) as f64))
                .collect();
            vec![Series::new("bars", samples)]
        };
        let end = 1_000.0 + 60.0 * points.max(1) as f64;
        chart.update(cx, |chart, cx| {
            chart.set_series(series, TimeAxis::utc(1_000.0, end), cx);
        });
        let _ = window;
        move |_, _| div().h(px(200.)).child(chart.clone()).into_any_element()
    });
    cx.update(|window, _| window.painted_quads().len())
}

#[gpui::test]
fn chart_renders(cx: &mut TestAppContext) {
    // A chart paints into a box its parent sizes, so the box proves nothing; its bars do. Each
    // bar is a painted quad, so a chart that draws nothing leaves the quad count at its baseline.
    let baseline = bar_chart_quads(cx, 0);
    let drawn = bar_chart_quads(cx, 12);
    assert!(
        drawn >= baseline + 12,
        "12-bar chart painted {drawn} quads, empty chart {baseline}: the bars were not painted"
    );
}

#[gpui::test]
fn sparkline_renders(cx: &mut TestAppContext) {
    // A sparkline's own box is its fixed height: a zero-height box means it was not built.
    let cx = mount(cx, |_, _| {
        let points: std::sync::Arc<[Point]> = (0..30)
            .map(|i| Point::new(f64::from(i), f64::from(i % 5)))
            .collect::<Vec<_>>()
            .into();
        move |_, _| Sparkline::new("spark", points.clone()).into_any_element()
    });
    assert_laid_out(cx, "sparkline");
}

#[gpui::test]
fn markdown_renders(cx: &mut TestAppContext) {
    let cx = mount(cx, |_, _| {
        |_, _| {
            MarkdownView::markdown("md", "# Title\n\nSome **bold** text and `code`.")
                .into_any_element()
        }
    });
    assert_laid_out(cx, "markdown");
}

/// A README with every construct charts use: headings, nested lists, a table, fenced code, links,
/// an image, a badge and raw HTML. It is sanitised, parsed off the UI thread and laid out.
#[gpui::test]
fn markdown_renders_a_readme_with_html_and_images_sanitised(cx: &mut TestAppContext) {
    let readme = "<h1 align=\"center\">Chart</h1>\n\n# Title\n\n## Install\n\n- one\n  - nested\n    - deeper\n\n\
        | key | value |\n|-----|-------|\n| a | 1 |\n\n```yaml\nreplicas: 1\n```\n\n\
        [docs](https://example.com) ![logo](https://example.com/logo.png)\n\n\
        [![Build](https://ci/badge.svg)](https://ci/run)\n\n<script>alert(1)</script>\n";
    let clean = sanitize(readme);
    assert!(!clean.contains("<script") && !clean.contains("<h1") && !clean.contains("!["));
    let cx = mount(cx, move |_, cx| {
        let state = cx.new(|cx| MarkdownState::markdown(&clean, cx));
        move |_, _| MarkdownView::new(&state).into_any_element()
    });
    cx.run_until_parked();
    assert_laid_out(cx, "readme");
}

/// A 300 KB document parses off the UI thread: mounting it and parking the executor finishes.
#[gpui::test]
fn markdown_renders_a_very_large_document(cx: &mut TestAppContext) {
    let big = sanitize(
        &"## Section\n\nSome **bold** text, `code` and [a link](https://x.io).\n\n".repeat(5_000),
    );
    assert!(big.len() > 250_000);
    let cx = mount(cx, move |_, cx| {
        let state = cx.new(|cx| MarkdownState::markdown(&big, cx));
        move |_, _| MarkdownView::new(&state).into_any_element()
    });
    cx.run_until_parked();
    assert_laid_out(cx, "large markdown");
}

#[gpui::test]
fn menu_renders(cx: &mut TestAppContext) {
    let cx = mount(cx, |window, cx| {
        let menu = PopupMenu::build(window, cx, |menu, _, _| {
            menu.menu("Delete", Box::new(Noop))
                .menu("Restart", Box::new(Noop))
        });
        move |_, _| menu.clone().into_any_element()
    });
    assert_laid_out(cx, "menu");
}

/// A panel whose body is tagged, so the test can ask the dock where it put it.
struct ProbePanel(FocusHandle);

impl PanelBehavior for ProbePanel {
    fn panel_name(&self) -> &'static str {
        "ProbePanel"
    }
}
impl oxikube_ui::dock::Panel for ProbePanel {}
impl EventEmitter<PanelEvent> for ProbePanel {}

impl Focusable for ProbePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.0.clone()
    }
}

impl Render for ProbePanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("probe-panel")
            .debug_selector(|| "probe-panel".into())
            .size_full()
    }
}

#[gpui::test]
fn dock_area_renders(cx: &mut TestAppContext) {
    let cx = mount(cx, |window, cx| {
        let (area, _skin) = DockSkin::dock_area("dock", Some(1), window, cx);
        let panel = cx.new(|cx| ProbePanel(cx.focus_handle()));
        area.update(cx, |area, cx| {
            let layout = DockLayout::tabs().panel_view(panel_handle(panel), cx);
            area.set_center(layout, window, cx);
        });
        move |_, _| div().h(px(200.)).child(area.clone()).into_any_element()
    });
    // The dock, not the test, decides the panel's box: it exists only if the dock rendered it.
    let panel = cx
        .debug_bounds("probe-panel")
        .expect("the dock area did not render its panel");
    assert!(
        panel.size.width > px(100.) && panel.size.height > px(50.),
        "panel got a {:?} box inside a 480 x 200 dock",
        panel.size
    );
}

#[gpui::test]
fn dialog_opens_and_closes_through_the_root_layer(cx: &mut TestAppContext) {
    cx.update(oxikube_ui::init);
    let window = cx.add_window(|window, cx| {
        let content = cx.new(|_| Mount(|_: &mut Window, _: &mut App| div().into_any_element()));
        Root::new(content, window, cx)
    });
    let cx = &mut VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();

    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    cx.update(|window, cx| {
        window.open_dialog(cx, |dialog: Dialog, _, _| {
            dialog.title("Delete pod?").w(px(400.))
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));

    cx.update(|window, cx| window.close_dialog(cx));
    cx.run_until_parked();
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
}
