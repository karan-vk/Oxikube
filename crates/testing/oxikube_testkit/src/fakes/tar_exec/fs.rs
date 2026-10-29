//! The in-memory filesystem of [`FakeTarExec`](super::FakeTarExec).

use std::collections::BTreeMap;

/// Absolute, `/`-separated, without `.`, empty or trailing components.
pub(super) fn normalize(path: &str) -> String {
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    format!("/{}", parts.join("/"))
}

/// `dir` joined with `name`.
pub(super) fn join(dir: &str, name: &str) -> String {
    normalize(&format!("{dir}/{name}"))
}

fn parent(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }
    let (head, _) = path.rsplit_once('/')?;
    Some(if head.is_empty() {
        "/".to_owned()
    } else {
        head.to_owned()
    })
}

#[derive(Clone)]
pub(super) enum Node {
    Dir { mode: u32 },
    File { data: Vec<u8>, mode: u32 },
}

#[derive(Default)]
pub(super) struct MemFs {
    nodes: BTreeMap<String, Node>,
}

impl MemFs {
    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn paths(&self) -> Vec<String> {
        self.nodes.keys().cloned().collect()
    }

    fn ensure_parents(&mut self, path: &str) {
        let mut next = parent(path);
        while let Some(dir) = next {
            if dir == "/" {
                break;
            }
            self.nodes
                .entry(dir.clone())
                .or_insert(Node::Dir { mode: 0o755 });
            next = parent(&dir);
        }
    }

    pub(super) fn put_dir(&mut self, path: &str, mode: u32) {
        let path = normalize(path);
        self.ensure_parents(&path);
        if path != "/" {
            self.nodes.insert(path, Node::Dir { mode });
        }
    }

    pub(super) fn put_file(&mut self, path: &str, data: Vec<u8>, mode: u32) {
        let path = normalize(path);
        self.ensure_parents(&path);
        self.nodes.insert(path, Node::File { data, mode });
    }

    pub(super) fn node(&self, path: &str) -> Option<&Node> {
        self.nodes.get(&normalize(path))
    }

    pub(super) fn is_dir(&self, path: &str) -> bool {
        let path = normalize(path);
        path == "/" || matches!(self.nodes.get(&path), Some(Node::Dir { .. }))
    }

    pub(super) fn file(&self, path: &str) -> Option<(Vec<u8>, u32)> {
        match self.node(path)? {
            Node::File { data, mode } => Some((data.clone(), *mode)),
            Node::Dir { .. } => None,
        }
    }

    pub(super) fn mode(&self, path: &str) -> Option<u32> {
        match self.node(path)? {
            Node::File { mode, .. } | Node::Dir { mode } => Some(*mode),
        }
    }

    /// `root` and everything below it, parents first.
    pub(super) fn subtree(&self, root: &str) -> Vec<(String, Node)> {
        let root = normalize(root);
        let prefix = format!("{root}/");
        self.nodes
            .iter()
            .filter(|(path, _)| **path == root || path.starts_with(&prefix))
            .map(|(path, node)| (path.clone(), node.clone()))
            .collect()
    }
}
