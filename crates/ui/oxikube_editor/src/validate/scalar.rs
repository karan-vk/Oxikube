//! Typing of scalars by the YAML 1.2 core schema, and comparison with JSON enum values.
//!
//! The spanned model keeps only text and notation; what a plain scalar *is* (`3` an integer,
//! `"3"` a string, `yes` a string) is decided here, the way the API server sees the manifest.

use oxikube_domain::schema::SchemaType;

use crate::yaml::ScalarStyle;

/// The type of a YAML value as the validator sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ValueType {
    /// `null`, `~` or an empty value.
    Null,
    /// `true` / `false` (YAML 1.2 spellings only).
    Bool,
    /// A YAML integer (decimal, `0o` octal, `0x` hex).
    Int,
    /// A YAML float, including `.inf` and `.nan`.
    Float,
    /// Any other plain scalar, and every quoted or block scalar.
    String,
    /// A mapping.
    Object,
    /// A sequence.
    Array,
}

impl ValueType {
    /// The name used in messages.
    pub(super) fn name(self) -> &'static str {
        match self {
            ValueType::Null => "null",
            ValueType::Bool => "boolean",
            ValueType::Int => "integer",
            ValueType::Float => "number",
            ValueType::String => "string",
            ValueType::Object => "object",
            ValueType::Array => "array",
        }
    }

    /// Whether a value of this type satisfies the schema type `wanted`. Every number is a
    /// `number`; a float with no fractional part (`3.0`, `1e3`) is an `integer`, as the server
    /// reads it after the YAML to JSON conversion.
    pub(super) fn satisfies(self, wanted: SchemaType, text: &str) -> bool {
        match (self, wanted) {
            (ValueType::Null, SchemaType::Null)
            | (ValueType::Bool, SchemaType::Boolean)
            | (ValueType::Int, SchemaType::Integer | SchemaType::Number)
            | (ValueType::Float, SchemaType::Number)
            | (ValueType::String, SchemaType::String)
            | (ValueType::Object, SchemaType::Object)
            | (ValueType::Array, SchemaType::Array) => true,
            (ValueType::Float, SchemaType::Integer) => is_whole_float(text),
            _ => false,
        }
    }
}

/// The type of a scalar with this notation and decoded text.
pub(super) fn scalar_type(style: ScalarStyle, text: &str) -> ValueType {
    match style {
        ScalarStyle::Plain => plain_type(text),
        ScalarStyle::SingleQuoted
        | ScalarStyle::DoubleQuoted
        | ScalarStyle::Literal
        | ScalarStyle::Folded => ValueType::String,
    }
}

fn plain_type(text: &str) -> ValueType {
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return ValueType::Null,
        "true" | "True" | "TRUE" | "false" | "False" | "FALSE" => return ValueType::Bool,
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" | "-.inf" | "-.Inf" | "-.INF"
        | ".nan" | ".NaN" | ".NAN" => return ValueType::Float,
        _ => {}
    }
    let bytes = text.as_bytes();
    // The first byte decides most strings (names, images) without scanning further.
    if !matches!(bytes[0], b'0'..=b'9' | b'+' | b'-' | b'.') {
        return ValueType::String;
    }
    if is_int(text) {
        ValueType::Int
    } else if is_float(bytes) {
        ValueType::Float
    } else {
        ValueType::String
    }
}

