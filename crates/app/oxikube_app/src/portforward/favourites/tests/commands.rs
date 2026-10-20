//! `portforward::Favourite*` handlers.

use oxikube_domain::ErrorKind;
use oxikube_domain::command::Command;
use oxikube_domain::portforward::ForwardPort;

use super::{Favs, pod_in, spec};
use crate::command_bus::CommandRegistry;
use crate::portforward::favourites::commands::run;
use crate::portforward::favourites::{COMMANDS, register_commands};
use crate::portforward::tests::cluster;

#[tokio::test]
async fn add_toggle_and_remove_run_through_the_handlers() {
    let favs = Favs::new();
    let out = run(
        &favs.favourites,
        Command::PortForwardFavouriteAdd {
            target: pod_in(cluster(), "db-0"),
            remote_port: ForwardPort::Number(5432),
            local_port: Some(15432),
            bind: None,
            name: Some("db".into()),
            auto_start: None,
        },
    )
    .await
    .expect("add");
    let data = out.data.unwrap();
    let id = data["id"].as_str().unwrap().to_owned();
    assert_eq!(data["auto_start"], true, "on by default");
    let saved = favs.favourites.list().await.unwrap();
    assert_eq!(saved[0].spec.local_port, 15432);

    let out = run(
        &favs.favourites,
        Command::PortForwardFavouriteToggleAutoStart { id: id.clone() },
    )
    .await
    .expect("toggle");
    assert_eq!(out.data.unwrap()["auto_start"], false);

    run(
        &favs.favourites,
        Command::PortForwardFavouriteRemove { id: id.clone() },
    )
    .await
    .expect("remove");
    let err = run(&favs.favourites, Command::PortForwardFavouriteRemove { id })
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn removing_a_favourite_leaves_a_running_forward_running() {
    let favs = Favs::new();
    favs.rig.backend.push(
        "Pod",
        vec![crate::portforward::tests::pod("db-0", "Running")],
    );
    let saved = favs
        .favourites
        .add(spec("db-0", 5432), None, true)
        .await
        .unwrap();
    favs.favourites.restore(&cluster()).await.unwrap();
    favs.favourites.remove(&saved.id).await.unwrap();
    assert_eq!(favs.rig.manager.list().len(), 1);
}

#[tokio::test]
async fn the_three_commands_register_with_their_declared_metadata() {
    let favs = Favs::new();
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, favs.favourites.clone()).expect("registers");
    for id in COMMANDS {
        assert!(registry.contains(id), "{id} has a handler");
        let meta = oxikube_domain::command::lookup(id).expect("declared");
        assert!(!meta.title.is_empty());
    }
    // Registering twice is a wiring bug, reported.
    assert!(register_commands(&mut registry, favs.favourites).is_err());
}
