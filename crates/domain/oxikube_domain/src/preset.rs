//! [`ClusterPreset`]: the named safety postures a user can give a cluster in one step.
//!
//! A preset is a shortcut, not a stored state: applying one writes the same two settings
//! fields the user could edit by hand (`clusters.<id>.colour` and, for production,
//! `clusters.<id>.read_only`). Nothing records *which* preset was applied; the app derives it
//! from the fields with [`ClusterPreset::detect`], so editing `settings.json` and using the
//! menu can never disagree.

use serde::{Deserialize, Serialize};

use crate::colour::ClusterColour;

/// A named colour (and, for production, read-only) posture.
///
/// | Preset | Colour | Read-only |
/// |---|---|---|
/// | [`Prod`](Self::Prod) | red `#e5484d` | turned on |
/// | [`Staging`](Self::Staging) | amber `#f5a524` | left as it is |
/// | [`Dev`](Self::Dev) | green `#30a46c` | left as it is |
/// | [`None`](Self::None) | cleared | left as it is |
///
/// No preset ever lowers protection, so applying one is safe for every initiator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterPreset {
    /// Production: red, read-only on.
    Prod,
    /// Staging: amber.
    Staging,
    /// Development: green.
    Dev,
    /// No colour.
    None,
}

impl ClusterPreset {
    /// Every preset, in menu order.
    pub const ALL: [ClusterPreset; 4] = [Self::Prod, Self::Staging, Self::Dev, Self::None];

    /// The red of [`Prod`](Self::Prod).
    pub const PROD_COLOUR: ClusterColour = ClusterColour::rgb(0xe5, 0x48, 0x4d);
    /// The amber of [`Staging`](Self::Staging).
    pub const STAGING_COLOUR: ClusterColour = ClusterColour::rgb(0xf5, 0xa5, 0x24);
    /// The green of [`Dev`](Self::Dev).
    pub const DEV_COLOUR: ClusterColour = ClusterColour::rgb(0x30, 0xa4, 0x6c);

    /// The label shown in menus.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Prod => "Production",
            Self::Staging => "Staging",
            Self::Dev => "Development",
            Self::None => "No colour",
        }
    }

    /// The colour this preset writes; `None` clears the colour.
    pub const fn colour(self) -> Option<ClusterColour> {
        match self {
            Self::Prod => Some(Self::PROD_COLOUR),
            Self::Staging => Some(Self::STAGING_COLOUR),
            Self::Dev => Some(Self::DEV_COLOUR),
            Self::None => Option::None,
        }
    }

    /// The read-only value this preset writes, or `None` when it leaves the flag alone.
    /// Only [`Prod`](Self::Prod) writes one (`true`).
    pub const fn read_only(self) -> Option<bool> {
        match self {
            Self::Prod => Some(true),
            Self::Staging | Self::Dev | Self::None => Option::None,
        }
    }

    /// The preset whose colour is `colour`: what the cluster is "flagged" as.
    pub fn detect(colour: Option<ClusterColour>) -> Self {
        match colour {
            Some(c) if c == Self::PROD_COLOUR => Self::Prod,
            Some(c) if c == Self::STAGING_COLOUR => Self::Staging,
            Some(c) if c == Self::DEV_COLOUR => Self::Dev,
            _ => Self::None,
        }
    }

    /// Suggests a preset for a kubeconfig context name, to offer (never apply) when a new
    /// cluster is connected. A name containing `prod` (as a word part, any case) suggests
    /// [`Prod`](Self::Prod); `stag`/`stage`/`uat` suggest [`Staging`](Self::Staging);
    /// `dev`/`local`/`kind-`/`minikube` suggest [`Dev`](Self::Dev). Anything else: `None`.
    pub fn suggest(context_name: &str) -> Option<Self> {
        let name = context_name.to_ascii_lowercase();
        let has = |needles: &[&str]| needles.iter().any(|n| name.contains(n));
        if has(&["prod", "prd"]) {
            Some(Self::Prod)
        } else if has(&["staging", "stage", "stg", "uat"]) {
            Some(Self::Staging)
        } else if has(&["dev", "local", "kind-", "minikube", "sandbox"]) {
            Some(Self::Dev)
        } else {
            Option::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prod_is_red_and_read_only_the_others_leave_the_flag() {
        assert_eq!(ClusterPreset::Prod.colour().unwrap().to_string(), "#e5484d");
        assert_eq!(ClusterPreset::Prod.read_only(), Some(true));
        for preset in [
            ClusterPreset::Staging,
            ClusterPreset::Dev,
            ClusterPreset::None,
        ] {
            assert_eq!(preset.read_only(), None, "{preset:?} must not touch it");
        }
        assert_eq!(ClusterPreset::None.colour(), None);
    }

    #[test]
    fn detect_inverts_colour_for_every_preset() {
        for preset in ClusterPreset::ALL {
            assert_eq!(ClusterPreset::detect(preset.colour()), preset);
        }
        assert_eq!(
            ClusterPreset::detect(Some(ClusterColour::rgb(1, 2, 3))),
            ClusterPreset::None
        );
    }

    #[test]
    fn suggestions_follow_the_context_name_and_prefer_the_safest() {
        let cases = [
            ("prod-eu-1", Some(ClusterPreset::Prod)),
            ("arn:aws:eks:eu-west-1:1:cluster/PROD", Some(ClusterPreset::Prod)),
            ("gke_acme_europe_staging", Some(ClusterPreset::Staging)),
            ("kind-oxikube", Some(ClusterPreset::Dev)),
            ("minikube", Some(ClusterPreset::Dev)),
            // `prod` wins over `dev` when both appear: suggest the safer posture.
            ("dev-prod-mirror", Some(ClusterPreset::Prod)),
            ("payments", None),
        ];
        for (name, want) in cases {
            assert_eq!(ClusterPreset::suggest(name), want, "{name}");
        }
    }

    #[test]
    fn serde_uses_snake_case_names() {
        assert_eq!(serde_json::to_string(&ClusterPreset::Prod).unwrap(), "\"prod\"");
        assert_eq!(
            serde_json::from_str::<ClusterPreset>("\"none\"").unwrap(),
            ClusterPreset::None
        );
        assert!(serde_json::from_str::<ClusterPreset>("\"red\"").is_err());
    }
}
