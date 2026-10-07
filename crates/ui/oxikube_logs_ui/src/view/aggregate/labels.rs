//! What identifies a pod in a merged log: its colour (a stable hash of its name into the theme's
//! palette) and its short name (the part of the name that tells it from the others), drawn as a
//! fixed-width gutter before each line. Plain Rust, no gpui.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_app::logs::SourceInfo;

/// Widest gutter, in characters: a longer label is cut with a leading `…`.
pub const MAX_GUTTER: usize = 28;

/// The palette slot of `pod`: a hash of the name (FNV-1a), so a pod keeps its colour whatever
/// else is streaming, across reopens and restarts of the app. `slots` is the palette's length.
pub fn colour_index(pod: &str, slots: usize) -> usize {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in pod.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    usize::try_from(hash % slots.max(1) as u64).unwrap_or(0)
}

/// The short names of `pods`: the common prefix of the names (up to a `-`) is dropped, so
/// `web-7d9c4b5f6-abcde` and `web-7d9c4b5f6-fghij` become `abcde` and `fghij`, and `db-0`, `db-1`
/// become `0`, `1`. A lone pod keeps the last `-` segment of its name. The result is in the order
/// of `pods`.
pub fn short_names(pods: &[&str]) -> Vec<String> {
    let strip = if pods.len() > 1 {
        common_prefix_len(pods)
    } else {
        pods.first()
            .and_then(|pod| pod.rfind('-').map(|at| at + 1))
            .unwrap_or(0)
    };
    pods.iter()
        .map(|pod| {
            let short = &pod[strip..];
            if short.is_empty() { pod } else { short }.to_owned()
        })
        .collect()
}

/// Length of the longest prefix shared by every name that ends in a `-`.
fn common_prefix_len(names: &[&str]) -> usize {
    let first = names[0];
    let shared = first
        .char_indices()
        .take_while(|&(i, c)| {
            names
                .iter()
                .all(|n| n.get(i..).is_some_and(|rest| rest.starts_with(c)))
        })
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    first[..shared].rfind('-').map_or(0, |at| at + 1)
}

/// One source's gutter: its text, padded to the gutter's width, and its palette slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefix {
    /// The padded label.
    pub text: Arc<str>,
    /// The palette slot (`colour_index` of the pod, for the palette's length).
    pub colour: usize,
}

/// The gutters of an aggregate's sources.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceLabels {
    width: usize,
    by_pod: HashMap<Arc<str>, Vec<(Arc<str>, Prefix)>>,
}

impl SourceLabels {
    /// The labels for `sources`: the pod's short name, plus `/container` when the sources have
    /// more than one container name between them; `slots` is the palette's length.
    pub fn new(sources: &[SourceInfo], slots: usize) -> Self {
        let mut pods: Vec<&str> = Vec::new();
        for source in sources {
            if !pods.contains(&&*source.pod) {
                pods.push(&source.pod);
            }
        }
        let shorts = short_names(&pods);
        let mut containers: Vec<&str> = sources.iter().map(|s| &*s.container).collect();
        containers.sort_unstable();
        containers.dedup();
        let with_container = containers.len() > 1;

        let mut labels: Vec<(Arc<str>, Arc<str>, String)> = Vec::new();
        for source in sources {
            let at = pods.iter().position(|p| *p == &*source.pod).unwrap_or(0);
            let mut label = shorts[at].clone();
            if with_container {
                label.push('/');
                label.push_str(&source.container);
            }
            labels.push((source.pod.clone(), source.container.clone(), label));
        }
        let width = labels
            .iter()
            .map(|(_, _, label)| label.chars().count())
            .max()
            .unwrap_or(0)
            .min(MAX_GUTTER);
        let mut by_pod: HashMap<Arc<str>, Vec<(Arc<str>, Prefix)>> = HashMap::new();
        for (pod, container, label) in labels {
            let prefix = Prefix {
                text: Arc::from(fit(&label, width)),
                colour: colour_index(&pod, slots),
            };
            by_pod.entry(pod).or_default().push((container, prefix));
        }
        Self { width, by_pod }
    }

