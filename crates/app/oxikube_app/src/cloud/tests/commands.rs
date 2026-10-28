//! The `cloud::*` commands over the fakes: refresh with an observer, add by key, remove,
//! cancel, and the registry.

use futures::StreamExt as _;
use oxikube_domain::ErrorKind;
use oxikube_domain::command::{CloudRefreshMode, Command, CommandId};
use oxikube_domain::ids::ContextName;
use oxikube_ports::CloudProvider;

use super::{Fixture, aws, aws_scope, eks};
use crate::cloud::{CloudDiscoveryEvent, register_commands};
use crate::command_bus::CommandRegistry;

fn refresh(provider: Option<CloudProvider>) -> Command {
    Command::CloudRefresh {
        provider,
        mode: CloudRefreshMode::Force,
    }
}

#[tokio::test]
async fn refresh_reports_every_event_to_the_observer_and_summarises() {
    let prod = aws_scope("prod", "eu-west-1");
    let f = Fixture::new(vec![aws([prod.clone()], [eks(&prod, "web")])]);
    let watched = f.service.watch();

    let output = f
        .service
        .execute(&refresh(Some(CloudProvider::Aws)))
        .await
        .unwrap();

    assert_eq!(output.message.as_deref(), Some("Found 1 cloud cluster."));
    // Detected, Started, ScopeDone, Finished.
    let events: Vec<_> = watched.take(4).collect().await;
    assert!(matches!(
        events.first().map(|e| &**e),
        Some(CloudDiscoveryEvent::Detected(_))
    ));
    assert!(matches!(
        events.last().map(|e| &**e),
        Some(CloudDiscoveryEvent::Finished { .. })
    ));
    let key = output.data.unwrap()["clusters"][0]["key"].clone();
    assert_eq!(key, eks(&prod, "web").key());
}

#[tokio::test]
async fn refresh_of_all_providers_skips_the_ones_without_an_adapter() {
    let prod = aws_scope("prod", "eu-west-1");
    let f = Fixture::new(vec![aws([prod.clone()], [eks(&prod, "web")])]);
    let output = f.service.execute(&refresh(None)).await.unwrap();
    assert_eq!(output.message.as_deref(), Some("Found 1 cloud cluster."));
}

#[tokio::test]
async fn add_resolves_the_key_from_the_last_listing_then_remove_undoes_it() {
    let prod = aws_scope("prod", "eu-west-1");
    let web = eks(&prod, "web");
    let f = Fixture::new(vec![aws([prod.clone()], [web.clone()])]);

    let unknown = Command::CloudAdd {
        provider: CloudProvider::Aws,
        cluster: web.key(),
    };
    let error = f.service.execute(&unknown).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound, "nothing was listed yet");

    f.service.execute(&refresh(None)).await.unwrap();
    let added = f.service.execute(&unknown).await.unwrap();
    assert!(added.message.unwrap().starts_with("Added "));
    let again = f.service.execute(&unknown).await.unwrap();
    assert!(again.message.unwrap().contains("already"));
    assert!(f.managed_text(CloudProvider::Aws).is_some());

    let context = f.service.added(CloudProvider::Aws).await.unwrap()[0]
        .entry
        .context
        .clone();
    let removed = f
        .service
        .execute(&Command::CloudRemove {
            provider: CloudProvider::Aws,
            context: context.clone(),
        })
        .await
        .unwrap();
    assert!(removed.message.unwrap().starts_with("Removed "));
    let missing = Command::CloudRemove {
        provider: CloudProvider::Aws,
        context: ContextName::new("nope"),
    };
    let error = f.service.execute(&missing).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn cancel_with_nothing_running_says_so_and_other_commands_are_refused() {
    let f = Fixture::new(vec![]);
    let output = f
        .service
        .execute(&Command::CloudCancel { provider: None })
        .await
        .unwrap();
    assert_eq!(
        output.message.as_deref(),
        Some("No cloud discovery was running.")
    );
    let error = f
        .service
        .execute(&Command::KubeconfigReload)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
}

#[tokio::test]
async fn the_four_commands_register_with_their_tool_stubs() {
    let f = Fixture::new(vec![]);
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, &f.service).unwrap();
    for id in [
        CommandId::CLOUD_REFRESH,
        CommandId::CLOUD_CANCEL,
        CommandId::CLOUD_ADD,
        CommandId::CLOUD_REMOVE,
    ] {
        assert!(registry.contains(id), "{id}");
    }
}
