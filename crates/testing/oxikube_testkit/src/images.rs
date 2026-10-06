//! The container images the kind integration suites run, and the one list `cargo xtask kind-up`
//! pre-pulls into every node.
//!
//! The list is `fixtures/test-images.txt`. A test that needs an image names it through a
//! constant here instead of writing a literal, so the image is guaranteed to be on the node
//! before the first test starts and no test waits on a registry or depends on an image that
//! only happens to be cached on a developer's long-lived cluster. `tests/test_images.rs` fails
//! when an integration test or a cluster fixture uses an image that is not in the list.

/// The raw `fixtures/test-images.txt` (one image per line, `#` comments).
pub const LIST: &str = include_str!("../fixtures/test-images.txt");

/// The pause image: the cheapest pod that stays running. The kind node ships it.
pub const PAUSE: &str = "registry.k8s.io/pause:3.10";

/// An older pause image: the second revision of a rollout history scenario.
pub const PAUSE_PREVIOUS: &str = "registry.k8s.io/pause:3.9";

/// busybox from the Kubernetes e2e images: pods that write predictable logs, ephemeral
/// containers.
pub const E2E_BUSYBOX: &str = "registry.k8s.io/e2e-test-images/busybox:1.36.1-1";

/// busybox with `sh`, `cat`, `stty`, `sleep` and `nsenter`: exec, attach and node shell scenarios.
/// Also the adapter's default node shell image.
pub const BUSYBOX: &str = "busybox:1.37";

/// nginx, small, serving its welcome page on port 80 as soon as it starts.
pub const NGINX: &str = "nginx:1.27-alpine";

/// The images of [`LIST`], in file order, without comments, blanks or duplicates.
pub fn all() -> Vec<&'static str> {
    parse(LIST)
}

/// The images in `text`: one per line, `#` comments and blank lines skipped, duplicates dropped.
/// Panics on a line that is not a single image reference, so a typo cannot silently drop an
/// image from the pre-pull.
pub fn parse(text: &'static str) -> Vec<&'static str> {
    let mut images: Vec<&'static str> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        assert!(
            is_image_reference(line),
            "test-images.txt line {}: `{line}` is not an image reference",
            index + 1
        );
        if !images.contains(&line) {
            images.push(line);
        }
    }
    images
}

/// Whether `text` looks like `[registry/]path:tag`: no whitespace, a tag that starts with an
/// alphanumeric character, only the characters of an image name. A digest is not accepted here
/// because the list pins by tag.
pub fn is_image_reference(text: &str) -> bool {
    let Some((name, tag)) = text.rsplit_once(':') else {
        return false;
    };
    let name_ok = !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
    let tag_ok = tag.starts_with(|c: char| c.is_ascii_alphanumeric())
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    name_ok && tag_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_names_every_constant_once() {
        let all = all();
        for image in [PAUSE, PAUSE_PREVIOUS, E2E_BUSYBOX, BUSYBOX, NGINX] {
            assert_eq!(all.iter().filter(|i| **i == image).count(), 1, "{image}");
        }
    }

    #[test]
    fn parse_skips_comments_blanks_and_duplicates() {
        let text = "# c\n\n  a/b:1  \nc:2\na/b:1\n";
        assert_eq!(parse(text), ["a/b:1", "c:2"]);
    }

    #[test]
    #[should_panic(expected = "not an image reference")]
    fn parse_rejects_a_line_with_two_words() {
        parse("busybox:1 nginx:1\n");
    }

    #[test]
    fn image_references_need_a_name_and_a_tag() {
        assert!(is_image_reference("busybox:1.37"));
        assert!(is_image_reference(
            "registry.k8s.io/e2e-test-images/busybox:1.36.1-1"
        ));
        assert!(!is_image_reference("busybox"));
        assert!(!is_image_reference("busybox:"));
        assert!(!is_image_reference("pasted: 0123"));
        assert!(!is_image_reference("127.0.0.1:0 and more"));
    }
}
