//! CRD browsing against a real cluster (E07-S07): a sample CRD with `additionalPrinterColumns`
//! and two served versions, two custom resources, and the user's path through the app (the real
//! init order with the kube adapter, its main window headless):
//!
//! * the sidebar's Custom Resources section lists the CRD's API group with how many kinds it has,
//!   and expanding it starts no feed;
//! * the "Definitions" entry opens the CRD list (group, version, scope, short names) and Enter on
//!   the sample CRD's row opens the table of its custom resources, on the server's Table feed:
//!   the columns are the printer columns (`Size`, `Replicas`, `Owner` as a wide one) with the
//!   cells the objects have, and the version switcher offers both served versions;
//! * the generic detail opens a custom resource unchanged (the full object, read once), and the
//!   CRD's detail shows its schema (type, description, required, enum) as a tree.
//!
//! `cargo test -p oxikube --features integration --test kind_crds` with `OXIKUBE_TEST_CONTEXT`
//! set (`cargo xtask kind-up`); without it the test returns at once. It creates a CRD with a
//! random group (`crd-<rand>.test.oxikube.dev`) and a random namespace, and removes both at the
//! end, also when the test fails.
//!
//! Real I/O wakes GPUI tasks from Tokio and SQLite threads, so the test allows parking and polls
//! with short real-time sleeps; everything else is the deterministic test scheduler.
#![cfg(feature = "integration")]

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube::app_state::AppState;
use oxikube::startup::{
    ConfigSource, PortsChoice, RuntimeChoice, StartupEnv, StartupReport, init, window,
};
use oxikube_app::ColumnId;
use oxikube_app::store::FeedState;
use oxikube_catalog_ui::CatalogView;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command as OxiCommand;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::session::SessionPhase;
use oxikube_resources_ui::crds::crd_gvk;
use oxikube_resources_ui::detail::{DetailDrawer, DetailState, DetailTab};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::integration::{
    TestNamespace, ensure_kind_context, random_namespace_name, test_context,
};
use oxikube_workspace::sidebar::{Row, SidebarPanel};
use oxikube_workspace::{ClusterTab, Workspace};

/// Far longer than anything here takes against kind; a hang fails here.
const DEADLINE: Duration = Duration::from_secs(60);

/// A CRD created by the test and deleted (without waiting) when dropped, also on a panic.
struct SampleCrd {
    context: String,
    name: String,
    group: String,
}

impl SampleCrd {
    /// `gadgets.crd-<rand>.test.oxikube.dev`: namespaced, short name `gd`, `v1` (storage) with
    /// printer columns Size, Replicas, Owner (priority 1) and Age, and `v1beta1` (served).
    fn apply(context: &str) -> Self {
        let suffix = random_namespace_name().replace("oxi-test-", "");
        let group = format!("crd-{suffix}.test.oxikube.dev");
        let name = format!("gadgets.{group}");
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "spec": {
                    "type": "object",
                    "description": "What the gadget should be.",
                    "required": ["size"],
                    "properties": {
                        "size": {
                            "type": "string",
                            "description": "How big the gadget is.",
                            "enum": ["small", "large"]
                        },
                        "replicas": {"type": "integer"},
                        "owner": {"type": "string"}
                    }
                }
            }
        });
        let crd = serde_json::json!({
            "apiVersion": "apiextensions.k8s.io/v1",
            "kind": "CustomResourceDefinition",
            "metadata": {"name": name},
            "spec": {
                "group": group,
                "scope": "Namespaced",
                "names": {
                    "plural": "gadgets", "singular": "gadget", "kind": "Gadget",
                    "listKind": "GadgetList", "shortNames": ["gd"]
                },
                "versions": [
                    {
                        "name": "v1beta1", "served": true, "storage": false,
                        "schema": {"openAPIV3Schema": schema}
                    },
                    {
                        "name": "v1", "served": true, "storage": true,
                        "schema": {"openAPIV3Schema": schema},
                        "additionalPrinterColumns": [
                            {"name": "Size", "type": "string", "jsonPath": ".spec.size"},
                            {"name": "Replicas", "type": "integer", "jsonPath": ".spec.replicas"},
                            {"name": "Owner", "type": "string", "jsonPath": ".spec.owner",
                             "priority": 1},
                            {"name": "Age", "type": "date",
                             "jsonPath": ".metadata.creationTimestamp"}
                        ]
                    }
                ]
            }
        });
        // Registered before the create, so a failed or interrupted create still cleans up.
        let guard = Self {
            context: context.to_owned(),
            name: name.clone(),
            group,
        };
        kubectl_apply(context, &crd);
        let waited = Command::new("kubectl")
            .args(["--context", context, "wait", "--for=condition=Established"])
            .arg(format!("crd/{name}"))
            .arg("--timeout=60s")
            .output()
            .expect("run kubectl wait");
        assert!(
            waited.status.success(),
            "the CRD became Established: {}",
            String::from_utf8_lossy(&waited.stderr)
        );
        guard
    }

    fn gvk(&self, version: &str) -> Gvk {
        Gvk::new(self.group.as_str(), version, "Gadget")
    }
}

