//! A Secret end to end on the kind cluster (E12-S11): made in the "Create secret" dialog, edited
//! decoded, revealed, and refused read-only; the server holds base64 throughout and neither the
//! audit log nor the dry-run preview holds a value.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use gpui::TestAppContext;
use kube::api::{Patch, PatchParams};
use oxikube_app::SecretService;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use serde_json::json;

use crate::kind_common::editor::{EditorSession, secrets};
use crate::kind_common::{on_cluster, wait_value};

const NAME: &str = "app-creds";
const PASSWORD: &str = "hunter2-plaintext";
const ROTATED: &str = "rotated-plaintext";

fn secret_gvk() -> Gvk {
    Gvk::new("", "v1", "Secret")
}

fn live_data(context: &str, namespace: &str) -> serde_json::Value {
    on_cluster(context, async |client| {
        let object = secrets(client, namespace)
            .get(NAME)
            .await
            .expect("the Secret");
        serde_json::to_value(object).expect("JSON")["data"].clone()
    })
}

#[gpui::test]
fn a_secret_is_created_edited_decoded_and_revealed_with_base64_on_the_server(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let namespace = TestNamespace::create(&context).expect("a namespace");
    let ns = namespace.name().to_owned();
    let mut s = EditorSession::start(cx, &context, &ns);

    // Create: Opaque rows in the dialog, validated, then an editor tab with the values as text.
    let dialog = s.create_secret_dialog();
    let before = s.editors().len();
    dialog.update_in(&mut s.vcx, |dialog, window, cx| {
        dialog.fill_identity(NAME, &ns, window, cx);
        dialog.fill_row(0, "password", PASSWORD, window, cx);
        dialog.fill_row(1, "user", "admin", window, cx);
        dialog.submit(window, cx);
    });
    let editor = s.secret_editor(before);
    s.wait_checked(&editor, &[secret_gvk()]);
    let text = s.text(&editor);
    assert!(text.contains(&format!("password: {PASSWORD}")), "{text}");
    let problems = s.vcx.update(|_, cx| editor.read(cx).model().problems());
    assert_eq!(problems.errors, 0, "{text}");

    s.apply(&editor);
    let preview = s.single_preview("the dry run of the new Secret");
    let (title, diff) = s.vcx.update(|_, cx| {
        let dialog = preview.read(cx);
        (dialog.title().to_string(), format!("{:?}", dialog.diff()))
    });
    assert_eq!(title, format!("Create Secret {ns}/{NAME}?"));
    for leaked in [PASSWORD.to_owned(), STANDARD.encode(PASSWORD)] {
        assert!(!diff.contains(&leaked), "the preview shows a value");
    }
    s.confirm(&editor, "the dry run of the new Secret");
    let created = wait_value(&mut s.vcx, "the Secret to exist", |_| {
        on_cluster(&context, async |client| {
            secrets(client, &ns).get_opt(NAME).await.unwrap()
        })
    });
    assert_eq!(created.data["data"]["password"], STANDARD.encode(PASSWORD));
    assert_eq!(created.data["type"], "Opaque");

    // A binary value set by another client, to see it carried through untouched.
    on_cluster(&context, async |client| {
        secrets(client, &ns)
            .patch(
                NAME,
                &PatchParams::apply("oxi-test-setup"),
                &Patch::Apply(&json!({
                    "apiVersion": "v1", "kind": "Secret",
                    "metadata": {"name": NAME},
                    "data": {"blob": "//4="}
                })),
            )
            .await
            .expect("add a binary key");
    });

    // Edit decoded: plaintext in the buffer, one value changed, base64 on the server.
    let target = ResourceRef::namespaced(s.cluster.clone(), secret_gvk(), &*ns, NAME);
    let editor = s.edit_decoded(target.clone());
    s.wait_checked(&editor, &[secret_gvk()]);
    let text = s.text(&editor);
    assert!(text.contains(&format!("password: {PASSWORD}")), "{text}");
    assert!(text.contains("blob: (binary, 2 bytes)"), "{text}");
    assert!(!text.contains(&STANDARD.encode(PASSWORD)), "{text}");
    s.set_text(&editor, &text.replace(PASSWORD, ROTATED));
    s.wait_checked(&editor, &[secret_gvk()]);

    // Read-only: refused before any request; the live value is untouched.
    s.set_read_only(true);
    s.apply(&editor);
    let notice = s.settled_notice(&editor, "the read-only refusal");
    assert!(notice.text.contains("read-only"), "{notice:?}");
    assert_eq!(
        live_data(&context, &ns)["password"],
        STANDARD.encode(PASSWORD),
        "nothing was sent"
    );
    s.set_read_only(false);

    s.apply(&editor);
    s.confirm(&editor, "the dry run of the edit");
    wait_value(&mut s.vcx, "the rotated value on the server", |_| {
        (live_data(&context, &ns)["password"] == STANDARD.encode(ROTATED)).then_some(())
    });
    let data = live_data(&context, &ns);
    assert_eq!(data["blob"], "//4=", "the binary value is carried through");
    assert_eq!(data["user"], STANDARD.encode("admin"));

    // Reveal: the live values read once and decoded in memory.
    let reader = s
        .state()
        .services()
        .sessions
        .get(&s.cluster)
        .and_then(|session| session.resources())
        .expect("a connected reader");
    let decoded = s.on_kube("the revealed values", async move {
        SecretService::new().read_decoded(&*reader, &target).await
    });
    let decoded = decoded.expect("the Secret is readable");
    assert_eq!(decoded.text("password"), Some(ROTATED));
    assert_eq!(decoded.text("user"), Some("admin"));
    assert!(decoded.get("blob").is_some_and(|v| v.text().is_none()));

    // The audit log names the object and the manager, and holds no value.
    let records = s.apply_audit(NAME, 4);
    assert!(
        records.iter().any(|r| r.outcome == AuditOutcome::Denied),
        "the read-only refusal is audited"
    );
    for record in &records {
        let text = format!("{record:?}");
        for leaked in [
            PASSWORD.to_owned(),
            ROTATED.to_owned(),
            STANDARD.encode(PASSWORD),
            STANDARD.encode(ROTATED),
        ] {
            assert!(!text.contains(&leaked), "a value is in the audit: {text}");
        }
    }
}
