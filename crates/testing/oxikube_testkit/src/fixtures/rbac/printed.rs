//! [`printed_rules`]: the rows `kubectl auth can-i --list` prints, read back as rules.

use oxikube_domain::rbac::PolicyRule;

/// One row of `kubectl auth can-i --list` as a rule: `resource.group/sub`, `[urls]`,
/// `[names]`, `[verbs]`. A row with an empty first column is a non-resource rule.
///
/// # Panics
///
/// When `line` is not such a row (no bracketed lists, or not three of them).
fn printed_rule(line: &str) -> PolicyRule {
    let column = line[..line.find('[').expect("a printed row has lists")].trim();
    let lists: Vec<Vec<&str>> = line
        .split('[')
        .skip(1)
        .map(|part| {
            part.split(']')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect()
        })
        .collect();
    let [urls, names, verbs] = lists.as_slice() else {
        panic!("three lists in {line:?}");
    };
    if column.is_empty() {
        return PolicyRule::non_resource(verbs, urls);
    }
    let (head, subresource) = column
        .split_once('/')
        .map_or((column, None), |(h, s)| (h, Some(s)));
    let (resource, group) = head.split_once('.').unwrap_or((head, ""));
    let resource =
        subresource.map_or_else(|| resource.to_owned(), |sub| format!("{resource}/{sub}"));
    PolicyRule::resource(verbs, &[group], &[&resource], names)
}

/// Every row of `printed` (the header already dropped) as a rule. Blank lines are skipped.
///
/// # Panics
///
/// When a row is not one `kubectl auth can-i --list` prints (see [`printed_rule`]).
pub fn printed_rules<S: AsRef<str>>(printed: &[S]) -> Vec<PolicyRule> {
    printed
        .iter()
        .map(AsRef::as_ref)
        .filter(|line| !line.trim().is_empty())
        .map(printed_rule)
        .collect()
}
