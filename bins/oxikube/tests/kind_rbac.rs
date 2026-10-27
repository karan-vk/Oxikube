//! The RBAC epic in the running app on a real cluster (E17-S10): the app on its real init order,
//! the kind admin connected, and the restricted ServiceAccount of
//! `oxikube_testkit::fixtures::rbac::RbacScenario` (it may read pods in its `home` namespace and
//! nothing else of its own) taken through what a user does:
//!
//! 1. "Generate Kubeconfig" on the account (`serviceaccount::GenerateKubeconfig`, confirmed, on
//!    the app's bus: the server address and CA come from the app's own kubeconfig source);
//! 2. the result added as a pasted kubeconfig source (`kubeconfig::AddSource`, stored owner-only
//!    in the app's config dir, the test's temp dir) and its context connected from the catalog;
//! 3. that tab's sidebar shows Workloads > Pods and hides the sections the account may not list
//!    (Config, Nodes), with the "Limited access" line (epic E17 "done when" 1);
//! 4. a RoleBinding built with the binding dialog's model (`BindingDraft`: the built-in
//!    aggregated ClusterRole `view` for the account, in `other`) and saved as the dialog saves it
//!    (`binding::Create`, confirmed) is a valid object the API server accepts (3), and the
//!    account's sidebar on `other` then shows Config > ConfigMaps but still not Secrets.
//!
//! The service-level halves (the matrix against `kubectl auth can-i --list`, the namespace
//! fallback, the kubeconfig with `kubectl`) are `oxikube_app`'s `kind_smoke::rbac`.
//!
//! Both namespaces go when the test ends, also when it fails; nothing cluster-scoped is made. The
//! token stays in the app's memory and in the pasted file under the temp dir.
//!
//! `cargo test -p oxikube --features integration --test kind_rbac` with `OXIKUBE_TEST_CONTEXT`
//! set (`cargo xtask kind-up`); without it the test returns at once.
#![cfg(feature = "integration")]

#[path = "kind_common/mod.rs"]
mod kind_common;

use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube::app_state::AppState;
use oxikube_app::command_bus::{CommandBus, DispatchContext, Outcome};
use oxikube_app::sa_kubeconfig::KubeconfigVault;
use oxikube_app::{Confirmation, DispatchError};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, NewKubeconfigSource, PastedText};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::rbac::{BindingKind, RoleKind, RoleRef};
use oxikube_rbac_ui::binding::{BindingDraft, SubjectDraft, SubjectKind};
use oxikube_testkit::fixtures::rbac::{ACCOUNT, VIEW_BINDING};
use oxikube_testkit::integration::rbac::AppliedScenario;
use oxikube_testkit::integration::{ensure_kind_context, test_context};
use oxikube_workspace::ClusterTab;
use oxikube_workspace::sidebar::{Row, SidebarPanel};

use kind_common::{Launched, connect, launch, wait, wait_value};

/// Sends `command` on the app's Kubernetes runtime, where its handler's requests run, and waits
/// for the outcome while the app runs.
fn dispatch(
    app: &mut Launched,
    bus: &CommandBus,
    command: Command,
    context: DispatchContext,
) -> Result<Outcome, DispatchError> {
    let bus = bus.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let task = app.vcx.update(|_, cx| {
        oxikube_runtime::spawn_kube(cx, async move {
            let _ = tx.send(bus.dispatch(command, context).await);
        })
    });
    let outcome = wait_value(&mut app.vcx, "the command to finish", |_| {
        rx.try_recv().ok()
    });
    drop(task);
    outcome
}

/// Dispatches `command` as the UI does, then confirms what the guard asks.
fn run_confirmed(app: &mut Launched, bus: &CommandBus, command: Command) {
    let ui = || DispatchContext::new(Initiator::Ui, "kind-rbac-test");
    let asked = dispatch(app, bus, command.clone(), ui()).expect("dispatches");
    let Outcome::NeedsConfirmation(request) = asked else {
        panic!("expected a confirmation request, got {asked:?}");
    };
    let done = dispatch(
        app,
        bus,
        command,
        ui().with_confirmation(Confirmation::simple(request.token)),
    )
    .expect("the confirmed command runs");
    assert!(matches!(done, Outcome::Completed(_)), "{done:?}");
}

/// The cluster tab's sidebar.
fn sidebar(vcx: &mut VisualTestContext, tab: &Entity<ClusterTab>) -> Entity<SidebarPanel> {
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    vcx.update(|_, cx| inner.read(cx).panel::<SidebarPanel>())
        .expect("the cluster tab has a sidebar")
}

fn has_row(vcx: &mut VisualTestContext, panel: &Entity<SidebarPanel>, id: &str) -> bool {
    vcx.update(|_, cx| panel.read(cx).row(id).is_some())
}

/// The "Limited access: ... hidden" line, if the sidebar shows one.
fn limited_access(vcx: &mut VisualTestContext, panel: &Entity<SidebarPanel>) -> Option<String> {
    vcx.update(|_, cx| {
        panel.read(cx).rows().iter().find_map(|row| match row {
            Row::Notice(notice) if notice.id == "access-limited" => Some(notice.text.to_string()),
            _ => None,
        })
    })
}

