//! The user directory: listing, the override, the files that are skipped.

use super::*;
use crate::templates::TemplateOrigin;

fn names(catalog: &TemplateCatalog) -> Vec<&str> {
    catalog.templates().iter().map(|t| &*t.name).collect()
}

#[test]
fn without_a_user_directory_the_embedded_templates_are_the_catalog() {
    let fs = Arc::new(FakeFsPort::new());
    let catalog = catalog(&service(&fs));
    assert_eq!(names(&catalog), ["Namespace", "ConfigMap", "Deployment"]);
    assert!(catalog.skipped().is_empty());
    assert!(
        catalog
            .templates()
            .iter()
            .all(|t| t.origin == TemplateOrigin::Embedded)
    );
}

#[test]
fn the_embedded_templates_know_their_kind_and_documents() {
    let catalog = TemplateCatalog::of(embedded());
    let configmap = catalog
        .get("configmap")
        .expect("found without regard to case");
    assert_eq!(configmap.gvk.as_ref().map(|g| &*g.kind), Some("ConfigMap"));
    assert_eq!(configmap.documents, 1);
    assert_eq!(&*configmap.summary, "Plain settings");
}

#[test]
fn user_templates_are_named_by_a_comment_or_the_file_name() {
    let fs = Arc::new(FakeFsPort::new());
    fs.insert(
        path("quota.yaml"),
        "apiVersion: v1\nkind: ResourceQuota\nmetadata:\n  name: q\n",
    );
    fs.insert(
        path("web.yml"),
        "# name: Web stack\n# description: Deployment and Service\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n---\napiVersion: v1\nkind: Service\nmetadata:\n  name: web2\n",
    );
    fs.insert(path("notes.txt"), "not a template");
    let catalog = catalog(&service(&fs));
    assert_eq!(
        names(&catalog),
        ["Namespace", "ConfigMap", "Deployment", "quota", "Web stack"]
    );
    let web = catalog.get("Web stack").unwrap();
    assert_eq!(&*web.summary, "Deployment and Service");
    assert_eq!(web.documents, 2);
    assert_eq!(web.origin, TemplateOrigin::User);
    assert!(catalog.skipped().is_empty(), "a .txt file is not looked at");
}

#[test]
fn a_user_template_with_an_embedded_name_overrides_it_in_place() {
    let fs = Arc::new(FakeFsPort::new());
    fs.insert(
        path("mine.yaml"),
        "# name: configmap\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: mine\n  namespace: default\n",
    );
    let catalog = catalog(&service(&fs));
    assert_eq!(names(&catalog), ["Namespace", "configmap", "Deployment"]);
    let template = catalog.get("ConfigMap").unwrap();
    assert_eq!(template.origin, TemplateOrigin::User);
    assert!(template.body.contains("name: mine"));
}

#[test]
fn an_invalid_user_template_is_skipped_with_a_reason_and_the_rest_load() {
    let fs = Arc::new(FakeFsPort::new());
    fs.insert(path("broken.yaml"), "a: [unclosed\nsecret: hunter2");
    fs.insert(path("empty.yaml"), "# nothing\n");
    fs.insert(path("scalar.yaml"), "just text\n");
    fs.insert(path("binary.yaml"), vec![0xff, 0xfe, 0x00]);
    fs.insert(path("huge.yaml"), vec![b'a'; 300 * 1024]);
    fs.insert(
        path("good.yaml"),
        "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: ns\n",
    );
    let catalog = catalog(&service(&fs));
    assert_eq!(
        names(&catalog),
        ["Namespace", "ConfigMap", "Deployment", "good"]
    );
    let skipped: Vec<(&str, &str)> = catalog
        .skipped()
        .iter()
        .map(|s| (s.file.as_str(), s.reason.as_str()))
        .collect();
    assert_eq!(
        skipped,
        [
            ("binary.yaml", "is not UTF-8 text"),
            ("broken.yaml", "not valid YAML"),
            ("empty.yaml", "holds no manifest"),
            ("huge.yaml", "is too large for a template"),
            ("scalar.yaml", "holds a document that is not an object"),
        ]
    );
    // The reason never quotes the file.
    assert!(
        catalog
            .skipped()
            .iter()
            .all(|s| !s.reason.contains("hunter2"))
    );
}

#[test]
fn the_directory_is_scanned_again_on_every_call() {
    let fs = Arc::new(FakeFsPort::new());
    let service = service(&fs);
    assert_eq!(catalog(&service).templates().len(), 3);
    fs.insert(
        path("late.yaml"),
        "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: late\n",
    );
    assert_eq!(catalog(&service).templates().len(), 4);
    fs.remove(path("late.yaml"));
    assert_eq!(catalog(&service).templates().len(), 3);
}
