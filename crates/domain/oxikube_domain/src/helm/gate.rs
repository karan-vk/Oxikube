//! [`HelmGate`]: what the Helm views, commands and tools may offer, from [`HelmCapabilities`]
//! (E16-S11).
//!
//! One pure mapping so no view, palette entry or tool decides on its own. The gate answers
//! "may I offer this [`HelmFeature`]?" and, when not, "what do I tell the user to fix it?".
//! Native reads (releases, history, values, manifest, notes, resources) need no binary; the
//! mutating features and the chart catalog need a supported `helm`. Gating is advisory UX: the
//! `MutationGuard` still enforces policy on every mutation.

use serde::Serialize;

use super::capabilities::{HelmCapabilities, HelmVersion};
use crate::command::Capabilities;

/// One thing the Helm views and commands offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HelmFeature {
    /// The releases list.
    Releases,
    /// A release's revision history.
    History,
    /// A release's values.
    Values,
    /// A release's rendered manifest.
    Manifest,
    /// A release's NOTES.
    Notes,
    /// A release's resources.
    Resources,
    /// Install a chart.
    Install,
    /// Upgrade a release.
    Upgrade,
    /// Roll a release back.
    Rollback,
    /// Uninstall a release.
    Uninstall,
    /// Chart repositories.
    Repos,
    /// The charts catalog (search, versions, README).
    Charts,
}

impl HelmFeature {
    /// Every feature, native ones first.
    pub const ALL: [HelmFeature; 12] = [
        HelmFeature::Releases,
        HelmFeature::History,
        HelmFeature::Values,
        HelmFeature::Manifest,
        HelmFeature::Notes,
        HelmFeature::Resources,
        HelmFeature::Install,
        HelmFeature::Upgrade,
        HelmFeature::Rollback,
        HelmFeature::Uninstall,
        HelmFeature::Repos,
        HelmFeature::Charts,
    ];

    /// `true` for the features that drive the `helm` binary; the others read release
    /// Secrets/ConfigMaps natively.
    pub const fn needs_cli(self) -> bool {
        !matches!(
            self,
            HelmFeature::Releases
                | HelmFeature::History
                | HelmFeature::Values
                | HelmFeature::Manifest
                | HelmFeature::Notes
                | HelmFeature::Resources
        )
    }

    /// The stable snake_case name (the same as in tool results).
    pub const fn name(self) -> &'static str {
        match self {
            HelmFeature::Releases => "releases",
            HelmFeature::History => "history",
            HelmFeature::Values => "values",
            HelmFeature::Manifest => "manifest",
            HelmFeature::Notes => "notes",
            HelmFeature::Resources => "resources",
            HelmFeature::Install => "install",
            HelmFeature::Upgrade => "upgrade",
            HelmFeature::Rollback => "rollback",
            HelmFeature::Uninstall => "uninstall",
            HelmFeature::Repos => "repos",
            HelmFeature::Charts => "charts",
        }
    }
}

/// Why the CLI features are not available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GateReason {
    /// No `helm` binary at the configured path, on `PATH` or in the usual directories.
    BinaryMissing,
    /// A binary older than the supported minimum.
    VersionTooOld {
        /// What the binary reported.
        found: String,
        /// The minimum supported version.
        needed: String,
    },
    /// Helm has not been detected yet (start-up, or no check has run): nothing is known.
    NotConfigured,
}

impl GateReason {
    /// How to fix it, in one or two sentences: says to install Helm or set the path.
    pub fn guidance(&self) -> String {
        match self {
            GateReason::BinaryMissing => "Helm is not installed or not on PATH. Install Helm \
                 3.10 or newer, or set its path in Settings (helm.path)."
                .to_owned(),
            GateReason::VersionTooOld { found, needed } => format!(
                "Helm {found} is too old; {needed} or newer is required. Upgrade Helm, or set \
                 the path of a newer binary in Settings (helm.path)."
            ),
            GateReason::NotConfigured => "Looking for Helm. If it is not found, install Helm \
                 3.10 or newer, or set its path in Settings (helm.path)."
                .to_owned(),
        }
    }

    /// The stable snake_case name (the `kind` in tool results).
    pub const fn name(&self) -> &'static str {
        match self {
            GateReason::BinaryMissing => "binary_missing",
            GateReason::VersionTooOld { .. } => "version_too_old",
            GateReason::NotConfigured => "not_configured",
        }
    }
}

