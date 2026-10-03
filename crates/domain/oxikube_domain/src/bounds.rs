//! Size-bound helper shared by the record types in this crate.

/// Longest prefix of `s` that is at most `max_bytes` long and ends on a char boundary.
///
/// Never panics and never splits a UTF-8 sequence, for any input.
pub(crate) fn char_boundary_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    // A char boundary exists within 3 bytes below any index in a valid string.
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Cap `s` at `max_bytes` on a char boundary, in place. Returns whether it was cut.
pub(crate) fn truncate_in_place(s: &mut String, max_bytes: usize) -> bool {
    if s.len() <= max_bytes {
        return false;
    }
    let keep = char_boundary_prefix(s, max_bytes).len();
    s.truncate(keep);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_respects_boundaries() {
        assert_eq!(char_boundary_prefix("abc", 10), "abc");
        assert_eq!(char_boundary_prefix("abc", 2), "ab");
        // 'é' is two bytes; cutting inside it backs off to the previous boundary.
        assert_eq!(char_boundary_prefix("aé", 2), "a");
        assert_eq!(char_boundary_prefix("é", 0), "");
        assert_eq!(char_boundary_prefix("😀", 3), "");
    }

    #[test]
    fn truncate_reports_cut() {
        let mut s = String::from("héllo");
        assert!(truncate_in_place(&mut s, 2));
        assert_eq!(s, "h");
        assert!(!truncate_in_place(&mut s, 2));
    }
}
