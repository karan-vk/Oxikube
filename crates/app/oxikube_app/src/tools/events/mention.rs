//! The `@events/...` grammar.

use oxikube_domain::{OxiError, OxiResult};

const GRAMMAR: &str = "expected @events, @events/<namespace>, @events/<kind>/<namespace>/<name>, \
                       or @events/<kind>/<name> for a cluster-scoped object";

/// What an `@events` mention names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventsMention {
    /// The newest events of a namespace (`@events/shop`), or of the thread's namespace
    /// (`@events` in a thread that has one), or of the whole cluster.
    Recent {
        /// The namespace; `None`: every namespace.
        namespace: Option<String>,
    },
    /// The events about one object (`@events/pod/shop/web-0`, `@events/node/worker-1`).
    Object {
        /// A kind word: `pod`, `deploy`, `Deployment`, resolved with the cluster's aliases.
        kind: String,
        /// The object's namespace; `None` for a cluster-scoped object.
        namespace: Option<String>,
        /// The object's name.
        name: String,
    },
}

impl EventsMention {
    /// Reads the segments after `@events`.
    ///
    /// # Errors
    ///
    /// A validation error that shows the grammar, for any other shape.
    pub fn parse(path: &[String], default_namespace: Option<&str>) -> OxiResult<Self> {
        match path {
            [] => Ok(Self::Recent {
                namespace: default_namespace.map(str::to_owned),
            }),
            [namespace] => Ok(Self::Recent {
                namespace: Some(namespace.clone()),
            }),
            [kind, name] => Ok(Self::Object {
                kind: kind.clone(),
                namespace: None,
                name: name.clone(),
            }),
            [kind, namespace, name] => Ok(Self::Object {
                kind: kind.clone(),
                namespace: Some(namespace.clone()),
                name: name.clone(),
            }),
            _ => Err(OxiError::validation(format!(
                "@events/{} has too many segments: {GRAMMAR}",
                path.join("/")
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn an_object_is_named_by_kind_namespace_and_name() {
        let m = EventsMention::parse(&path(&["pod", "shop", "web-0"]), None).unwrap();
        assert_eq!(
            m,
            EventsMention::Object {
                kind: "pod".into(),
                namespace: Some("shop".into()),
                name: "web-0".into()
            }
        );
    }

    #[test]
    fn a_cluster_scoped_object_has_no_namespace_segment() {
        let m = EventsMention::parse(&path(&["node", "worker-1"]), Some("shop")).unwrap();
        assert_eq!(
            m,
            EventsMention::Object {
                kind: "node".into(),
                namespace: None,
                name: "worker-1".into()
            }
        );
    }

    #[test]
    fn fewer_segments_name_the_recent_events_of_a_namespace() {
        assert_eq!(
            EventsMention::parse(&path(&["shop"]), None).unwrap(),
            EventsMention::Recent {
                namespace: Some("shop".into())
            }
        );
        assert_eq!(
            EventsMention::parse(&path(&[]), Some("shop")).unwrap(),
            EventsMention::Recent {
                namespace: Some("shop".into())
            }
        );
        assert_eq!(
            EventsMention::parse(&path(&[]), None).unwrap(),
            EventsMention::Recent { namespace: None }
        );
    }

    #[test]
    fn too_many_segments_show_the_grammar() {
        let error = EventsMention::parse(&path(&["pod", "a", "b", "c"]), None).unwrap_err();
        assert!(
            error
                .message()
                .contains("@events/<kind>/<namespace>/<name>"),
            "{}",
            error.message()
        );
    }
}