/// How much of Helm is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelmGate {
    /// A supported `helm` binary: everything works.
    Full,
    /// No usable binary, but releases are read natively: the mutating features and the
    /// charts catalog are hidden, with guidance.
    NativeOnly {
        /// Why there is no CLI.
        reason: GateReason,
    },
    /// Nothing works: no native reader and no binary (or nothing detected yet).
    Unavailable {
        /// Why.
        reason: GateReason,
    },
}

impl Default for HelmGate {
    /// Nothing known yet: [`Unavailable`](HelmGate::Unavailable) with
    /// [`NotConfigured`](GateReason::NotConfigured), until detection reports.
    fn default() -> Self {
        Self::Unavailable {
            reason: GateReason::NotConfigured,
        }
    }
}

impl HelmGate {
    /// The gate for `capabilities`: a supported binary is [`Full`](Self::Full); otherwise the
    /// native reader decides between [`NativeOnly`](Self::NativeOnly) and
    /// [`Unavailable`](Self::Unavailable).
    pub fn from_capabilities(capabilities: &HelmCapabilities) -> Self {
        if capabilities.cli_available {
            return Self::Full;
        }
        let too_old = capabilities.cli_version.filter(|v| !v.is_supported());
        let reason = |fallback: GateReason| match too_old {
            Some(found) => GateReason::VersionTooOld {
                found: found.to_string(),
                needed: HelmVersion::MINIMUM.to_string(),
            },
            None => fallback,
        };
        if capabilities.native_read {
            Self::NativeOnly {
                reason: reason(GateReason::BinaryMissing),
            }
        } else {
            Self::Unavailable {
                reason: reason(GateReason::BinaryMissing),
            }
        }
    }

    /// Whether `feature` may be offered.
    pub fn allows(&self, feature: HelmFeature) -> bool {
        match self {
            Self::Full => true,
            Self::NativeOnly { .. } => !feature.needs_cli(),
            Self::Unavailable { .. } => false,
        }
    }

    /// Why something is hidden, `None` for [`Full`](Self::Full).
    pub fn reason(&self) -> Option<&GateReason> {
        match self {
            Self::Full => None,
            Self::NativeOnly { reason } | Self::Unavailable { reason } => Some(reason),
        }
    }

    /// The banner text for the Helm views, `None` when everything is available. In native-only
    /// mode it also says what keeps working.
    pub fn guidance(&self) -> Option<String> {
        match self {
            Self::Full => None,
            Self::NativeOnly { reason } => Some(format!(
                "Native-only mode: releases, history, values, manifests and notes still work; \
                 install, upgrade, rollback, uninstall and the charts catalog are hidden. {}",
                reason.guidance()
            )),
            Self::Unavailable { reason } => Some(reason.guidance()),
        }
    }

    /// The capability flags this gate lets a session keep: [`Capabilities::HELM`] when the
    /// mutating features are available, nothing otherwise. A command that `needs` HELM is then
    /// unavailable in the palette, keymap and tools without a check of its own.
    pub fn capabilities(&self) -> Capabilities {
        match self {
            Self::Full => Capabilities::HELM,
            Self::NativeOnly { .. } | Self::Unavailable { .. } => Capabilities::empty(),
        }
    }

    /// `capabilities` with its [`HELM`](Capabilities::HELM) flag following this gate.
    pub fn apply_to(&self, capabilities: Capabilities) -> Capabilities {
        capabilities
            .difference(Capabilities::HELM)
            .union(self.capabilities())
    }

    /// The structured answer a tool returns for `feature`, `None` when it is available.
    pub fn unavailable(&self, feature: HelmFeature) -> Option<FeatureUnavailable> {
        if self.allows(feature) {
            return None;
        }
        let reason = self.reason()?.clone();
        Some(FeatureUnavailable {
            status: "unavailable",
            feature,
            message: reason.guidance(),
            reason,
        })
    }
}

