//! Checking what a person typed against the field's schema before anything is written.
//!
//! Pure functions: the text goes in, the JSON value to write (or the message to show under the
//! control) comes out. Invalid input never reaches the file.

use oxikube_domain::ClusterColour;
use serde_json::{Number, Value};

use super::spec::{ControlSpec, NumberBounds};

/// What to show under a control whose input is not acceptable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldError(pub String);

impl std::fmt::Display for FieldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The JSON value for the text `input` typed into a control of kind `spec`.
///
/// Blank text means "no value" and gives [`Value::Null`], which resets the field. Switches and
/// lists do not take text; they are checked by [`validate_list`].
///
/// # Errors
///
/// A [`FieldError`] saying what is wrong: not a number, outside the bounds, not one of the
/// choices, not a colour.
pub fn validate_text(spec: &ControlSpec, input: &str) -> Result<Value, FieldError> {
    let text = input.trim();
    if text.is_empty() {
        return Ok(Value::Null);
    }
    match spec {
        ControlSpec::Number(bounds) => validate_number(*bounds, text),
        ControlSpec::Choice(values) => values
            .iter()
            .any(|value| value.as_ref() == text)
            .then(|| Value::String(text.to_owned()))
            .ok_or_else(|| {
                let allowed: Vec<&str> = values.iter().map(AsRef::as_ref).collect();
                FieldError(format!("Expected one of: {}.", allowed.join(", ")))
            }),
        ControlSpec::Color => ClusterColour::parse(text)
            .map(|colour| Value::String(text_of_colour(colour, text)))
            .map_err(|_| {
                FieldError(
                    "Expected a colour: #rrggbb, #rgb or red, orange, green, blue, purple, gray."
                        .to_owned(),
                )
            }),
        ControlSpec::Text | ControlSpec::Font => Ok(Value::String(text.to_owned())),
        ControlSpec::Switch | ControlSpec::List | ControlSpec::Json => {
            Err(FieldError("This setting is not edited as text.".to_owned()))
        }
    }
}

/// A palette name stays a name; hex is written normalised (`#rrggbb`, lowercase).
fn text_of_colour(colour: ClusterColour, typed: &str) -> String {
    if typed.starts_with('#') {
        colour.to_string()
    } else {
        typed.to_ascii_lowercase()
    }
}

fn validate_number(bounds: NumberBounds, text: &str) -> Result<Value, FieldError> {
    let whole = if bounds.integer {
        text.parse::<i64>()
            .map_err(|_| FieldError(format!("\"{text}\" is not a whole number.")))
            .map(Some)?
    } else {
        None
    };
    let parsed = match whole {
        Some(whole) => whole as f64,
        None => text
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .ok_or_else(|| FieldError(format!("\"{text}\" is not a number.")))?,
    };
    match (bounds.min, bounds.max) {
        (Some(min), _) if parsed < min => {
            return Err(FieldError(format!(
                "Must be at least {}.",
                trim_number(min)
            )));
        }
        (_, Some(max)) if parsed > max => {
            return Err(FieldError(format!("Must be at most {}.", trim_number(max))));
        }
        _ => {}
    }
    let number = if let Some(whole) = whole {
        Number::from(whole)
    } else {
        Number::from_f64(parsed).ok_or_else(|| FieldError("Not a finite number.".to_owned()))?
    };
    Ok(Value::Number(number))
}

/// `3.0` as `3`, `0.5` as `0.5`.
fn trim_number(number: f64) -> String {
    if number.fract() == 0.0 {
        format!("{number:.0}")
    } else {
        number.to_string()
    }
}

/// The JSON array for the items of a list editor. Items are trimmed; an empty list is written as
/// `[]` (it is a meaningful value, for example "notify about nothing"; the reset button restores
/// the default).
///
/// # Errors
///
/// A [`FieldError`] when an item is blank or two items are equal.
pub fn validate_list(items: &[String]) -> Result<Value, FieldError> {
    let mut seen: Vec<&str> = Vec::with_capacity(items.len());
    let mut values = Vec::with_capacity(items.len());
    for (ix, item) in items.iter().enumerate() {
        let item = item.trim();
        if item.is_empty() {
            return Err(FieldError(format!("Item {} is empty.", ix + 1)));
        }
        if seen.contains(&item) {
            return Err(FieldError(format!("\"{item}\" appears twice.")));
        }
        seen.push(item);
        values.push(Value::String(item.to_owned()));
    }
    Ok(Value::Array(values))
}
