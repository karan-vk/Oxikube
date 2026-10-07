//! Pod manifests for the terminal and exec kind suites (E09-S13), as JSON ready for
//! `serde_json::from_value::<Pod>` or `kubectl apply -f -`.
//!
//! Every pod runs one container named [`MAIN`], never restarts and has no termination grace
//! period, so deleting a test's namespace is quick. Images come from [`images`], so
//! `cargo xtask kind-up` has already pulled them into the nodes. The pods carry the label
//! [`SUITE_LABEL`] so a leftover is easy to find with `kubectl get pods -A -l oxikube.test/suite`.
//!
//! | Fixture | What it is for |
//! |---|---|
//! | [`sleeper`] | a busybox pod that sleeps: exec, resize, full-screen programs, debug containers |
//! | [`cat`] | the main process is `cat` with stdin open: attach round trips |
//! | [`logger`] | the main process prints `tick <n>` every second: attach to a long-running pod |
//! | [`shell_less`] | a pod whose only binary is `/pause`: the stand-in for a distroless image (no shell, so the shell flow offers a debug container) |

use serde_json::{Value, json};

use crate::images;

/// The name of the container every fixture runs.
pub const MAIN: &str = "main";

/// The label key every fixture carries (value: the fixture's name).
pub const SUITE_LABEL: &str = "oxikube.test/suite";

/// The line [`logger`] prints, as `<LOGGER_PREFIX><n>` with `n` counting from 1.
pub const LOGGER_PREFIX: &str = "tick ";

fn pod(name: &str, fixture: &str, container: Value) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": name, "labels": { SUITE_LABEL: fixture } },
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": [container],
        },
    })
}

/// A busybox pod `name` that sleeps for an hour.
pub fn sleeper(name: &str) -> Value {
    pod(
        name,
        "sleeper",
        json!({ "name": MAIN, "image": images::BUSYBOX, "command": ["sleep", "3600"] }),
    )
}

/// A busybox pod `name` whose main process is `cat` with stdin open, so `attach` talks to it.
pub fn cat(name: &str) -> Value {
    pod(
        name,
        "cat",
        json!({ "name": MAIN, "image": images::BUSYBOX, "command": ["cat"], "stdin": true }),
    )
}

/// A busybox pod `name` that prints `tick 1`, `tick 2`, ... one line per second until deleted.
pub fn logger(name: &str) -> Value {
    let script =
        format!("n=0; while true; do n=$((n+1)); echo \"{LOGGER_PREFIX}$n\"; sleep 1; done");
    pod(
        name,
        "logger",
        json!({ "name": MAIN, "image": images::BUSYBOX, "command": ["sh", "-c", script] }),
    )
}

/// A pod `name` running the pause image: no shell and no tools, like a distroless image. A shell
/// in it fails with a pointer to a debug container; a debug container sees its process.
pub fn shell_less(name: &str) -> Value {
    pod(
        name,
        "shell-less",
        json!({ "name": MAIN, "image": images::PAUSE }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fixture_is_one_container_named_main_on_a_listed_image() {
        let listed = images::all();
        for pod in [sleeper("a"), cat("a"), logger("a"), shell_less("a")] {
            let containers = pod["spec"]["containers"].as_array().expect("containers");
            assert_eq!(containers.len(), 1);
            assert_eq!(containers[0]["name"], MAIN);
            let image = containers[0]["image"].as_str().expect("an image");
            assert!(listed.contains(&image), "{image} is not in test-images.txt");
            assert_eq!(pod["spec"]["restartPolicy"], "Never");
            assert!(pod["metadata"]["labels"][SUITE_LABEL].is_string());
        }
    }

    #[test]
    fn the_logger_counts_from_one() {
        let pod = logger("l");
        let script = pod["spec"]["containers"][0]["command"][2].as_str().unwrap();
        assert!(script.contains("echo \"tick $n\""), "{script}");
        assert!(script.starts_with("n=0;"), "{script}");
    }
}
