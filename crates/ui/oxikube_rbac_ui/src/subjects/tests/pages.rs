//! The Users and Groups pages.

use gpui::{AppContext as _, TestAppContext};
use oxikube_domain::command::Command;
use oxikube_domain::rbac::Subject;

use super::{Fx, NS, cluster};
use crate::subjects::Page;

#[gpui::test]
fn the_users_page_lists_users_and_service_accounts_with_counts(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.users();
    fx.vcx.update(|_, cx| {
        let page = view.read(cx);
        assert_eq!(page.page(), Page::Users);
        let names = page.list().read(cx, |d| d.shown_names());
        assert!(names.contains(&"fx-viewer".to_owned()), "{names:?}");
        assert!(
            names.contains(&format!("system:serviceaccount:{NS}:fx-sa")),
            "{names:?}"
        );
        assert!(
            !names.contains(&"fx-team".to_owned()),
            "a group is not a user"
        );
        let viewer = page.list().read(cx, |d| {
            let row = d
                .shown_names()
                .iter()
                .position(|n| n == "fx-viewer")
                .unwrap();
            d.summary(row).cloned().unwrap()
        });
        assert_eq!((viewer.bindings, viewer.roles), (1, 1));
    });
    assert!(fx.drawn("rbac-table"));
    assert!(!fx.drawn("rbac-matrix"));
}

#[gpui::test]
fn the_groups_page_lists_groups_and_says_the_api_cannot_list_members(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.groups();
    fx.vcx.update(|_, cx| {
        let names = view.read(cx).list().read(cx, |d| d.shown_names());
        assert!(names.contains(&"fx-team".to_owned()), "{names:?}");
        assert!(!names.contains(&"fx-viewer".to_owned()));
    });
    assert!(fx.drawn("rbac-group-note"));
}

#[gpui::test]
fn the_filter_narrows_the_rows_and_a_bad_pattern_says_why(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.users();
    fx.vcx.update(|_, cx| {
        view.update(cx, |view, cx| view.set_filter_text("fx-ed", cx));
    });
    fx.settle();
    fx.vcx.update(|_, cx| {
        assert_eq!(
            view.read(cx).list().read(cx, |d| d.shown_names()),
            ["fx-editor"]
        );
    });
    assert!(fx.drawn("rbac-count"));
    fx.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.set_filter_text("(", cx)));
    fx.settle();
    assert!(fx.drawn("rbac-filter-problem"));
    fx.vcx.update(|_, cx| {
        let total = view.read(cx).list().read(cx, |d| d.total());
        assert_eq!(
            view.read(cx).list().read(cx, |d| d.len()),
            total,
            "kept every row"
        );
    });
}

#[gpui::test]
fn enter_on_a_row_sends_the_show_subject_command_and_opens_its_matrix(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.users();
    fx.vcx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.set_filter_text("fx-viewer", cx);
            view.select_row(0, cx);
            view.open_selected(cx);
        });
    });
    fx.settle();
    assert!(fx.dispatcher.sent().contains(&Command::RbacShowSubject {
        cluster: cluster(),
        subject: "fx-viewer".to_owned(),
    }));
    fx.vcx.update(|_, cx| {
        let page = view.read(cx);
        assert_eq!(page.page(), Page::Subject);
        assert_eq!(page.subject(), Some(&Subject::User("fx-viewer".into())));
    });
}

#[gpui::test]
fn opening_twice_shows_the_open_page_not_a_second_one(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let first = fx.users();
    let second = fx.groups();
    assert_eq!(first.entity_id(), second.entity_id());
    let count = fx.vcx.update(|_, cx| {
        fx.workspace
            .read(cx)
            .items_of_type::<crate::subjects::SubjectsView>()
            .len()
    });
    assert_eq!(count, 1);
}

#[gpui::test]
fn while_the_feeds_list_the_page_shows_a_loading_row_with_the_count_pending(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::over(cx, oxikube_app::rbac_subjects::PolicyHandle::loading(3));
    let view = fx.users();
    fx.vcx.update(|_, cx| {
        assert_eq!(
            view.read(cx).phase(),
            &crate::subjects::Phase::Loading { pending: 3 }
        );
    });
    assert!(fx.drawn("rbac-count"));
}

#[gpui::test]
fn a_cluster_that_is_not_connected_says_so(cx: &mut TestAppContext) {
    struct Offline;
    impl crate::subjects::SubjectsBackend for Offline {
        fn cluster(&self) -> &oxikube_domain::ids::ClusterId {
            static CLUSTER: std::sync::OnceLock<oxikube_domain::ids::ClusterId> =
                std::sync::OnceLock::new();
            CLUSTER.get_or_init(cluster)
        }
        fn follow(&self) -> Option<crate::subjects::PolicyWatch> {
            None
        }
    }
    let mut fx = Fx::new(cx);
    let backend = std::rc::Rc::new(Offline);
    let dispatcher = fx.dispatcher.clone();
    let view = fx.vcx.update(|window, cx| {
        cx.new(|cx| {
            crate::subjects::SubjectsView::new(
                crate::subjects::SubjectsDeps {
                    backend,
                    dispatcher: std::rc::Rc::new(dispatcher),
                },
                None,
                window,
                cx,
            )
        })
    });
    fx.settle();
    fx.vcx.update(|_, cx| {
        assert!(matches!(
            view.read(cx).phase(),
            crate::subjects::Phase::Unavailable(_)
        ));
    });
}

#[gpui::test]
fn a_page_opened_while_offline_follows_the_policy_once_the_cluster_connects(
    cx: &mut TestAppContext,
) {
    use std::cell::Cell;
    use std::rc::Rc;

    use oxikube_app::rbac_subjects::PolicyHandle;
    use oxikube_testkit::fixtures::rbac;

    use crate::subjects::{Phase, PolicyWatch, SubjectsBackend};

    struct Flaky {
        online: Rc<Cell<bool>>,
    }
    impl SubjectsBackend for Flaky {
        fn cluster(&self) -> &oxikube_domain::ids::ClusterId {
            static CLUSTER: std::sync::OnceLock<oxikube_domain::ids::ClusterId> =
                std::sync::OnceLock::new();
            CLUSTER.get_or_init(cluster)
        }
        fn follow(&self) -> Option<PolicyWatch> {
            self.online.get().then(|| PolicyWatch {
                handle: PolicyHandle::fixed(&rbac::cluster()),
                feed: None,
                store: None,
            })
        }
    }
    let online = Rc::new(Cell::new(false));
    let mut fx = Fx::on(
        cx,
        Rc::new(Flaky {
            online: online.clone(),
        }),
    );
    let view = fx.users();
    fx.vcx.update(|_, cx| {
        assert!(matches!(view.read(cx).phase(), Phase::Unavailable(_)));
    });

    online.set(true);
    let again = fx.users();
    fx.settle();
    fx.vcx.update(|_, cx| {
        assert_eq!(again.entity_id(), view.entity_id(), "the page is reused");
        assert_eq!(view.read(cx).phase(), &Phase::Ready);
        let names = view.read(cx).list().read(cx, |d| d.shown_names());
        assert!(names.contains(&"fx-viewer".to_owned()), "{names:?}");
    });
}
