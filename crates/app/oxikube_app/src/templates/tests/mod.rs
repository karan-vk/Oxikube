//! Tests of the templates: the namespace placeholder, the user directory through the fake
//! filesystem, the override and the files that are skipped.

mod prefill;
mod service;

use std::path::PathBuf;
use std::sync::Arc;

use futures::FutureExt as _;
use oxikube_testkit::FakeFsPort;

use super::{Template, TemplateCatalog, TemplateService};

pub(super) const DIR: &str = "/home/me/.config/oxikube/templates";

pub(super) const CONFIGMAP: &str = "\
apiVersion: v1
kind: ConfigMap
metadata:
  name: example
  namespace: default
data:
  key: value
";

pub(super) fn embedded() -> Vec<Template> {
    vec![
        Template::embedded(
            "Namespace",
            "A namespace",
            "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: example\n",
        ),
        Template::embedded("ConfigMap", "Plain settings", CONFIGMAP),
        Template::embedded(
            "Deployment",
            "",
            "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: default\n",
        ),
    ]
}

pub(super) fn service(fs: &Arc<FakeFsPort>) -> TemplateService {
    TemplateService::new(embedded(), fs.clone(), PathBuf::from(DIR))
}

pub(super) fn catalog(service: &TemplateService) -> TemplateCatalog {
    service
        .catalog()
        .now_or_never()
        .expect("the fake answers at once")
}

pub(super) fn path(file: &str) -> PathBuf {
    PathBuf::from(DIR).join(file)
}
