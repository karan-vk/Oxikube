//! "Did you mean": the nearest sibling name by edit distance.

/// The closest of `candidates` to `name`, if one is near enough to be a typo of it.
///
/// Distance is the optimal-string-alignment distance (insert, delete, substitute, swap two
/// neighbours) on lowercase text, so `Replicas` finds `replicas` and `contianers` finds
/// `containers`. The allowed distance grows with the name's length (1 up to 4 characters, 2 up
/// to 8, then 3) and is always below the name's length. Ties keep the first candidate.
pub(super) fn nearest<'a>(
    name: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    let wanted: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    let limit = match wanted.len() {
        0 => return None,
        1..=4 => 1,
        5..=8 => 2,
        _ => 3,
    }
    .min(wanted.len() - 1);
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        let other: Vec<char> = candidate.chars().flat_map(char::to_lowercase).collect();
        if other.len().abs_diff(wanted.len()) > limit {
            continue;
        }
        let distance = osa(&wanted, &other);
        // A case-only difference is distance 0: still a different name worth suggesting.
        if distance <= limit && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

fn osa(a: &[char], b: &[char]) -> usize {
    let width = b.len() + 1;
    let mut rows = vec![0usize; 3 * width];
    // rows: previous-previous, previous, current.
    let (mut pp, mut p, mut c) = (0, width, 2 * width);
    for (j, slot) in rows[p..p + width].iter_mut().enumerate() {
        *slot = j;
    }
    for i in 1..=a.len() {
        rows[c] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (rows[p + j] + 1)
                .min(rows[c + j - 1] + 1)
                .min(rows[p + j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(rows[pp + j - 2] + 1);
            }
            rows[c + j] = best;
        }
        (pp, p, c) = (p, c, pp);
    }
    rows[p + b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near<'a>(name: &str, of: &[&'a str]) -> Option<&'a str> {
        nearest(name, of.iter().copied())
    }

    #[test]
    fn finds_typos() {
        let props = ["containers", "restartPolicy", "replicas", "selector"];
        assert_eq!(near("replica", &props), Some("replicas"));
        assert_eq!(near("contianers", &props), Some("containers"));
        assert_eq!(near("Replicas", &props), Some("replicas"));
        assert_eq!(near("restartPolicy", &props), Some("restartPolicy"));
        assert_eq!(near("selectr", &props), Some("selector"));
    }

    #[test]
    fn ignores_unrelated_names() {
        let props = ["containers", "restartPolicy"];
        assert_eq!(near("zzz", &props), None);
        assert_eq!(near("x", &["y"]), None, "one-letter names never match");
        assert_eq!(near("", &props), None);
        assert_eq!(near("foo", &["bar", "baz"]), None);
    }

    #[test]
    fn prefers_the_closest() {
        assert_eq!(near("imagess", &["image", "images"]), Some("images"));
    }
}
