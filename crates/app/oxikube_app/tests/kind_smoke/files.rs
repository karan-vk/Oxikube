//! The pod file browser's app half (E20-S07) over the real exec adapter: `FileService` and the
//! `pod::DownloadFile` / `pod::UploadFile` commands through the `CommandBus` and `MutationGuard`
//! on a kind cluster, no UI.
//!
//! * a busybox pod: `/etc` lists with its files and sizes (the busybox `ls` columns), a missing
//!   directory is `NotFound`, a file is not a directory;
//! * an image with no shell at all (`pause`) is the clear "no shell tools" error;
//! * a 5 MiB file is uploaded through the guard (audited with the container, path and size, never
//!   the content), listed with its size, downloaded and compared byte for byte; uploading it again
//!   needs the overwrite flag; a directory is downloaded and unpacked;
//! * a read-only session refuses the upload and sends nothing.
//!
//! Local files are the in-memory `FakeFsPort`; the 100 MB transfer is `oxikube_kube`'s
//! `exec_transfer` test. Every pod lives in the test's own `oxi-test-<rand>` namespace.

use std::path::Path;
use std::sync::Arc;

use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::PostParams;
use oxikube_app::MutationGuard;
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::files::{FileKind, FileRunner, FileService, NO_SHELL_TOOLS, register_commands};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_domain::ErrorKind;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::images::{BUSYBOX, PAUSE};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeFsPort, FakeStatePort};
use serde_json::json;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

fn pod_json(name: &str, image: &str, command: Option<[&str; 2]>) -> Pod {
    let mut container = json!({ "name": "main", "image": image });
    if let Some(command) = command {
        container["command"] = json!(command);
    }
    serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": name, "labels": { "oxikube.test/suite": "pod-files" } },
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": [container],
        },
    }))
    .expect("a pod")
}

async fn running(pods: &Api<Pod>, name: &str) {
    crate::eventually("the pod to run", String::new, || async {
        pods.get(name)
            .await
            .ok()
            .and_then(|p| p.status)
            .and_then(|s| s.phase)
            .is_some_and(|phase| phase == "Running")
    })
    .await;
}

struct World {
    service: Arc<FileService>,
    runner: FileRunner,
    fs: Arc<FakeFsPort>,
    state: Arc<FakeStatePort>,
    template: ResourceRef,
}

async fn world(kind: &Kind, read_only: bool) -> World {
    let admin = catalog_entry(&kind.context);
    let manager = ClusterSessionManager::with_config(
        Arc::new(KubeConnector::new(
            kind.kubeconfig.clone(),
            PoolConfig::default(),
            ConnectorConfig::default(),
        )),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
        SessionManagerConfig::default(),
    );
    manager.open(
        &admin,
        SessionOptions {
            read_only,
            ..SessionOptions::default()
        },
    );
    manager.connect(&admin.cluster).await.expect("connect");
    let fs = Arc::new(FakeFsPort::new());
    let service = Arc::new(FileService::new(manager.clone(), fs.clone()));
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, service.clone()).expect("register the file commands");
    let state = Arc::new(FakeStatePort::new());
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager, state.clone(), Arc::new(TokioClock)),
    );
    World {
        runner: FileRunner::new(bus, "tester", service.clone()),
        service,
        fs,
        state,
        template: ResourceRef::namespaced(
            admin.cluster,
            Gvk::new("", "v1", "Pod"),
            "placeholder",
            "placeholder",
        ),
    }
}

fn in_pod(template: &ResourceRef, namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(
        template.cluster.clone(),
        template.gvk.clone(),
        namespace,
        name,
    )
}