/// The binding the dialog builds for "ClusterRole `view`, the account, in `other`".
fn draft(home: &str, other: &str) -> BindingDraft {
    let mut draft = BindingDraft::new(BindingKind::RoleBinding, other);
    draft.name = VIEW_BINDING.to_owned();
    draft.role = Some(RoleRef {
        kind: RoleKind::ClusterRole,
        name: "view".into(),
    });
    draft.subjects = vec![SubjectDraft {
        kind: SubjectKind::ServiceAccount,
        name: ACCOUNT.to_owned(),
        namespace: home.to_owned(),
    }];
    draft
}

#[gpui::test]
fn a_restricted_account_sees_less_and_a_binding_from_the_dialog_grants_more(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let applied = AppliedScenario::apply(&context).expect("the RBAC scenario");
    let scenario = applied.scenario().clone();

    cx.executor().allow_parking();
    let mut app = launch(cx, &context);
    let state = app.vcx.read(AppState::global);
    let bus = state.command_bus().expect("the bus").clone();
    let admin: ClusterId = app.cluster.clone();

    // --- 1. "Generate Kubeconfig" on the account, as the admin -------------------------------
    let account = ResourceRef::namespaced(
        admin.clone(),
        Gvk::new("", "v1", "ServiceAccount"),
        scenario.home.as_str(),
        ACCOUNT,
    );
    run_confirmed(
        &mut app,
        &bus,
        Command::ServiceAccountGenerateKubeconfig {
            target: account.clone(),
            expiration_seconds: None,
            audiences: Vec::new(),
            allow_legacy_token: false,
        },
    );
    let generated = KubeconfigVault::shared()
        .get(&account)
        .expect("the kubeconfig is in the account's panel");
    let restricted_context = generated
        .text
        .expose()
        .lines()
        .find_map(|line| line.strip_prefix("current-context: "))
        .expect("the kubeconfig names its context")
        .trim_matches('"')
        .to_owned();

    // --- 2. added as a pasted source and connected from the catalog --------------------------
    let added = dispatch(
        &mut app,
        &bus,
        Command::KubeconfigAddSource {
            source: NewKubeconfigSource::Pasted {
                name: format!("{}-{ACCOUNT}", scenario.home),
                text: PastedText::new(generated.text.expose()),
            },
        },
        DispatchContext::new(Initiator::Ui, "kind-rbac-test"),
    );
    assert!(matches!(added, Ok(Outcome::Completed(_))), "{added:?}");
    let (restricted, tab) = connect(&mut app.vcx, &restricted_context);
    assert_ne!(restricted, admin, "a session of its own");
    let panel = sidebar(&mut app.vcx, &tab);

    // --- 3. the sidebar: Pods shown, Config and Nodes hidden, the limited-access line --------
    wait(
        &mut app.vcx,
        "the account's sidebar to show Pods and say what it hides",
        |vcx| has_row(vcx, &panel, "workloads/pods") && limited_access(vcx, &panel).is_some(),
    );
    let sections = app.vcx.update(|_, cx| panel.read(cx).visible_sections());
    for hidden in ["config", "nodes", "network", "storage"] {
        assert!(
            !sections.iter().any(|section| section == hidden),
            "{hidden} is hidden: {sections:?}"
        );
    }
    assert!(sections.iter().any(|section| section == "workloads"));
    assert!(!has_row(&mut app.vcx, &panel, "workloads/deployments"));
    let line = limited_access(&mut app.vcx, &panel).expect("the line");
    assert!(line.starts_with("Limited access:"), "{line}");

    // --- 4. a binding from the dialog's model, saved as the dialog saves it ------------------
    let draft = draft(&scenario.home, &scenario.other);
    assert!(!draft.has_errors(), "{:?}", draft.problems());
    let manifest = draft.manifest();
    let expected = scenario.view_binding();
    for field in ["kind", "roleRef", "subjects"] {
        assert_eq!(manifest[field], expected[field], "{field}");
    }
    assert_eq!(manifest["metadata"], expected["metadata"]);
    run_confirmed(
        &mut app,
        &bus,
        Command::BindingCreate {
            cluster: admin,
            manifest: manifest.to_string(),
        },
    );

    // The account, on `other`, now lists config maps there (`view`), still not secrets.
    let selected = dispatch(
        &mut app,
        &bus,
        Command::NamespaceSelect {
            cluster: restricted,
            namespaces: vec![scenario.other],
        },
        DispatchContext::new(Initiator::Ui, "kind-rbac-test"),
    );
    assert!(selected.is_ok(), "{selected:?}");
    wait(
        &mut app.vcx,
        "the account's sidebar to show ConfigMaps on `other`",
        |vcx| has_row(vcx, &panel, "config/configmaps"),
    );
    assert!(!has_row(&mut app.vcx, &panel, "config/secrets"));
    drop(applied);
}
