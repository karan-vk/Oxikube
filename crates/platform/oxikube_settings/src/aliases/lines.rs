//! Where each key of a JSON-with-comments object is, so a problem with an entry can name its
//! line. `serde_json` parses to values without positions, and the text has already been parsed
//! once successfully when this runs, so a small scanner is enough: it tracks nesting, skips
//! strings and both comment styles, and notes the string that starts each member of the root
//! object.

/// The keys of the root object of `text` in file order, each with the 1-based line it starts on.
/// A key can appear twice; both are listed. Meant for text that parsed as an object; anything
/// else yields what could be found.
pub(super) fn top_level_keys(text: &str) -> Vec<(String, usize)> {
    let bytes = text.as_bytes();
    let mut keys = Vec::new();
    let mut line = 1;
    let mut depth = 0usize;
    let mut expecting_key = false;
    // Whether the root value is an object (commas of a root array do not announce keys).
    let mut root_is_object = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => line += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                i += 2;
                continue;
            }
            b'"' => {
                let start = i;
                let start_line = line;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    match bytes[i] {
                        b'\\' => i += 1,
                        b'\n' => line += 1,
                        _ => {}
                    }
                    i += 1;
                }
                if depth == 1
                    && expecting_key
                    && let Some(token) = text.get(start..=i.min(bytes.len() - 1))
                    && let Ok(key) = serde_json::from_str::<String>(token)
                {
                    keys.push((key, start_line));
                    expecting_key = false;
                }
            }
            b'{' => {
                depth += 1;
                if depth == 1 {
                    root_is_object = true;
                    expecting_key = true;
                }
            }
            b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 1 && root_is_object => expecting_key = true,
            _ => {}
        }
        i += 1;
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_each_key_with_its_line_through_comments_and_nesting() {
        let text = "// header { \"fake\": 1 }\n{\n  \"a\": \"x\", /* \"no\": 1,\n still comment */\n  \"b\": {\"c\": [1, {\"d\": 2}]},\n\n  \"e\\\"q\": \"v,\" ,\n  \"f\": 1,\n}\n";
        let keys = top_level_keys(text);
        assert_eq!(
            keys,
            [
                ("a".to_owned(), 3),
                ("b".to_owned(), 5),
                ("e\"q".to_owned(), 7),
                ("f".to_owned(), 8)
            ]
        );
    }

    #[test]
    fn a_repeated_key_is_listed_twice() {
        let keys = top_level_keys("{\"a\": 1,\n\"a\": 2}");
        assert_eq!(keys, [("a".to_owned(), 1), ("a".to_owned(), 2)]);
    }

    #[test]
    fn string_values_are_not_keys() {
        let keys = top_level_keys("{\"a\": \"b\", \"c\": \"d\"}");
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[1].0, "c");
    }

    #[test]
    fn text_that_is_not_an_object_finds_nothing() {
        assert!(top_level_keys("[\"a\", \"b\"]").is_empty());
        assert!(top_level_keys("").is_empty());
        assert!(top_level_keys("\"unterminated").is_empty());
    }
}
