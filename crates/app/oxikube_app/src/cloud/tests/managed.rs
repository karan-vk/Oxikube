//! Adding and removing clusters: the managed kubeconfig, the source list, the session.

use std::path::PathBuf;

use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{CloudProvider, ClusterContext, SourceId, UserSource};
use oxikube_testkit::{FakeClusterSourcePort, FakeFsPort, FsCall};

use super::{DIR, Fixture, aws, aws_scope, eks, source_with_managed};
use crate::cloud::managed::{MARKER, ManagedKubeconfig};
use crate::session::SessionOptions;
use crate::sources::SourceListStore as _;

fn aws_yaml() -> PathBuf {
    PathBuf::from(DIR).join("aws.yaml")
}

fn writes(f: &Fixture) -> usize {
    f.fs.recorded_calls()
        .iter()
        .filter(|call| matches!(call, FsCall::WritePrivate(..)))
        .count()
}

#[tokio::test]
async fn adding_writes_an_exec_auth_entry_owner_only_and_lists_the_file() {
    let prod = aws_scope("prod", "eu-west-1");
    let web = eks(&prod, "web");
    let f = Fixture::new(vec![aws([prod.clone()], [web.clone()])]);

    let added = f.service.add(&web).await.unwrap();
    assert_eq!(added.path, aws_yaml());
    assert!(!added.unchanged);
    assert!(
        f.fs.is_private(aws_yaml()),
        "written with write_private (0600)"
    );

    let text = f.managed_text(CloudProvider::Aws).unwrap();
    assert!(text.starts_with(MARKER), "{text}");
    assert!(text.contains("command: aws"), "{text}");
    assert!(text.contains("interactiveMode: IfAvailable"), "{text}");
    assert!(
        text.contains("https://web.gr7.eu-west-1.eks.amazonaws.com"),
        "{text}"
    );
    for secret in ["token:", "client-key-data", "password"] {
        assert!(!text.contains(secret), "no {secret} in {text}");
    }
    let file = ManagedKubeconfig::parse(&text).unwrap();
    assert_eq!(file.entries().len(), 1);
    assert_eq!(file.entries()[0].entry.context, added.context);
    assert_eq!(
        file.entries()[0].cluster_key.as_deref(),
        Some(web.key().as_str())
    );

    let listed = [UserSource::default_source(), UserSource::file(aws_yaml())];
    assert_eq!(f.list.snapshot(), listed);
    assert_eq!(
        f.source.user_sources(),
        listed,
        "the cluster source reloaded with it"
    );
    assert_eq!(
        f.service.added(CloudProvider::Aws).await.unwrap(),
        file.entries()
    );
}

#[tokio::test]
async fn adding_twice_is_idempotent() {
    let prod = aws_scope("prod", "eu-west-1");
    let web = eks(&prod, "web");
    let f = Fixture::new(vec![aws([prod.clone()], [web.clone()])]);
    let first = f.service.add(&web).await.unwrap();
    let text = f.managed_text(CloudProvider::Aws).unwrap();

    let again = f.service.add(&web).await.unwrap();
    assert!(again.unchanged);
    assert_eq!(again.context, first.context);
    assert_eq!(f.managed_text(CloudProvider::Aws).unwrap(), text);
    assert_eq!(writes(&f), 1, "the file is not rewritten");
    assert_eq!(f.list.snapshot().len(), 2, "listed once");
}

#[tokio::test]
async fn clusters_merge_into_one_file_by_context_name() {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    let (web, api) = (eks(&prod, "web"), eks(&dev, "api"));
    let f = Fixture::new(vec![aws(
        [prod.clone(), dev.clone()],
        [web.clone(), api.clone()],
    )]);
    f.service.add(&web).await.unwrap();
    f.service.add(&api).await.unwrap();

    let contexts: Vec<_> = f
        .service
        .added(CloudProvider::Aws)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.entry.context.to_string())
        .collect();
    let mut sorted = contexts.clone();
    sorted.sort();
    assert_eq!(contexts.len(), 2);
    assert_eq!(contexts, sorted, "deterministic order");
    assert_eq!(f.list.snapshot().len(), 2, "one source per provider file");
}

#[tokio::test]
async fn removing_drops_the_entry_and_the_last_one_takes_the_file() {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    let (web, api) = (eks(&prod, "web"), eks(&dev, "api"));
    let f = Fixture::new(vec![aws(
        [prod.clone(), dev.clone()],
        [web.clone(), api.clone()],
    )]);
    let web_ctx = f.service.add(&web).await.unwrap().context;
    let api_ctx = f.service.add(&api).await.unwrap().context;

    let removed = f
        .service
        .remove(CloudProvider::Aws, &web_ctx)
        .await
        .unwrap();
    assert!(!removed.deleted_file && !removed.disconnected);
    let left = f.service.added(CloudProvider::Aws).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].entry.context, api_ctx);
    assert!(f.fs.is_private(aws_yaml()));

    let removed = f
        .service
        .remove(CloudProvider::Aws, &api_ctx)
        .await
        .unwrap();
    assert!(removed.deleted_file);
    assert_eq!(f.fs.file(aws_yaml()), None);
    assert_eq!(f.list.snapshot(), [UserSource::default_source()]);
    assert_eq!(f.source.user_sources(), [UserSource::default_source()]);

    let error = f
        .service
        .remove(CloudProvider::Aws, &api_ctx)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn removing_a_file_the_user_unlisted_still_deletes_it() {
    let prod = aws_scope("prod", "eu-west-1");
    let web = eks(&prod, "web");
    let f = Fixture::new(vec![aws([prod.clone()], [web.clone()])]);
    let context = f.service.add(&web).await.unwrap().context;
    f.list.save(&[UserSource::default_source()]).await.unwrap();

    let removed = f
        .service
        .remove(CloudProvider::Aws, &context)
        .await
        .unwrap();
    assert!(removed.deleted_file);
    assert_eq!(f.fs.file(aws_yaml()), None);
}

