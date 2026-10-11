//! The blocks completion offers whole: a container, a volume. A small embedded table keyed by the
//! name of the list they belong to (`containers`), so the same snippet serves a Pod, a
//! Deployment, a Job and a CronJob, whatever path leads to the list. (The templates of
//! E10-S09 are separate manifests and do not share this table.)
//!
//! Bodies are inserted verbatim: the library's completion has no tab stops, so they carry
//! plausible values the reader edits (`name: app`), never `${1:placeholders}`.

/// One block.
pub(super) struct Snippet {
    /// The property the block is an item of.
    pub(super) list: &'static str,
    /// The menu label.
    pub(super) label: &'static str,
    /// What the block holds, for the menu's note.
    pub(super) detail: &'static str,
    /// The block's lines, the first being the item's first key. Indentation is relative.
    pub(super) body: &'static str,
}

const CONTAINER: &str = "name: app
image: nginx:latest
ports:
  - containerPort: 80
resources:
  requests:
    cpu: 100m
    memory: 128Mi
  limits:
    cpu: 500m
    memory: 256Mi";

const INIT_CONTAINER: &str = "name: init
image: busybox:latest
command:
  - sh
  - -c
  - echo ready";

/// Every snippet.
pub(super) const SNIPPETS: &[Snippet] = &[
    Snippet {
        list: "containers",
        label: "container",
        detail: "name, image, ports, resources",
        body: CONTAINER,
    },
    Snippet {
        list: "initContainers",
        label: "init container",
        detail: "name, image, command",
        body: INIT_CONTAINER,
    },
    Snippet {
        list: "volumes",
        label: "volume (emptyDir)",
        detail: "name, emptyDir",
        body: "name: scratch\nemptyDir: {}",
    },
    Snippet {
        list: "volumes",
        label: "volume (configMap)",
        detail: "name, configMap",
        body: "name: config\nconfigMap:\n  name: app-config",
    },
    Snippet {
        list: "volumes",
        label: "volume (persistentVolumeClaim)",
        detail: "name, persistentVolumeClaim",
        body: "name: data\npersistentVolumeClaim:\n  claimName: app-data",
    },
];

/// The snippets for items of the list `list`.
pub(super) fn for_list(list: &str) -> impl Iterator<Item = &'static Snippet> + '_ {
    SNIPPETS.iter().filter(move |s| s.list == list)
}

/// `body` with every line after the first indented by `indent` spaces (the first is placed where
/// the cursor is).
pub(super) fn indented(body: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    body.lines()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                line.to_owned()
            } else {
                format!("{pad}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