/// 5 MiB that is not all one byte.
fn payload() -> Vec<u8> {
    (0..5 * 1024 * 1024_u32).map(|n| (n % 251) as u8).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn browse_upload_download_and_the_guard_on_a_real_cluster() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    pods.create(
        &PostParams::default(),
        &pod_json("web", BUSYBOX, Some(["sleep", "3600"])),
    )
    .await
    .expect("create the pod");
    pods.create(&PostParams::default(), &pod_json("nosh", PAUSE, None))
        .await
        .expect("create the pause pod");
    running(&pods, "web").await;
    running(&pods, "nosh").await;

    let w = world(&kind, false).await;
    let web = in_pod(&w.template, ns.name(), "web");

    // --- browse: busybox's `ls` columns ---------------------------------------------------------
    let listing = w.service.list(&web, None, "/etc").await.expect("list /etc");
    let passwd = listing
        .entries
        .iter()
        .find(|e| e.name == "passwd")
        .expect("/etc/passwd");
    assert_eq!(passwd.kind, FileKind::File);
    assert!(passwd.size.is_some_and(|size| size > 0), "{passwd:?}");
    assert!(passwd.modified.is_some(), "a modification time was read");
    assert!(!passwd.mode.is_empty());
    let root = w.service.list(&web, None, "/").await.expect("list /");
    assert!(
        root.entries
            .iter()
            .any(|e| e.name == "etc" && e.kind == FileKind::Directory),
        "{:?}",
        root.entries.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    let missing = w
        .service
        .list(&web, None, "/no/such/dir")
        .await
        .unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound, "{missing}");
    let not_dir = w.service.list(&web, None, "/etc/passwd").await.unwrap_err();
    assert_eq!(not_dir.kind(), ErrorKind::Validation, "{not_dir}");

    // --- a container with no shell tools -------------------------------------------------------
    let nosh = in_pod(&w.template, ns.name(), "nosh");
    let error = w.service.list(&nosh, None, "/").await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported, "{error}");
    assert_eq!(error.message(), NO_SHELL_TOOLS);

    // --- upload through the guard, list, download, compare --------------------------------------
    let content = payload();
    w.fs.insert("/local/big.bin", content.clone());
    let plan = w
        .runner
        .plan_upload(&web, None, "/tmp", Path::new("/local/big.bin"))
        .await
        .expect("plan");
    assert!(!plan.exists);
    assert_eq!(plan.size, content.len() as u64);
    w.runner
        .upload(&web, None, "/tmp", &plan, false)
        .await
        .expect("upload");
    let listing = w.service.list(&web, None, "/tmp").await.expect("list /tmp");
    let uploaded = listing
        .entries
        .iter()
        .find(|e| e.name == "big.bin")
        .expect("the uploaded file is listed");
    assert_eq!(uploaded.size, Some(content.len() as u64));

    let download = Command::PodDownloadFile {
        target: web.clone(),
        container: None,
        path: "/tmp/big.bin".into(),
        directory: false,
        destination: "/dl/big.bin".into(),
        as_archive: false,
        size: uploaded.size,
    };
    w.runner.download(download).await.expect("download");
    assert_eq!(
        w.fs.file("/dl/big.bin"),
        Some(content.clone()),
        "byte for byte"
    );

    // The same name again needs the overwrite flag.
    let again = w
        .runner
        .plan_upload(&web, None, "/tmp", Path::new("/local/big.bin"))
        .await
        .expect("plan");
    assert!(again.exists);
    w.runner
        .upload(&web, None, "/tmp", &again, true)
        .await
        .expect("replace");

    // The audit log has the two uploads: container, path and size; never the content.
    let audit = w.state.audit_log();
    assert_eq!(audit.len(), 2, "{audit:?}");
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Succeeded));
    let first = audit[0].detail.as_deref().unwrap_or("");
    assert!(
        first.contains("path=/tmp/big.bin") && first.contains(&format!("size={}", content.len())),
        "{first}"
    );
    assert!(
        first.contains("overwrite=false")
            && audit[1]
                .detail
                .as_deref()
                .unwrap_or("")
                .contains("overwrite=true")
    );

    // --- a directory is downloaded and unpacked --------------------------------------------------
    w.runner
        .download(Command::PodDownloadFile {
            target: web.clone(),
            container: None,
            path: "/etc".into(),
            directory: true,
            destination: "/dl/tree".into(),
            as_archive: false,
            size: None,
        })
        .await
        .expect("download /etc");
    assert!(
        w.fs.file("/dl/tree/etc/passwd")
            .is_some_and(|bytes| !bytes.is_empty()),
        "/etc/passwd was unpacked under the chosen folder"
    );

    // --- the board kept the transfers; nothing in it is content ----------------------------------
    let board = w.service.board().snapshot();
    assert!(board.iter().all(|t| t.state.is_finished()));
    assert!(board.len() >= 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_only_session_refuses_the_upload_and_still_browses_and_downloads() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    pods.create(
        &PostParams::default(),
        &pod_json("web", BUSYBOX, Some(["sleep", "3600"])),
    )
    .await
    .expect("create the pod");
    running(&pods, "web").await;

    let w = world(&kind, true).await;
    let web = in_pod(&w.template, ns.name(), "web");
    w.fs.insert("/local/a.txt", b"hello".to_vec());
    let plan = w
        .runner
        .plan_upload(&web, None, "/tmp", Path::new("/local/a.txt"))
        .await
        .expect("plan");
    let error = w
        .runner
        .upload(&web, None, "/tmp", &plan, false)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Forbidden, "{error}");
    let listing = w.service.list(&web, None, "/tmp").await.expect("list /tmp");
    assert!(
        !listing.entries.iter().any(|e| e.name == "a.txt"),
        "nothing was sent"
    );
    assert_eq!(
        w.state.audit_log().last().map(|r| r.outcome),
        Some(AuditOutcome::Denied)
    );

    // Reads work on a read-only cluster.
    w.runner
        .download(Command::PodDownloadFile {
            target: web,
            container: None,
            path: "/etc/hostname".into(),
            directory: false,
            destination: "/dl/hostname".into(),
            as_archive: false,
            size: None,
        })
        .await
        .expect("download on a read-only cluster");
    assert!(w.fs.file("/dl/hostname").is_some_and(|b| !b.is_empty()));
}
