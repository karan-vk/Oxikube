//! The cluster colour on screen, and the initials of a cluster's name.

use gpui::{Hsla, rgb};
use oxikube_domain::ClusterColour;

/// The GPUI colour of a cluster's accent (`#rrggbb`, opaque).
pub fn cluster_hsla(colour: ClusterColour) -> Hsla {
    rgb((u32::from(colour.r) << 16) | (u32::from(colour.g) << 8) | u32::from(colour.b)).into()
}

/// Up to two letters that stand for a cluster in a narrow place (the hotbar): the first letters
/// of its first two words (words split at `-`, `_`, `.`, `/`, `:`, `@` and spaces), upper-cased;
/// a single word gives its first two letters. `prod-eu-1` is `PE`, `minikube` is `MI`, an empty
/// name is `?`.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let letters: Vec<char> = match words.as_slice() {
        [] => return "?".to_owned(),
        [only] => only.chars().take(2).collect(),
        [first, second, ..] => first
            .chars()
            .take(1)
            .chain(second.chars().take(1))
            .collect(),
    };
    letters.iter().flat_map(|c| c.to_uppercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_of_names() {
        assert_eq!(initials("prod-eu-1"), "PE");
        assert_eq!(initials("minikube"), "MI");
        assert_eq!(initials("kind-oxikube"), "KO");
        assert_eq!(initials("gke_proj_europe-west1_main"), "GP");
        assert_eq!(initials("x"), "X");
        assert_eq!(initials("arn:aws:eks:us-east-1:1:cluster/a"), "AA");
        assert_eq!(initials("  "), "?");
        assert_eq!(initials(""), "?");
        assert_eq!(initials("épreuve"), "ÉP");
    }

    #[test]
    fn colours_convert_exactly() {
        let c = cluster_hsla(ClusterColour::rgb(0xe5, 0x39, 0x35));
        let back = gpui::Rgba::from(c);
        assert!((back.r - 0xe5 as f32 / 255.).abs() < 1e-3);
        assert!((back.g - 0x39 as f32 / 255.).abs() < 1e-3);
        assert!((back.b - 0x35 as f32 / 255.).abs() < 1e-3);
        assert_eq!(back.a, 1.0);
    }

    #[test]
    fn distinct_colours_stay_distinct() {
        assert_ne!(
            cluster_hsla(ClusterColour::rgb(255, 0, 0)),
            cluster_hsla(ClusterColour::rgb(0, 0, 255))
        );
    }
}