    /// Characters of the gutter.
    pub fn width(&self) -> usize {
        self.width
    }

    /// The gutter of the line of `container` of `pod`; `None` for a source that was never opened.
    pub fn prefix(&self, pod: &str, container: &str) -> Option<&Prefix> {
        self.by_pod
            .get(pod)?
            .iter()
            .find(|(name, _)| &**name == container)
            .map(|(_, prefix)| prefix)
    }
}

/// `label` padded with spaces to `width` characters, or cut to it with a leading `…`.
fn fit(label: &str, width: usize) -> String {
    let chars = label.chars().count();
    if chars <= width {
        return format!("{label}{}", " ".repeat(width - chars));
    }
    let keep: String = label
        .chars()
        .skip(chars - width.saturating_sub(1))
        .collect();
    format!("…{keep}")
}

#[cfg(test)]
mod tests {
    use oxikube_app::logs::SourceState;

    use super::*;

    fn source(id: u32, pod: &str, container: &str) -> SourceInfo {
        SourceInfo {
            id,
            pod: Arc::from(pod),
            container: Arc::from(container),
            state: SourceState::Streaming,
        }
    }

    #[test]
    fn the_colour_is_a_stable_hash_of_the_pod_name() {
        let a = colour_index("web-7d9c4b5f6-abcde", 10);
        assert_eq!(a, colour_index("web-7d9c4b5f6-abcde", 10), "stable");
        assert!(a < 10);
        let distinct: std::collections::HashSet<usize> = (0..40)
            .map(|i| colour_index(&format!("web-7d9c4b5f6-pod{i}"), 10))
            .collect();
        assert!(
            distinct.len() >= 7,
            "names spread over the palette: {distinct:?}"
        );
        assert_eq!(colour_index("x", 0), 0, "an empty palette does not panic");
    }

    #[test]
    fn short_names_drop_what_the_pods_share() {
        assert_eq!(
            short_names(&["web-7d9c4b5f6-abcde", "web-7d9c4b5f6-fghij"]),
            ["abcde", "fghij"]
        );
        assert_eq!(short_names(&["db-0", "db-1", "db-2"]), ["0", "1", "2"]);
        assert_eq!(
            short_names(&["web-7d9-x1", "web-8aa-y2"]),
            ["7d9-x1", "8aa-y2"],
            "a rollout's two replica sets"
        );
        assert_eq!(
            short_names(&["web-7d9c4b5f6-abcde"]),
            ["abcde"],
            "a lone pod"
        );
        assert_eq!(short_names(&["solo"]), ["solo"]);
        assert_eq!(
            short_names(&["a-1", "b-2"]),
            ["a-1", "b-2"],
            "nothing shared"
        );
    }

    #[test]
    fn the_gutter_has_one_width_and_names_the_container_only_when_there_are_several() {
        let one = SourceLabels::new(
            &[source(0, "web-a1", "app"), source(1, "web-b22", "app")],
            10,
        );
        assert_eq!(one.width(), 3);
        assert_eq!(&*one.prefix("web-a1", "app").unwrap().text, "a1 ");
        assert_eq!(&*one.prefix("web-b22", "app").unwrap().text, "b22");
        assert!(one.prefix("web-zzz", "app").is_none());

        let two = SourceLabels::new(
            &[
                source(0, "web-a1", "app"),
                source(1, "web-a1", "proxy"),
                source(2, "web-b2", "app"),
            ],
            10,
        );
        assert_eq!(&*two.prefix("web-a1", "proxy").unwrap().text, "a1/proxy");
        assert_eq!(&*two.prefix("web-b2", "app").unwrap().text, "b2/app  ");
        assert_eq!(
            two.prefix("web-a1", "app").unwrap().colour,
            two.prefix("web-a1", "proxy").unwrap().colour,
            "a pod's containers share its colour"
        );
    }

    #[test]
    fn a_label_longer_than_the_gutter_is_cut_from_the_left() {
        assert_eq!(fit("abcdefghij", 5), "…ghij");
        assert_eq!(fit("ab", 4), "ab  ");
    }
}
