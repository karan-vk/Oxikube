//! The embedded templates: one minimal manifest per core kind, as `templates/*.yaml` files
//! compiled in.

use oxikube_app::Template;

/// One embedded template: the name the picker lists, a line under it, and the manifest.
struct Entry {
    name: &'static str,
    summary: &'static str,
    body: &'static str,
}

macro_rules! entries {
    ($($name:literal, $summary:literal, $file:literal;)*) => {
        &[$(Entry {
            name: $name,
            summary: $summary,
            body: include_str!(concat!("../../templates/", $file)),
        }),*]
    };
}

/// In the order the picker lists them when the query is empty: workloads, then config, network,
/// storage, access control, scaling and quotas, definitions, and the one multi-document example.
const ENTRIES: &[Entry] = entries! {
    "Namespace", "A namespace to put objects in", "namespace.yaml";
    "Pod", "A single nginx container", "pod.yaml";
    "Deployment", "Replicated pods with rolling updates", "deployment.yaml";
    "StatefulSet", "Pods with stable names and identity", "statefulset.yaml";
    "DaemonSet", "One pod on every node", "daemonset.yaml";
    "ReplicaSet", "A fixed number of identical pods", "replicaset.yaml";
    "Job", "A run-to-completion task", "job.yaml";
    "CronJob", "A Job on a schedule", "cronjob.yaml";
    "ConfigMap", "Settings as key/value pairs", "configmap.yaml";
    "Secret", "Credentials as key/value pairs (stringData)", "secret.yaml";
    "Service", "A stable address for pods", "service.yaml";
    "Ingress", "HTTP routing to a Service", "ingress.yaml";
    "NetworkPolicy", "Which pods may talk to which", "networkpolicy.yaml";
    "PersistentVolumeClaim", "A request for storage", "persistentvolumeclaim.yaml";
    "PersistentVolume", "A piece of storage (hostPath)", "persistentvolume.yaml";
    "StorageClass", "A kind of storage and how it is provisioned", "storageclass.yaml";
    "ServiceAccount", "An identity for pods", "serviceaccount.yaml";
    "Role", "Permissions within a namespace", "role.yaml";
    "RoleBinding", "Grants a Role to a ServiceAccount", "rolebinding.yaml";
    "ClusterRole", "Permissions across the cluster", "clusterrole.yaml";
    "ClusterRoleBinding", "Grants a ClusterRole to a ServiceAccount", "clusterrolebinding.yaml";
    "HorizontalPodAutoscaler", "Scales a Deployment on CPU", "horizontalpodautoscaler.yaml";
    "PodDisruptionBudget", "Pods that must stay up during maintenance", "poddisruptionbudget.yaml";
    "ResourceQuota", "Caps what a namespace may use", "resourcequota.yaml";
    "LimitRange", "Default and maximum container resources", "limitrange.yaml";
    "CustomResourceDefinition", "A new kind of object (Widget)", "customresourcedefinition.yaml";
    "Deployment and Service", "Two documents, applied in order", "deployment-and-service.yaml";
};

/// The embedded templates, in the picker's order. Parses each manifest once (the kind and the
/// number of documents), so call it where blocking is fine: when the picker opens it runs with
/// the directory scan.
pub fn embedded() -> Vec<Template> {
    ENTRIES
        .iter()
        .map(|entry| Template::embedded(entry.name, entry.summary, entry.body))
        .collect()
}
