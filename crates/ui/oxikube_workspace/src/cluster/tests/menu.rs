//! The context-menu model: the toggle and the presets, as commands.

use gpui::TestAppContext;
use oxikube_domain::ClusterPreset;
use oxikube_domain::command::Command;

use super::fixture;
use crate::cluster::cluster_menu_entries;
use crate::cluster::tests::fixture::{PROD, id};

#[gpui::test]
fn the_menu_offers_the_toggle_and_every_preset_as_commands(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let session = f.manager.get(&id(PROD)).unwrap();
    let rows = cluster_menu_entries(&session);
    let labels: Vec<_> = rows.iter().map(|r| r.label).collect();
    assert_eq!(
        labels,
        [
            "Read-only",
            "Production",
            "Staging",
            "Development",
            "No colour"
        ]
    );
    // Not read-only and no colour: the toggle would turn it on, "No colour" is the checked preset.
    assert!(!rows[0].checked);
    assert_eq!(
        rows[0].command,
        Command::ClusterToggleReadOnly {
            cluster: id(PROD),
            read_only: Some(true)
        }
    );
    assert_eq!(
        rows.iter().filter(|r| r.checked).count(),
        1,
        "exactly the current preset is checked"
    );
    assert!(rows[4].checked);
    for (row, preset) in rows[1..].iter().zip(ClusterPreset::ALL) {
        assert_eq!(
            row.command,
            Command::ClusterApplyPreset {
                cluster: id(PROD),
                preset
            }
        );
    }
}

#[gpui::test]
fn the_toggle_row_reflects_and_inverts_the_flag(cx: &mut TestAppContext) {
    let f = fixture(cx);
    f.manager.set_read_only(&id(PROD), true).unwrap();
    f.manager
        .set_colour(&id(PROD), ClusterPreset::Prod.colour())
        .unwrap();
    let rows = cluster_menu_entries(&f.manager.get(&id(PROD)).unwrap());
    assert!(rows[0].checked);
    assert_eq!(
        rows[0].command,
        Command::ClusterToggleReadOnly {
            cluster: id(PROD),
            read_only: Some(false)
        }
    );
    assert!(rows[1].checked, "production is the current preset");
}

#[gpui::test]
fn the_popup_menu_builds_from_the_rows(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let session = f.manager.get(&id(PROD)).unwrap();
    let ran = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = ran.clone();
    let menu = f.vcx.update(|window, cx| {
        oxikube_ui::menu::PopupMenu::build(window, cx, |menu, _, _| {
            crate::cluster::cluster_menu(
                menu,
                &session,
                std::rc::Rc::new(move |command, _, _| sink.borrow_mut().push(command)),
            )
        })
    });
    assert!(!f.vcx.update(|_, cx| menu.read(cx).is_empty()));
    assert!(ran.borrow().is_empty(), "building a menu runs nothing");
}
