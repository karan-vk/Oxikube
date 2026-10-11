//! The ConfigMaps kind (E12-S11): the key/value detail with a syntax-aware preview.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::view::{PREVIEW_CAP, PreviewLanguage};
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::Timeline;
use serde_json::{Value, json};

use crate::detail::tests::fixture::Detail;
use crate::kinds::{self, ConfigMapPanel};
use crate::table::tests::fixture::cluster;

fn config(version: &str, data: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "app", "namespace": "shop", "resourceVersion": version},
        "data": data,
        "binaryData": {"logo.png": "iVBORw0K"}
    }))
    .unwrap()
}

fn target() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "ConfigMap"), "shop", "app")
}

fn sample() -> Resource {
    config(
        "3",
        json!({
            "app.yaml": "name: shop\nreplicas: 2\n",
            "db.properties": "host=db\nport=5432\n",
            "feature.json": "{\"on\": true}",
            "notes": "just words"
        }),
    )
}

fn register(d: &mut Detail) {
    d.f.vcx.update(|_, cx| kinds::init(cx));
}

fn panel(
    d: &mut Detail,
    view: &gpui::Entity<crate::detail::DetailView>,
) -> gpui::Entity<ConfigMapPanel> {
    d.read(view, |v| v.kind_panel().cloned())
        .expect("a kind panel")
        .downcast::<ConfigMapPanel>()
        .expect("the ConfigMap panel")
}

#[gpui::test]
fn a_config_map_lists_its_keys_with_language_and_size(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [sample()]);
    register(&mut d);
    let view = d.open(&target());
    d.settle();
    assert!(d.shown("configmap-panel"));
    assert!(d.shown("configmap-count"));
    for row in ["configmap-key-0", "configmap-key-3", "configmap-binary-4"] {
        assert!(d.shown(row), "{row}");
    }
    assert!(
        !d.shown("configmap-key-4"),
        "the fifth row is the binary key"
    );
    let panel = panel(&mut d, &view);
    let (keys, languages) = d.f.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        let view = panel.view().expect("made").clone();
        (
            view.entries
                .iter()
                .map(|e| e.key.to_string())
                .collect::<Vec<_>>(),
            view.entries.iter().map(|e| e.language).collect::<Vec<_>>(),
        )
    });
    assert_eq!(keys, ["app.yaml", "db.properties", "feature.json", "notes"]);
    assert_eq!(
        languages,
        [
            PreviewLanguage::Yaml,
            PreviewLanguage::Properties,
            PreviewLanguage::Json,
            PreviewLanguage::Plain
        ]
    );
}

#[gpui::test]
fn nothing_is_highlighted_until_a_key_is_expanded(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [sample()]);
    register(&mut d);
    let view = d.open(&target());
    d.settle();
    let panel = panel(&mut d, &view);
    assert!(
        d.f.vcx
            .update(|_, cx| panel.read(cx).preview_view().is_none())
    );
    assert!(!d.shown("configmap-preview"));

    d.click("configmap-key-0");
    assert!(d.shown("configmap-preview"));
    let (open, language, text) = d.f.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        let code = panel.preview_view().expect("a preview").read(cx);
        (
            panel.expanded().map(str::to_owned),
            code.language(),
            code.text().map(|t| t.to_string()),
        )
    });
    assert_eq!(open.as_deref(), Some("app.yaml"));
    assert_eq!(language, Some("yaml"));
    assert_eq!(text.as_deref(), Some("name: shop\nreplicas: 2\n"));

    // Another key switches the preview (JSON has its own grammar); the same key closes it.
    d.click("configmap-key-2");
    let language = d.f.vcx.update(|_, cx| {
        panel
            .read(cx)
            .preview_view()
            .expect("a preview")
            .read(cx)
            .language()
    });
    assert_eq!(language, Some("json"));
    d.click("configmap-key-2");
    assert!(!d.shown("configmap-preview"));
}

#[gpui::test]
fn a_big_value_is_capped_in_the_preview_and_its_row_stays_one_line(cx: &mut TestAppContext) {
    let big = format!("first line\n{}", "x".repeat(PREVIEW_CAP * 2));
    let mut d = Detail::new(cx, [config("3", json!({"big.txt": big}))]);
    register(&mut d);
    let view = d.open(&target());
    d.settle();
    let panel = panel(&mut d, &view);
    d.click("configmap-key-0");
    let (shown, summary) = d.f.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        (
            panel
                .preview_view()
                .and_then(|v| v.read(cx).text().map(|t| t.len())),
            panel.view().unwrap().entries[0].summary.clone(),
        )
    });
    assert!(shown.is_some_and(|len| len <= PREVIEW_CAP), "{shown:?}");
    assert_eq!(summary, "first line", "the row shows the first line only");
}

#[gpui::test]
fn the_keys_follow_a_new_version(cx: &mut TestAppContext) {
    let first = sample();
    let second = config("4", json!({"app.yaml": "name: shop\nreplicas: 5\n"}));
    let mut d = Detail::new(cx, [first.clone()]);
    d.f.ports().resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![first])])])
            .ok_at(
                Duration::from_secs(1),
                DeltaBatch::from_deltas(vec![Delta::Applied(second)]),
            )
            .keep_open(),
    );
    register(&mut d);
    let view = d.open(&target());
    d.settle();
    let panel = panel(&mut d, &view);
    d.click("configmap-key-0");

    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(1));
    d.settle();
    d.settle();
    let (keys, text) = d.f.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        (
            panel.view().unwrap().entries.len(),
            panel
                .preview_view()
                .and_then(|v| v.read(cx).text().map(|t| t.to_string())),
        )
    });
    assert_eq!(keys, 1);
    assert_eq!(
        text.as_deref(),
        Some("name: shop\nreplicas: 5\n"),
        "the open preview follows"
    );
}

#[gpui::test]
fn an_empty_config_map_says_so(cx: &mut TestAppContext) {
    let empty = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "app", "namespace": "shop", "resourceVersion": "1"}
    }))
    .unwrap();
    let mut d = Detail::new(cx, [empty]);
    register(&mut d);
    d.open(&target());
    d.settle();
    assert!(d.shown("configmap-empty"));
}