/// The structured "unavailable" result of a Helm tool or command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeatureUnavailable {
    /// Always `"unavailable"`.
    pub status: &'static str,
    /// The feature that is not available.
    pub feature: HelmFeature,
    /// Why.
    pub reason: GateReason,
    /// How to fix it.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old() -> HelmCapabilities {
        HelmCapabilities {
            native_read: true,
            cli_version: Some(HelmVersion::new(3, 8, 0)),
            ..HelmCapabilities::none()
        }
    }

    #[test]
    fn a_supported_binary_is_full_and_allows_everything() {
        let gate = HelmGate::from_capabilities(&HelmCapabilities::full(HelmVersion::new(3, 14, 2)));
        assert_eq!(gate, HelmGate::Full);
        assert!(HelmFeature::ALL.iter().all(|f| gate.allows(*f)));
        assert_eq!(gate.guidance(), None);
        assert_eq!(gate.reason(), None);
        assert_eq!(gate.capabilities(), Capabilities::HELM);
        assert_eq!(gate.unavailable(HelmFeature::Install), None);
    }

    #[test]
    fn no_binary_with_a_native_reader_hides_exactly_the_cli_features() {
        let gate = HelmGate::from_capabilities(&HelmCapabilities::native_only());
        assert_eq!(
            gate,
            HelmGate::NativeOnly {
                reason: GateReason::BinaryMissing
            }
        );
        for feature in HelmFeature::ALL {
            assert_eq!(gate.allows(feature), !feature.needs_cli(), "{feature:?}");
        }
        for hidden in [
            HelmFeature::Install,
            HelmFeature::Upgrade,
            HelmFeature::Rollback,
            HelmFeature::Uninstall,
            HelmFeature::Repos,
            HelmFeature::Charts,
        ] {
            assert!(!gate.allows(hidden), "{hidden:?}");
        }
        assert!(gate.allows(HelmFeature::Releases) && gate.allows(HelmFeature::Notes));
        assert_eq!(gate.capabilities(), Capabilities::empty());
    }

    #[test]
    fn a_too_old_binary_reports_the_versions() {
        let gate = HelmGate::from_capabilities(&old());
        assert_eq!(
            gate.reason(),
            Some(&GateReason::VersionTooOld {
                found: "3.8.0".into(),
                needed: "3.10.0".into()
            })
        );
        let text = gate.guidance().unwrap();
        assert!(text.contains("3.8.0") && text.contains("3.10.0"), "{text}");
        assert!(text.contains("helm.path"), "{text}");
    }

    #[test]
    fn nothing_at_all_is_unavailable_and_nothing_detected_is_the_default() {
        let gate = HelmGate::from_capabilities(&HelmCapabilities::none());
        assert_eq!(
            gate,
            HelmGate::Unavailable {
                reason: GateReason::BinaryMissing
            }
        );
        assert_eq!(
            HelmGate::default(),
            HelmGate::Unavailable {
                reason: GateReason::NotConfigured
            }
        );
        assert!(HelmFeature::ALL.iter().all(|f| !gate.allows(*f)));
        let too_old_no_native = HelmGate::from_capabilities(&HelmCapabilities {
            native_read: false,
            ..old()
        });
        assert!(matches!(
            too_old_no_native,
            HelmGate::Unavailable {
                reason: GateReason::VersionTooOld { .. }
            }
        ));
    }

    #[test]
    fn every_reason_says_how_to_fix_it() {
        for reason in [
            GateReason::BinaryMissing,
            GateReason::NotConfigured,
            GateReason::VersionTooOld {
                found: "3.9.0".into(),
                needed: "3.10.0".into(),
            },
        ] {
            let text = reason.guidance();
            assert!(text.contains("Settings (helm.path)"), "{text}");
            assert!(text.to_lowercase().contains("helm"), "{text}");
        }
    }

    #[test]
    fn a_tool_gets_a_structured_unavailable_result() {
        let gate = HelmGate::from_capabilities(&HelmCapabilities::native_only());
        let result = gate.unavailable(HelmFeature::Rollback).expect("hidden");
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "unavailable");
        assert_eq!(json["feature"], "rollback");
        assert_eq!(json["reason"]["kind"], "binary_missing");
        assert!(json["message"].as_str().unwrap().contains("Install Helm"));
        assert_eq!(gate.unavailable(HelmFeature::Values), None);
    }

    #[test]
    fn apply_to_follows_the_gate_and_leaves_other_flags_alone() {
        let session = Capabilities::MUTATE | Capabilities::LOGS | Capabilities::HELM;
        let native = HelmGate::from_capabilities(&HelmCapabilities::native_only());
        assert_eq!(
            native.apply_to(session),
            Capabilities::MUTATE | Capabilities::LOGS
        );
        let full = HelmGate::Full;
        assert_eq!(
            full.apply_to(Capabilities::MUTATE),
            Capabilities::MUTATE | Capabilities::HELM
        );
    }
}
