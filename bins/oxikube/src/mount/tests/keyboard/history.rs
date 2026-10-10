//! Scenario D: `[`, `]` and `-` in a table replay the lines run this session and flip between the
//! last two views.

use gpui::TestAppContext;

use super::{SYSTEM_PODS, check};
use crate::mount::tests::App;

impl App {
    fn view(&mut self) -> (String, usize) {
        let (kind, rows) = self.shown().expect("a table is shown");
        (kind, rows.len())
    }
}

#[gpui::test]
fn bracket_keys_replay_the_jumps_and_dash_flips_between_the_last_two_views(
    cx: &mut TestAppContext,
) {
    let mut app = App::keyboard(cx, false);
    app.colon("pods kube-system");
    app.colon("deploy");
    app.colon("cm");
    check!(app, app.view().0 == "ConfigMap", "the last jump is shown");

    // `[` steps back through the lines: `deploy`, then `pods kube-system`, whose namespace is
    // applied again.
    app.press("[");
    app.tick();
    check!(app, app.view().0 == "Deployment", "`[` replays `deploy`");
    app.press("[");
    app.tick();
    let shown = app.shown();
    check!(
        app,
        shown == Some(("Pod".into(), SYSTEM_PODS.map(String::from).to_vec())),
        "`[` again replays `pods kube-system`: {shown:?}"
    );

    // `]` goes forward again.
    app.press("]");
    app.tick();
    check!(app, app.view().0 == "Deployment", "`]` replays `deploy`");
    app.press("]");
    app.tick();
    check!(app, app.view().0 == "ConfigMap", "`]` replays `cm`");

    // `-` flips between the last two views.
    app.press("-");
    app.tick();
    check!(
        app,
        app.view().0 == "Deployment",
        "`-` goes back to the view before"
    );
    app.press("-");
    app.tick();
    check!(
        app,
        app.view().0 == "ConfigMap",
        "and again to where it was"
    );
    assert!(
        app.jump_bar_open().is_none(),
        "the history keys never open the bar"
    );
}