fn is_int(text: &str) -> bool {
    if let Some(octal) = text.strip_prefix("0o") {
        return !octal.is_empty() && octal.bytes().all(|b| matches!(b, b'0'..=b'7'));
    }
    if let Some(hex) = text.strip_prefix("0x") {
        return !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// `[-+]? ( \. [0-9]+ | [0-9]+ ( \. [0-9]* )? ) ( [eE] [-+]? [0-9]+ )?`
fn is_float(bytes: &[u8]) -> bool {
    let mut i = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let int_digits = digits(i);
    i += int_digits;
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let frac = digits(i);
        i += frac;
        if int_digits == 0 && frac == 0 {
            return false;
        }
    } else if int_digits == 0 {
        return false;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exp = digits(i);
        if exp == 0 {
            return false;
        }
        i += exp;
    }
    i == bytes.len()
}

fn is_whole_float(text: &str) -> bool {
    text.parse::<f64>()
        .is_ok_and(|v| v.is_finite() && v.fract() == 0.0)
}

/// Whether the typed scalar equals the JSON enum value.
pub(super) fn matches_enum(ty: ValueType, text: &str, allowed: &serde_json::Value) -> bool {
    match ty {
        ValueType::String => allowed.as_str() == Some(text),
        ValueType::Bool => allowed.as_bool() == Some(text.eq_ignore_ascii_case("true")),
        // `0x10` and `0o7` do not parse as f64: a hex or octal enum value is not compared.
        ValueType::Int | ValueType::Float => allowed
            .as_f64()
            .zip(text.parse::<f64>().ok())
            .is_some_and(|(want, got)| want == got),
        ValueType::Null => allowed.is_null(),
        ValueType::Object | ValueType::Array => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> ValueType {
        scalar_type(ScalarStyle::Plain, text)
    }

    #[test]
    fn core_schema_typing() {
        for (text, want) in [
            ("3", ValueType::Int),
            ("-3", ValueType::Int),
            ("+3", ValueType::Int),
            ("0o17", ValueType::Int),
            ("0x1F", ValueType::Int),
            ("3.5", ValueType::Float),
            (".5", ValueType::Float),
            ("5.", ValueType::Float),
            ("1e3", ValueType::Float),
            ("1.5E-3", ValueType::Float),
            (".inf", ValueType::Float),
            (".NaN", ValueType::Float),
            ("true", ValueType::Bool),
            ("False", ValueType::Bool),
            ("yes", ValueType::String),
            ("on", ValueType::String),
            ("y", ValueType::String),
            ("null", ValueType::Null),
            ("~", ValueType::Null),
            ("", ValueType::Null),
            ("500m", ValueType::String),
            ("1Gi", ValueType::String),
            ("5s", ValueType::String),
            ("1.27.3", ValueType::String),
            ("-", ValueType::String),
            (".", ValueType::String),
            ("e3", ValueType::String),
            ("1e", ValueType::String),
            ("0o8", ValueType::String),
            ("nginx:1.27", ValueType::String),
        ] {
            assert_eq!(plain(text), want, "{text:?}");
        }
    }

    #[test]
    fn quoted_and_block_scalars_are_strings() {
        for style in [
            ScalarStyle::SingleQuoted,
            ScalarStyle::DoubleQuoted,
            ScalarStyle::Literal,
            ScalarStyle::Folded,
        ] {
            assert_eq!(scalar_type(style, "3"), ValueType::String);
            assert_eq!(scalar_type(style, "true"), ValueType::String);
            assert_eq!(scalar_type(style, ""), ValueType::String);
        }
    }

    #[test]
    fn whole_floats_are_integers() {
        assert!(ValueType::Float.satisfies(SchemaType::Integer, "3.0"));
        assert!(ValueType::Float.satisfies(SchemaType::Integer, "1e3"));
        assert!(!ValueType::Float.satisfies(SchemaType::Integer, "3.5"));
        assert!(!ValueType::Float.satisfies(SchemaType::Integer, ".inf"));
        assert!(ValueType::Int.satisfies(SchemaType::Number, "3"));
        assert!(!ValueType::Int.satisfies(SchemaType::String, "3"));
    }

    #[test]
    fn enum_comparison_is_typed() {
        use serde_json::json;
        assert!(matches_enum(ValueType::String, "TCP", &json!("TCP")));
        assert!(!matches_enum(ValueType::String, "tcp", &json!("TCP")));
        assert!(matches_enum(ValueType::Int, "2", &json!(2)));
        assert!(matches_enum(ValueType::Bool, "True", &json!(true)));
        assert!(!matches_enum(ValueType::String, "2", &json!(2)));
    }
}