impl Drop for SampleCrd {
    fn drop(&mut self) {
        let _ = Command::new("kubectl")
            .args(["--context", &self.context, "delete", "crd", &self.name])
            .args(["--ignore-not-found", "--wait=false"])
            .output();
    }
}

/// `kubectl apply -f -` of `manifest` (JSON is YAML).
fn kubectl_apply(context: &str, manifest: &serde_json::Value) {
    let mut child = Command::new("kubectl")
        .args(["--context", context, "apply", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run kubectl apply");
    child
        .stdin
        .take()
        .expect("kubectl's stdin")
        .write_all(manifest.to_string().as_bytes())
        .expect("write the manifest");
    let out = child.wait_with_output().expect("kubectl apply ends");
    assert!(
        out.status.success(),
        "kubectl apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[gpui::test]
fn a_crd_with_printer_columns_is_browsable_from_the_sidebar_to_its_schema(cx: &mut TestAppContext) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let namespace = TestNamespace::create(&context).expect("a test namespace");
    let crd = SampleCrd::apply(&context);
    for (name, size, replicas, owner) in
        [("g-1", "small", 1, "team-a"), ("g-2", "large", 7, "team-b")]
    {
        kubectl_apply(
            &context,
            &serde_json::json!({
                "apiVersion": format!("{}/v1", crd.group),
                "kind": "Gadget",
                "metadata": {"name": name, "namespace": namespace.name()},
                "spec": {"size": size, "replicas": replicas, "owner": owner}
            }),
        );
    }

    cx.executor().allow_parking();
    let dir = tempfile::tempdir().expect("a temp dir");
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    let settings = serde_json::json!({ "kubeconfig": { "sources": [{ "kind": "default" }] } });
    std::fs::write(config.join("settings.json"), settings.to_string()).unwrap();
    let env = StartupEnv {
        config: ConfigSource::Dir(config),
        runtime: RuntimeChoice::Tokio,
        ports: PortsChoice::Sqlite(dir.path().join("state.db")),
        data_dir: None,
        log: None,
        earlier: StartupReport::default(),
    };
    cx.update(|cx| init(cx, env)).expect("the init order runs");
    let handle = cx
        .update(|cx| window::open_main_window(cx, |content, _| content))
        .expect("the main window opens");
    let mut vcx = VisualTestContext::from_window(handle.into(), cx);
    let workspace = workspace(&mut vcx);
    let catalog = vcx.update(|_, cx| workspace.read(cx).items_of_type::<CatalogView>()[0].clone());
    wait(&mut vcx, "the catalog lists the kind context", |vcx| {
        vcx.update(|_, cx| {
            let model = catalog.read(cx).model();
            (0..model.visible_len())
                .filter_map(|ix| model.row(ix))
                .any(|row| row.entry().context.context.to_string() == context)
        })
    });
    vcx.update(|window, cx| {
        catalog.update(cx, |view, cx| {
            view.set_search(&context, window, cx);
            view.focus_search(window, cx);
        });
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    let cluster = wait_ready(&mut vcx, &context);
    let tab = tab_of(&mut vcx, &workspace, &cluster);
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let panel = vcx
        .update(|_, cx| inner.read(cx).panel::<SidebarPanel>())
        .expect("the cluster tab has its sidebar");

    // 1. The Custom Resources section lists the group, collapsed, with how many kinds it has.
    let group_row = format!("crd:{}", crd.group);
    wait(&mut vcx, "the sidebar to list the CRD's group", |vcx| {
        vcx.update(|_, cx| panel.read(cx).row(&group_row).is_some())
    });
    let kinds = vcx.update(|_, cx| match panel.read(cx).row(&group_row) {
        Some(Row::Group(group)) => group.count,
        _ => None,
    });
    assert_eq!(kinds, Some(1), "Gadget is the group's one kind");
    assert!(
        !vcx.update(|_, cx| panel.read(cx).is_open(&group_row)),
        "collapsed by default"
    );
    // Expanding it starts no feed for the kind: the badge reads feeds that are open.
    vcx.update(|_, cx| panel.update(cx, |panel, cx| panel.toggle(&group_row, cx)));
    vcx.run_until_parked();
    let entry = format!("{group_row}/gadgets");
    assert!(
        vcx.update(|_, cx| panel.read(cx).row(&entry).is_some()),
        "the kind is listed"
    );
    let feeds = gadget_feeds(&mut vcx, &cluster, &crd.group);
    assert_eq!(
        feeds, 0,
        "expanding the sidebar opened no feed of the group"
    );

    // 2. "Definitions" opens the CRD list; the sample CRD is a row with group, scope, short names.
    wait(&mut vcx, "the Definitions entry", |vcx| {
        vcx.update(|_, cx| panel.read(cx).row("custom-resources/definitions").is_some())
    });
    vcx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.activate("custom-resources/definitions", cx)
        })
    });
    let crd_table = wait_table(&mut vcx, &inner, "CustomResourceDefinition");
    wait(&mut vcx, "the CRD list to hold the sample CRD", |vcx| {
        vcx.update(|_, cx| {
            crd_table.read(cx).read_rows(cx, |d| {
                d.state() == &FeedState::Ready && d.rows().iter().any(|r| r.name() == crd.name)
            })
        })
    });
    let (row, group, scope, short_names, version) = vcx.update(|_, cx| {
        crd_table.read(cx).read_rows(cx, |d| {
            let row = d.rows().iter().position(|r| r.name() == crd.name).unwrap();
            let object = d.row(row).unwrap();
            let cell = |id: &str| {
                d.provider()
                    .cell(object, &ColumnId::new(id), jiff::Timestamp::now())
                    .display()
                    .to_owned()
            };
            (
                row,
                cell("group"),
                cell("scope"),
                cell("short-names"),
                cell("version"),
            )
        })
    });
    assert_eq!(group, crd.group);
    assert_eq!(scope, "Namespaced");
    assert_eq!(short_names, "gd");
    assert_eq!(version, "v1", "the storage version");

    // 3. Opening the row opens the custom resources: the Table feed's printer columns and cells.
    vcx.update(|_, cx| crd_table.update(cx, |table, cx| table.open_row(row, cx)));
    let widgets = wait_table(&mut vcx, &inner, "Gadget");
    wait(&mut vcx, "the Gadget table to list both objects", |vcx| {
        vcx.update(|_, cx| {
            widgets.read(cx).read_rows(cx, |d| {
                d.state() == &FeedState::Ready
                    && ["g-1", "g-2"]
                        .iter()
                        .all(|name| d.rows().iter().any(|r| r.name() == *name))
            })
        })
    });
    let (version, basic, switcher, visible, wide, sizes) = vcx.update(|_, cx| {
        let table = widgets.read(cx);
        let (visible, wide, sizes) = table.read_rows(cx, |d| {
            let visible: Vec<String> = d
                .layout()
                .visible_ids()
                .iter()
                .map(ToString::to_string)
                .collect();
            let wide: Vec<String> = d
                .layout()
                .columns()
                .filter(|(_, shown)| !shown)
                .map(|(c, _)| c.id.to_string())
                .collect();
            let cell = |name: &str, column: &str| {
                let object = d.rows().iter().find(|r| r.name() == name).unwrap();
                d.provider()
                    .cell(object, &ColumnId::new(column), jiff::Timestamp::now())
                    .display()
                    .to_owned()
            };
            let sizes = vec![
                cell("g-1", "size"),
                cell("g-2", "size"),
                cell("g-2", "replicas"),
                cell("g-2", "owner"),
            ];
            (visible, wide, sizes)
        });
        (
            table.gvk().version.to_string(),
            table.basic_columns(),
            table.has_version_switcher(),
            visible,
            wide,
            sizes,
        )
    });
    assert_eq!(version, "v1");
    assert!(!basic, "the server answered with its Table");
    assert!(switcher, "v1 and v1beta1 are both served");
    for column in ["name", "size", "replicas", "age"] {
        assert!(
            visible.iter().any(|c| c == column),
            "{column} is shown: {visible:?}"
        );
    }
    assert!(
        wide.iter().any(|c| c == "owner"),
        "Owner (priority 1) is a wide column: {wide:?}"
    );
    assert_eq!(
        sizes,
        ["small", "large", "7", "team-b"],
        "the printer columns' cells"
    );
    assert_eq!(
        gadget_feeds(&mut vcx, &cluster, &crd.group),
        1,
        "one Table feed, for the table that is open"
    );
    let gvk = vcx.update(|_, cx| widgets.read(cx).gvk().clone());
    assert_eq!(gvk, crd.gvk("v1"));

    // 4. The generic detail works on a custom resource, unchanged: it shows the object (read in
    //    full, since a Table row carries no `spec`), with the status chip from the Age column.
    let bus = vcx
        .update(|_, cx| AppState::global(cx).command_bus().cloned())
        .expect("the mount set the bus");
    let gadget = ResourceRef::namespaced(cluster.clone(), crd.gvk("v1"), namespace.name(), "g-2");
    let outcome = futures::executor::block_on(bus.dispatch(
        OxiCommand::ResourceOpen { target: gadget },
        oxikube_app::command_bus::DispatchContext::new(Initiator::Ui, "kind test"),
    ));
    assert!(outcome.is_ok(), "{outcome:?}");
    wait(
        &mut vcx,
        "the custom resource's detail to be live and complete",
        |vcx| {
            vcx.update(|_, cx| {
                let view = inner
                    .read(cx)
                    .panel::<DetailDrawer>()
                    .and_then(|drawer| drawer.read(cx).view().cloned());
                view.is_some_and(|view| {
                    let view = view.read(cx);
                    view.state() == &DetailState::Live
                        && view.model().is_some_and(|model| {
                            &*model.header.kind == "Gadget"
                                && &*model.header.name == "g-2"
                                && model.complete
                        })
                })
            })
        },
    );

    // 5. The CRD's detail: its schema as a tree.
    let target = ResourceRef::cluster_scoped(cluster.clone(), crd_gvk(), crd.name.as_str());
    let outcome = futures::executor::block_on(bus.dispatch(
        OxiCommand::ResourceOpen { target },
        oxikube_app::command_bus::DispatchContext::new(Initiator::Ui, "kind test"),
    ));
    assert!(outcome.is_ok(), "{outcome:?}");
    // The drawer opens on Overview, where the CRD is not decoded; the schema is read when the
    // Schema tab is opened.
    wait(&mut vcx, "the CRD's detail to be live", |vcx| {
        vcx.update(|_, cx| {
            let view = inner
                .read(cx)
                .panel::<DetailDrawer>()
                .and_then(|drawer| drawer.read(cx).view().cloned());
            view.is_some_and(|view| {
                let view = view.read(cx);
                view.state() == &DetailState::Live && view.model().is_some()
            })
        })
    });
    let view = vcx.update(|_, cx| {
        inner
            .read(cx)
            .panel::<DetailDrawer>()
            .and_then(|drawer| drawer.read(cx).view().cloned())
            .expect("the drawer")
    });
    vcx.update(|_, cx| view.update(cx, |view, cx| view.set_tab(DetailTab::Schema, cx)));
    wait(&mut vcx, "the CRD's schema to be read", |vcx| {
        vcx.update(|_, cx| view.read(cx).crd_info().is_some())
    });
    vcx.update(|_, cx| {
        view.update(cx, |view, cx| {
            assert!(view.toggle_schema("spec", cx), "spec has fields to open");
        })
    });
    let (versions, shown, rows) = vcx.update(|_, cx| {
        let view = view.read(cx);
        (
            view.schema_versions().to_vec(),
            view.schema_version().map(str::to_owned),
            view.schema_rows().to_vec(),
        )
    });
    assert_eq!(versions, ["v1beta1", "v1"]);
    assert_eq!(shown.as_deref(), Some("v1"), "the storage version's schema");
    let size = rows
        .iter()
        .find(|r| &*r.key == "spec.size")
        .expect("spec.size is a row");
    assert!(size.required);
    assert_eq!(size.ty, "string");
    assert_eq!(size.enum_values, ["small", "large"]);
    assert_eq!(size.description.as_deref(), Some("How big the gadget is."));
    assert!(
        rows.iter()
            .any(|r| &*r.key == "spec.replicas" && r.ty == "integer")
    );

    // Disconnect so the liveness loop stops before the runtime goes.
    let state = vcx.update(|_, cx| AppState::global(cx));
    state.services().sessions.disconnect(&cluster).ok();
    vcx.run_until_parked();
}

/// How many feeds the cluster's store holds for kinds of `group`.
fn gadget_feeds(vcx: &mut VisualTestContext, cluster: &ClusterId, group: &str) -> usize {
    vcx.update(|_, cx| {
        let state = AppState::global(cx);
        let session = state.services().sessions.get(cluster).expect("a session");
        let stores = state.resource_stores().expect("the app's stores");
        stores
            .for_session(&session)
            .map(|store| {
                store
                    .feeds()
                    .iter()
                    .filter(|feed| &*feed.key.gvk.group == group)
                    .count()
            })
            .unwrap_or(0)
    })
}

/// Waits for the table of `kind` to open in the cluster tab.
fn wait_table(
    vcx: &mut VisualTestContext,
    inner: &Entity<Workspace>,
    kind: &str,
) -> Entity<ResourceTable> {
    let mut found = None;
    wait(vcx, &format!("the {kind} table to open"), |vcx| {
        found = vcx.update(|_, cx| {
            inner
                .read(cx)
                .items_of_type::<ResourceTable>()
                .into_iter()
                .find(|table| &*table.read(cx).gvk().kind == kind)
        });
        found.is_some()
    });
    found.expect("found")
}

/// Polls until `done`, running the app in between; panics after [`DEADLINE`].
fn wait(
    vcx: &mut VisualTestContext,
    what: &str,
    mut done: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let started = Instant::now();
    loop {
        vcx.run_until_parked();
        if done(vcx) {
            return;
        }
        assert!(started.elapsed() < DEADLINE, "timed out waiting: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Waits until the session of `context` is `Ready`; returns its cluster id.
fn wait_ready(vcx: &mut VisualTestContext, context: &str) -> ClusterId {
    let context = ContextName::new(context);
    let mut found = None;
    wait(vcx, &format!("{context} to connect"), |vcx| {
        let state = vcx.update(|_, cx| AppState::global(cx));
        let session = state
            .services()
            .sessions
            .sessions()
            .into_iter()
            .find(|s| s.context() == &context);
        match session {
            Some(s) if s.phase() == SessionPhase::Ready => {
                found = Some(s.id().clone());
                true
            }
            _ => false,
        }
    });
    found.expect("found")
}

fn tab_of(
    vcx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    cluster: &ClusterId,
) -> Entity<ClusterTab> {
    vcx.update(|_, cx| {
        workspace
            .read(cx)
            .items_of_type::<ClusterTab>()
            .into_iter()
            .find(|tab| tab.read(cx).cluster() == cluster)
            .expect("the cluster has a tab")
    })
}

fn workspace(vcx: &mut VisualTestContext) -> Entity<Workspace> {
    vcx.update(|window, cx| {
        let main = window::main_view(window, cx).expect("the app's main view");
        main.read(cx).workspace().clone()
    })
}