#[tokio::test]
async fn removing_disconnects_the_open_session_of_that_context() {
    let prod = aws_scope("prod", "eu-west-1");
    let web = eks(&prod, "web");
    let port = aws([prod.clone()], [web.clone()]);
    let context = ContextName::new("aws-prod-eu-west-1-eu-west-1-web");
    let source_id = SourceId(format!("file:{DIR}/aws.yaml"));
    let cluster = ClusterId::new(&format!("{DIR}/aws.yaml"), &context);
    let catalog = ClusterContext::new(cluster.clone(), context.clone(), source_id);
    let f = Fixture::with(
        vec![port],
        FakeFsPort::new(),
        source_with_managed(vec![catalog.clone()]),
    );
    assert_eq!(f.service.add(&web).await.unwrap().context, context);
    f.sessions.open(&catalog, SessionOptions::default());

    let removed = f
        .service
        .remove(CloudProvider::Aws, &context)
        .await
        .unwrap();
    assert!(removed.disconnected);
    assert!(
        f.sessions.get(&cluster).is_some(),
        "disconnected, not closed"
    );
}

#[tokio::test]
async fn a_failed_write_leaves_the_old_file_and_the_list() {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    let (web, api) = (eks(&prod, "web"), eks(&dev, "api"));
    let f = Fixture::new(vec![aws(
        [prod.clone(), dev.clone()],
        [web.clone(), api.clone()],
    )]);
    f.service.add(&web).await.unwrap();
    let before = f.managed_text(CloudProvider::Aws).unwrap();
    let listed = f.list.snapshot();

    // The process dies between the temp file and the rename: the port reports the failure and
    // the target is untouched (`FsPort::write_private` renames only a complete temp file).
    f.fs.script()
        .write_private
        .push_err(OxiError::internal("disk full"));
    let error = f.service.add(&api).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert_eq!(f.managed_text(CloudProvider::Aws).unwrap(), before);
    assert_eq!(f.list.snapshot(), listed);
}

#[tokio::test]
async fn a_file_that_is_not_ours_is_never_overwritten() {
    let prod = aws_scope("prod", "eu-west-1");
    let web = eks(&prod, "web");
    let theirs = "apiVersion: v1\nkind: Config\ncontexts: []\n";
    let f = Fixture::with(
        vec![aws([prod.clone()], [web.clone()])],
        FakeFsPort::new().with_file(aws_yaml(), theirs),
        FakeClusterSourcePort::new(),
    );

    let error = f.service.add(&web).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Conflict, "{error}");
    let error = f
        .service
        .remove(CloudProvider::Aws, &ContextName::new("x"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert_eq!(f.fs.file(aws_yaml()).unwrap(), theirs.as_bytes());
    assert_eq!(writes(&f), 0);
    assert_eq!(f.list.snapshot(), [UserSource::default_source()]);
}

#[tokio::test]
async fn a_cluster_of_a_provider_without_an_adapter_cannot_be_added() {
    let f = Fixture::new(Vec::new());
    let web = eks(&aws_scope("prod", "eu-west-1"), "web");
    let error = f.service.add(&web).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(writes(&f), 0);
}

#[test]
fn the_yaml_round_trips_every_exec_field() {
    use oxikube_ports::{ExecEnvVar, ExecInteractiveMode, ExecPlugin, KubeconfigEntry};

    use crate::cloud::ManagedEntry;

    let mut file = ManagedKubeconfig::default();
    let entry = |name: &str, mode| ManagedEntry {
        entry: KubeconfigEntry {
            context: ContextName::new(format!("gke:shop:europe-west1:{name}")),
            server: "https://203.0.113.10".into(),
            ca_data: (name == "a").then(|| "LS0t".to_owned()),
            exec: ExecPlugin {
                env: vec![ExecEnvVar {
                    name: "CLOUDSDK_CORE_PROJECT".into(),
                    value: "shop".into(),
                }],
                install_hint: Some("gcloud components install gke-gcloud-auth-plugin".into()),
                provide_cluster_info: true,
                ..ExecPlugin::new(
                    "gke-gcloud-auth-plugin",
                    vec!["--use_application_default_credentials".into()],
                    mode,
                )
            },
        },
        cluster_key: (name == "a").then(|| format!("gcp/shop/europe-west1/{name}")),
    };
    assert!(file.upsert(entry("b", ExecInteractiveMode::Never)));
    assert!(file.upsert(entry("a", ExecInteractiveMode::Always)));
    assert!(!file.upsert(entry("a", ExecInteractiveMode::Always)));
    assert!(
        file.upsert(entry("a", ExecInteractiveMode::IfAvailable)),
        "replaced in place"
    );

    let text = file.render().unwrap();
    let back = ManagedKubeconfig::parse(&text).unwrap();
    assert_eq!(back, file, "{text}");
    assert_eq!(
        back.entries()[0].entry.context.as_str(),
        "gke:shop:europe-west1:a"
    );
    assert_eq!(file.render().unwrap(), text, "rendering is deterministic");
}
