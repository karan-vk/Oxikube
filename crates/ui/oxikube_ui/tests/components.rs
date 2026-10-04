//! Renders each curated component once with the default tokens (E05-S02 acceptance test).
//!
//! Each test mounts the component in a window and asserts it laid out with a non-empty box, which
//! fails if the wrapper is mis-wired (missing init, missing asset, panic in render).

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, Styled as _, TestAppContext, VisualTestContext, Window,
    div, px,
};
use oxikube_ui::{
    ActiveTokens as _, Icon, IconName,
    button::{Button, ButtonVariants as _},
    chart::LineChart,
    dialog::{Dialog, OverlayExt as _},
    dock::DockSkin,
    input::{Input, InputState},
    layout::v_flex,
    markdown::MarkdownView,
    menu::PopupMenu,
    root::Root,
    sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
    tabs::{Tab, TabBar},
};

gpui::actions!(components_test, [Noop]);

/// Renders `R` inside a 480 px wide, content-height div tagged with a debug selector. A zero-height
/// box therefore means the component produced no content; components that fill their parent are
/// given an explicit height by their test.
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

#[gpui::test]
fn sidebar_renders(cx: &mut TestAppContext) {
    let cx = mount(cx, |_, _| {
        |_, _| {
            let sidebar = Sidebar::<SidebarMenu>::new("sidebar").child(
                SidebarMenu::new()
                    .child(SidebarMenuItem::new("Workloads").icon(Icon::new(IconName::Layers)))
                    .child(SidebarMenuItem::new("Network").icon(Icon::new(IconName::Network))),
            );
            div().h(px(200.)).child(sidebar).into_any_element()
        }
    });
    assert_laid_out(cx, "sidebar");
}

#[gpui::test]
fn chart_renders(cx: &mut TestAppContext) {
    let cx = mount(cx, |_, _| {
        |_, _| {
            let data: Vec<(f64, f64)> = (0..20)
                .map(|i| (f64::from(i), f64::from(i * i % 7)))
                .collect();
            let chart = LineChart::new(data)
                .x(|d: &(f64, f64)| format!("{}", d.0))
                .y(|d: &(f64, f64)| d.1);
            div().h(px(200.)).child(chart).into_any_element()
        }
    });
    assert_laid_out(cx, "chart");
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

#[gpui::test]
fn dock_area_renders(cx: &mut TestAppContext) {
    let cx = mount(cx, |window, cx| {
        let (area, _skin) = DockSkin::dock_area("dock", Some(1), window, cx);
        move |_, _| div().h(px(200.)).child(area.clone()).into_any_element()
    });
    assert_laid_out(cx, "dock area");
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
