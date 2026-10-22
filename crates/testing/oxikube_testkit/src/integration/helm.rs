//! The real `helm` binary and Helm fixtures for the Helm kind suites (E16-S12).
//!
//! | Item | What it is for |
//! |---|---|
//! | [`HelmBin::find`] | the binary under test ([`HELM_ENV`] or `helm` on `PATH`), `None` with a skip notice when there is none or it is older than 3.10 |
//! | [`HelmBin::run`] | one `helm` command pinned to the kind context, a namespace and a storage [`Driver`], as the "made by the real CLI" side of a test |
//! | [`write_chart`] | a local chart (no repository, no image, nothing scheduled): one ConfigMap, a second one from version 0.2.0 on, `NOTES.txt` |
//! | [`release_record`] | a release object (Secret or ConfigMap) in Helm's own storage format, for listing many releases without installing them |
//!
//! The chart's values hold no credentials; a test that checks redaction passes its own token.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Value, json};

use super::IntegrationError;

/// Environment variable naming the `helm` binary to test (another version than the one on
/// `PATH`, for example Helm 3.10 in the nightly matrix).
pub const HELM_ENV: &str = "OXIKUBE_TEST_HELM";

/// The chart [`write_chart`] writes.
pub const CHART_NAME: &str = "oxi-it";

/// Where Helm keeps a release's records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    /// `HELM_DRIVER=secret` (Helm's default): `Secret`s of type `helm.sh/release.v1`.
    Secret,
    /// `HELM_DRIVER=configmap`: `ConfigMap`s.
    ConfigMap,
}

impl Driver {
    /// The value of `HELM_DRIVER`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Secret => "secret",
            Self::ConfigMap => "configmap",
        }
    }
}

/// A `helm` binary of at least version 3.10.
#[derive(Debug, Clone)]
pub struct HelmBin {
    path: PathBuf,
    version: String,
}

