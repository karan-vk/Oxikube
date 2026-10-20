//! Saving, listing, toggling and removing favourites; what is stored.

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::portforward::{ForwardPort, ForwardSpec};
use oxikube_ports::{StatePort as _, StateTable};

use super::{Favs, spec};
use crate::portforward::favourites::{FavouriteId, TABLE};
use crate::portforward::tests::{NS, cluster};

#[tokio::test]
async fn favourites_are_saved_and_listed() {
    let favs = Favs::new();
    let saved = favs
        .favourites
        .add(spec("db-0", 5432), Some("  staging db ".into()), true)
        .await
        .unwrap();
    assert_eq!(saved.name, "staging db");
    assert!(saved.auto_start);
    let again = favs
        .favourites
        .add(spec("web-0", 80).with_local_port(8080), None, false)
        .await
        .unwrap();
    assert_eq!(
        again.name, "pod/web-0:80",
        "named after the target and port"
    );
    assert!(!again.auto_start);

    let all = favs.favourites.list().await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(
        favs.favourites.find(&spec("db-0", 5432)).await.unwrap(),
        Some(saved)
    );
    assert_eq!(favs.favourites.find(&spec("db-0", 1)).await.unwrap(), None);
}

#[tokio::test]
async fn saving_the_same_forward_again_updates_it_and_keeps_its_date() {
    let favs = Favs::new();
    let first = favs
        .favourites
        .add(spec("db-0", 5432), Some("db".into()), true)
        .await
        .unwrap();
    favs.rig.clock.advance(std::time::Duration::from_secs(60));
    let second = favs
        .favourites
        .add(spec("db-0", 5432), Some("primary db".into()), false)
        .await
        .unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(second.created_ms, first.created_ms);
    let all = favs.favourites.list().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "primary db");
    assert!(!all[0].auto_start);
}

#[tokio::test]
async fn auto_start_toggles_and_removal_is_reported() {
    let favs = Favs::new();
    let saved = favs
        .favourites
        .add(spec("db-0", 5432), None, true)
        .await
        .unwrap();
    assert!(!favs.favourites.toggle_auto_start(&saved.id).await.unwrap());
    assert!(favs.favourites.toggle_auto_start(&saved.id).await.unwrap());

    assert!(favs.favourites.remove(&saved.id).await.unwrap());
    assert!(!favs.favourites.remove(&saved.id).await.unwrap());
    let err = favs
        .favourites
        .toggle_auto_start(&saved.id)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn only_pods_and_services_can_be_saved() {
    let favs = Favs::new();
    let deployment =
        ResourceRef::namespaced(cluster(), Gvk::new("apps", "v1", "Deployment"), NS, "web");
    let err = favs
        .favourites
        .add(
            ForwardSpec::new(deployment, ForwardPort::Number(80)),
            None,
            true,
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(favs.favourites.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_ids_are_stable_and_distinguish_what_is_forwarded() {
    let a = FavouriteId::of(&spec("db-0", 5432));
    assert_eq!(a, FavouriteId::of(&spec("db-0", 5432)));
    assert_ne!(a, FavouriteId::of(&spec("db-0", 5433)));
    assert_ne!(a, FavouriteId::of(&spec("db-1", 5432)));
    assert_ne!(
        a,
        FavouriteId::of(&spec("db-0", 5432).with_local_port(1234))
    );
    // A fixed id: it is what is on disk (a change would orphan every saved favourite).
    assert!(a.as_str().starts_with("fav-") && a.as_str().len() == 20);
}

#[tokio::test]
async fn a_row_holds_only_ids_ports_names_and_flags() {
    let favs = Favs::new();
    favs.favourites
        .add(spec("db-0", 5432), Some("db".into()), true)
        .await
        .unwrap();
    let rows = favs
        .state
        .table_list(&StateTable::new(TABLE).unwrap(), None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0].1;
    let mut keys: Vec<_> = row.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["auto_start", "created_ms", "id", "name", "spec"]);
    let mut spec_keys: Vec<_> = row["spec"].as_object().unwrap().keys().cloned().collect();
    spec_keys.sort();
    assert_eq!(spec_keys, ["bind", "local_port", "remote_port", "target"]);
}

#[tokio::test]
async fn an_unreadable_row_is_skipped_not_fatal() {
    let favs = Favs::new();
    favs.favourites
        .add(spec("db-0", 5432), None, true)
        .await
        .unwrap();
    favs.state
        .table_put(
            &StateTable::new(TABLE).unwrap(),
            &oxikube_ports::StateKey::new("fav-bad").unwrap(),
            serde_json::json!({"nonsense": true}),
        )
        .await
        .unwrap();
    assert_eq!(favs.favourites.list().await.unwrap().len(), 1);
}
