//! Typed input against the schema, before anything is written.

use serde_json::{Value, json};

use crate::controls::{ControlSpec, NumberBounds, validate_list, validate_text};

fn number(integer: bool, min: Option<f64>, max: Option<f64>) -> ControlSpec {
    ControlSpec::Number(NumberBounds { integer, min, max })
}

#[test]
fn blank_text_resets() {
    assert_eq!(validate_text(&ControlSpec::Text, "   "), Ok(Value::Null));
    assert_eq!(
        validate_text(&number(true, None, None), ""),
        Ok(Value::Null)
    );
}

#[test]
fn numbers_are_parsed_and_bounded() {
    let bounded = number(false, Some(6.0), Some(72.0));
    assert_eq!(validate_text(&bounded, " 14 "), Ok(json!(14.0)));
    assert_eq!(validate_text(&bounded, "12.5"), Ok(json!(12.5)));
    let low = validate_text(&bounded, "3").expect_err("below the minimum");
    assert_eq!(low.0, "Must be at least 6.");
    let high = validate_text(&bounded, "99").expect_err("above the maximum");
    assert_eq!(high.0, "Must be at most 72.");
    assert!(validate_text(&bounded, "big").is_err());
    assert!(validate_text(&bounded, "NaN").is_err());
    assert!(validate_text(&bounded, "inf").is_err());
}

#[test]
fn integers_reject_fractions() {
    let whole = number(true, Some(0.0), None);
    assert_eq!(validate_text(&whole, "10000"), Ok(json!(10000)));
    assert_eq!(
        validate_text(&whole, "1.5").expect_err("not whole").0,
        "\"1.5\" is not a whole number."
    );
    assert!(validate_text(&whole, "-1").is_err());
}

#[test]
fn choices_must_be_members() {
    let spec = ControlSpec::Choice(vec!["block".into(), "bar".into()]);
    assert_eq!(validate_text(&spec, "bar"), Ok(json!("bar")));
    assert_eq!(
        validate_text(&spec, "dot").expect_err("not a choice").0,
        "Expected one of: block, bar."
    );
}

#[test]
fn colours_are_normalised_and_names_kept() {
    assert_eq!(
        validate_text(&ControlSpec::Color, "#E5484D"),
        Ok(json!("#e5484d"))
    );
    assert_eq!(
        validate_text(&ControlSpec::Color, "#fff"),
        Ok(json!("#ffffff"))
    );
    assert_eq!(validate_text(&ControlSpec::Color, "Red"), Ok(json!("red")));
    assert!(validate_text(&ControlSpec::Color, "#12").is_err());
    assert!(validate_text(&ControlSpec::Color, "teal").is_err());
}

#[test]
fn lists_reject_blank_and_repeated_items() {
    let items = |texts: &[&str]| texts.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        validate_list(&items(&[" bash ", "sh"])),
        Ok(json!(["bash", "sh"]))
    );
    assert_eq!(validate_list(&[]), Ok(Value::Null));
    assert_eq!(
        validate_list(&items(&["bash", ""])).expect_err("blank").0,
        "Item 2 is empty."
    );
    assert_eq!(
        validate_list(&items(&["sh", "sh"])).expect_err("repeat").0,
        "\"sh\" appears twice."
    );
}

#[test]
fn switches_and_json_are_not_text() {
    assert!(validate_text(&ControlSpec::Switch, "true").is_err());
    assert!(validate_text(&ControlSpec::Json, "{}").is_err());
}
