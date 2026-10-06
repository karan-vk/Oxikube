//! Pre-pull of the integration test images into every kind node (part of `kind-up`).
//!
//! The list is `crates/testing/oxikube_testkit/fixtures/test-images.txt`, the same file
//! `oxikube_testkit::images` names the images from, so what the tests run and what the nodes hold
//! cannot drift. Each image is pulled *inside* the node with `crictl pull`, the node's own
//! container runtime: no `kind load` (which pushes the host's image store into the node and
//! breaks on multi-arch images), the node pulls its own platform, and it works for every node of a
//! multi-node cluster. A fresh CI cluster therefore starts the suites with every image already
//! present, instead of racing a registry against a 30 second test deadline.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::time::Duration;
use xshell::{Shell, cmd};

/// Attempts per image; registries throttle and drop connections, a pinned tag is worth retrying.
const ATTEMPTS: u32 = 3;
/// Pause before the second and third attempt.
const BACKOFF: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(6)];

/// `fixtures/test-images.txt`, next to the other cluster fixtures.
fn images_file() -> PathBuf {
    crate::kind::fixtures_dir().join("test-images.txt")
}

/// The images in `text`: one per line, `#` comments and blank lines skipped, duplicates dropped.
/// A line that is not a single `name:tag` reference is an error, so a typo cannot silently drop
/// an image from the pre-pull.
pub fn parse_images(text: &str) -> Result<Vec<String>> {
    let mut images: Vec<String> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let tagged = line
            .rsplit_once(':')
            .is_some_and(|(name, tag)| !name.is_empty() && !tag.is_empty());
        if !tagged || line.contains(char::is_whitespace) {
            bail!(
                "test-images.txt line {}: `{line}` is not a single `name:tag` image reference",
                index + 1
            );
        }
        if !images.iter().any(|i| i == line) {
            images.push(line.to_owned());
        }
    }
    Ok(images)
}

/// The container CLI that runs kind's node containers: `docker` unless kind was told otherwise.
fn node_runtime(provider: Option<&str>) -> &'static str {
    match provider {
        Some("podman") => "podman",
        Some("nerdctl") => "nerdctl",
        _ => "docker",
    }
}

/// Pulls every listed image into every node of cluster `name`. Images already present are
/// skipped, so re-running `kind-up` on a warm cluster costs one `crictl inspecti` each.
pub fn preload(name: &str) -> Result<()> {
    let images = parse_images(
        &std::fs::read_to_string(images_file()).context("read fixtures/test-images.txt")?,
    )?;
    let sh = Shell::new()?;
    let nodes: Vec<String> = cmd!(sh, "kind get nodes --name {name}")
        .read()
        .context("list the kind nodes")?
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect();
    if nodes.is_empty() {
        bail!("kind cluster `{name}` has no nodes");
    }
    let provider = std::env::var("KIND_EXPERIMENTAL_PROVIDER").ok();
    let runtime = node_runtime(provider.as_deref());

    println!(
        "pre-pulling {} test image(s) into {} node(s)",
        images.len(),
        nodes.len()
    );
    let failures: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = nodes
            .iter()
            .flat_map(|node| images.iter().map(move |image| (node, image)))
            .map(|(node, image)| {
                scope.spawn(move || {
                    pull(runtime, node, image).map_err(|e| format!("{image} on {node}: {e:#}"))
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().expect("pull thread").err())
            .collect()
    });
    if !failures.is_empty() {
        bail!(
            "could not pre-pull {} image(s):\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
    }
    Ok(())
}

/// One image into one node, with retries; a present image is left alone.
fn pull(runtime: &str, node: &str, image: &str) -> Result<()> {
    let sh = Shell::new()?;
    if cmd!(sh, "{runtime} exec {node} crictl inspecti {image}")
        .quiet()
        .ignore_stdout()
        .ignore_stderr()
        .run()
        .is_ok()
    {
        return Ok(());
    }
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        match cmd!(sh, "{runtime} exec {node} crictl pull {image}")
            .quiet()
            .ignore_stdout()
            .read_stderr()
        {
            Ok(_) => {
                println!("  pulled {image} into {node}");
                return Ok(());
            }
            Err(e) => last = Some(e),
        }
        if let Some(pause) = BACKOFF.get(attempt as usize) {
            std::thread::sleep(*pause);
        }
    }
    Err(last.map_or_else(|| anyhow::anyhow!("no attempt ran"), anyhow::Error::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skips_comments_blanks_and_duplicates() {
        let images = parse_images("# c\n\n  a/b:1  \nc:2\na/b:1\n").unwrap();
        assert_eq!(images, ["a/b:1", "c:2"]);
    }

    #[test]
    fn parse_rejects_untagged_or_multi_word_lines() {
        for bad in ["busybox", "busybox:", ":1", "busybox:1 nginx:1"] {
            assert!(parse_images(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_runtime_follows_kinds_provider_variable() {
        assert_eq!(node_runtime(None), "docker");
        assert_eq!(node_runtime(Some("docker")), "docker");
        assert_eq!(node_runtime(Some("podman")), "podman");
        assert_eq!(node_runtime(Some("nerdctl")), "nerdctl");
    }

    #[test]
    fn the_shipped_list_parses_and_is_not_empty() {
        let text = std::fs::read_to_string(images_file()).unwrap();
        let images = parse_images(&text).unwrap();
        assert!(images.len() >= 5, "{images:?}");
        // Every entry is pinned to a tag, never `:latest`.
        assert!(images.iter().all(|i| !i.ends_with(":latest")), "{images:?}");
    }

    #[test]
    fn the_metrics_server_image_matches_the_pinned_release() {
        let kustomization = std::fs::read_to_string(
            crate::kind::fixtures_dir().join("metrics-server/kustomization.yaml"),
        )
        .unwrap();
        let version = kustomization
            .split("/releases/download/")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .expect("a pinned release URL");
        let image = format!("registry.k8s.io/metrics-server/metrics-server:{version}");
        let images = parse_images(&std::fs::read_to_string(images_file()).unwrap()).unwrap();
        assert!(
            images.contains(&image),
            "test-images.txt must list `{image}` (the release kustomization.yaml pins)"
        );
    }
}