impl HelmBin {
    /// The binary named by [`HELM_ENV`], else `helm` on `PATH` (as an absolute path, so it can
    /// be handed to the adapter as its `helm.path`). `None` (after a skip notice) when there is
    /// none, it cannot be run or it is older than 3.10, so a CLI test returns early.
    pub fn find() -> Option<Self> {
        let Some(path) = std::env::var_os(HELM_ENV)
            .map(PathBuf::from)
            .or_else(on_path)
        else {
            eprintln!("skipping: no `helm` binary on PATH and {HELM_ENV} is unset");
            return None;
        };
        let out = Command::new(&path)
            .args(["version", "--template", "{{.Version}}"])
            .output();
        let version = match out {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim().to_owned()
            }
            _ => {
                eprintln!("skipping: no `helm` binary at {}", path.display());
                return None;
            }
        };
        if !supported(&version) {
            eprintln!("skipping: helm {version} is older than 3.10");
            return None;
        }
        Some(Self { path, version })
    }

    /// The binary.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Its version, as `helm version` prints it (`v4.3.1`).
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Runs `helm --kube-context <context> -n <namespace> <args>` with `HELM_DRIVER=<driver>` and
    /// returns its stdout. The error carries stderr, which for these charts holds no secrets.
    pub fn run(
        &self,
        context: &str,
        namespace: &str,
        driver: Driver,
        args: &[&str],
    ) -> Result<String, IntegrationError> {
        let out = Command::new(&self.path)
            .args(["--kube-context", context, "-n", namespace])
            .args(args)
            .env("HELM_DRIVER", driver.as_str())
            .output()
            .map_err(|e| IntegrationError::Kubectl(format!("helm could not be run: {e}")))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(IntegrationError::Kubectl(format!(
                "helm {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }
}

/// `helm` in the first `PATH` directory that has it.
fn on_path() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join("helm"))
        .find(|candidate| candidate.is_file())
}

/// Whether `version` (`v3.10.3`, `v4.3.1+gabc`) is at least 3.10.
fn supported(version: &str) -> bool {
    let mut parts = version.trim_start_matches('v').split(['.', '+', '-']);
    let mut next = || parts.next().and_then(|p| p.parse::<u32>().ok());
    matches!((next(), next()), (Some(major), Some(minor)) if (major, minor) >= (3, 10))
}

/// Writes chart [`CHART_NAME`] at `version` into `dir` and returns `dir`.
///
/// It renders ConfigMap `<release>-cfg` (`greeting` from the values, `chart` the chart version)
/// and, when `extra` is set, ConfigMap `<release>-extra`, so an upgrade to the `extra` chart adds
/// an object and a rollback removes it. `NOTES.txt` says `Installed <release>.`.
///
/// # Panics
///
/// When `dir` cannot be written.
pub fn write_chart(dir: &Path, version: &str, extra: bool) -> PathBuf {
    let templates = dir.join("templates");
    std::fs::create_dir_all(&templates).expect("chart directory");
    let write = |path: PathBuf, text: &str| std::fs::write(path, text).expect("chart file");
    write(
        dir.join("Chart.yaml"),
        &format!(
            "apiVersion: v2\nname: {CHART_NAME}\nversion: {version}\nappVersion: \"{version}\"\n"
        ),
    );
    write(dir.join("values.yaml"), "greeting: hello\ntoken: \"\"\n");
    write(
        templates.join("cfg.yaml"),
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {{ .Release.Name }}-cfg\n\
         data:\n  greeting: {{ .Values.greeting | quote }}\n  chart: {{ .Chart.Version | quote }}\n",
    );
    if extra {
        write(
            templates.join("extra.yaml"),
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {{ .Release.Name }}-extra\n\
             data:\n  added: \"true\"\n",
        );
    }
    write(
        templates.join("NOTES.txt"),
        "Installed {{ .Release.Name }}.\n",
    );
    dir.to_path_buf()
}

/// Revision `revision` of release `name` in `namespace`, `deployed`, as the object `driver`
/// stores it (labels and `data.release` exactly as Helm writes them: base64 of gzipped JSON,
/// base64'd once more by the API for a Secret). Ready for `kubectl create -f -`.
pub fn release_record(driver: Driver, namespace: &str, name: &str, revision: u32) -> Value {
    let record = json!({
        "name": name,
        "namespace": namespace,
        "version": revision,
        "info": {
            "first_deployed": "2026-01-01T00:00:00Z",
            "last_deployed": "2026-01-01T00:00:00Z",
            "description": "Install complete",
            "status": "deployed",
        },
        "chart": {"metadata": {"name": CHART_NAME, "version": "0.1.0", "appVersion": "0.1.0"}},
        "config": {},
        "manifest": "",
    });
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(record.to_string().as_bytes())
        .expect("gzip in memory");
    let payload = STANDARD.encode(gz.finish().expect("gzip in memory"));
    let metadata = json!({
        "name": format!("sh.helm.release.v1.{name}.v{revision}"),
        "labels": {
            "owner": "helm", "name": name, "status": "deployed",
            "version": revision.to_string(), "modifiedAt": "1767225600",
        },
    });
    match driver {
        Driver::Secret => json!({
            "apiVersion": "v1", "kind": "Secret", "type": "helm.sh/release.v1",
            "metadata": metadata, "data": {"release": STANDARD.encode(payload)},
        }),
        Driver::ConfigMap => json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": metadata, "data": {"release": payload},
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_from_3_10_are_supported() {
        assert!(supported("v3.10.0"));
        assert!(supported("v3.19.2"));
        assert!(supported("v4.3.1+gc0d08d0"));
        assert!(!supported("v3.9.4"));
        assert!(!supported("v2.17.0"));
        assert!(!supported("garbage"));
    }

    #[test]
    fn a_secret_record_is_base64_twice() {
        let object = release_record(Driver::Secret, "ns", "web", 2);
        assert_eq!(object["metadata"]["name"], "sh.helm.release.v1.web.v2");
        let outer = object["data"]["release"].as_str().unwrap();
        let inner = STANDARD.decode(outer).unwrap();
        let gz = STANDARD.decode(inner).unwrap();
        assert_eq!(&gz[..2], &[0x1f, 0x8b], "gzip magic");
    }
}
