use oxikube_domain::ErrorKind;
use oxikube_ports::{ListOptions, VersionMatch};

use crate::resources::params::list_params;

#[test]
fn selectors_limit_and_timeout_map_through() {
    let p = list_params(
        &ListOptions::default()
            .labels("app=web")
            .fields("spec.nodeName=n1")
            .limit(250)
            .timeout_secs(30),
    )
    .unwrap();
    assert_eq!(p.label_selector.as_deref(), Some("app=web"));
    assert_eq!(p.field_selector.as_deref(), Some("spec.nodeName=n1"));
    assert_eq!(p.limit, Some(250));
    assert_eq!(p.timeout, Some(30));
}

#[test]
fn empty_strings_and_zero_limit_mean_unset() {
    let mut options = ListOptions::default().labels("").fields("").limit(0);
    options.continue_token = Some(String::new());
    let p = list_params(&options).unwrap();
    assert_eq!(p.label_selector, None);
    assert_eq!(p.field_selector, None);
    assert_eq!(p.limit, None);
    assert_eq!(p.continue_token, None);
}

#[test]
fn resource_version_and_match_pass_through() {
    let p = list_params(&ListOptions::default().at("42", VersionMatch::Exact)).unwrap();
    assert_eq!(p.resource_version.as_deref(), Some("42"));
    assert!(p.version_match.is_some());
    let any = list_params(&{
        let mut o = ListOptions::default();
        o.resource_version = Some("0".into());
        o
    })
    .unwrap();
    assert_eq!(any.resource_version.as_deref(), Some("0"));
    assert!(any.version_match.is_none());
}

#[test]
fn a_continue_token_drops_the_resource_version() {
    let options = ListOptions::default()
        .at("42", VersionMatch::NotOlderThan)
        .continue_from("tok");
    let p = list_params(&options).unwrap();
    assert_eq!(p.continue_token.as_deref(), Some("tok"));
    assert_eq!(p.resource_version, None);
    assert!(p.version_match.is_none());
}

#[test]
fn invalid_version_combinations_are_validation_errors() {
    let mut no_rv = ListOptions::default();
    no_rv.version_match = Some(VersionMatch::NotOlderThan);
    assert_eq!(
        list_params(&no_rv).unwrap_err().kind(),
        ErrorKind::Validation
    );

    let exact_zero = ListOptions::default().at("0", VersionMatch::Exact);
    assert_eq!(
        list_params(&exact_zero).unwrap_err().kind(),
        ErrorKind::Validation
    );
}
