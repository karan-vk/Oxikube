//! YAML number spellings as JSON numbers, keeping every digit.
//!
//! Integers beyond `u64` and floats with more digits than an `f64` holds stay exact because the
//! text is rewritten, never parsed to a machine number (only hex and octal integers are
//! converted, through `u128`).

use super::error::ConvertError;

/// A YAML integer (core schema: decimal, `0o` octal, `0x` hex) as the decimal text JSON wants.
/// `octal` is set when a leading-zero literal (`0644`) was read as octal, the way the API server
/// reads YAML (YAML 1.1), instead of as decimal 644.
pub(super) fn int_text(text: &str, octal: &mut bool) -> Result<String, ConvertError> {
    let (negative, digits) = split_sign(text);
    let magnitude = if let Some(rest) = digits.strip_prefix("0x") {
        radix(rest, 16, text)?
    } else if let Some(rest) = digits.strip_prefix("0o") {
        radix(rest, 8, text)?
    } else if digits.len() > 1 && digits.starts_with('0') && digits.bytes().all(|b| b <= b'7') {
        *octal = true;
        radix(&digits[1..], 8, text)?
    } else {
        let trimmed = digits.trim_start_matches('0');
        if trimmed.is_empty() { "0" } else { trimmed }.to_owned()
    };
    Ok(if negative && magnitude != "0" {
        format!("-{magnitude}")
    } else {
        magnitude
    })
}

fn radix(digits: &str, base: u32, whole: &str) -> Result<String, ConvertError> {
    u128::from_str_radix(digits, base)
        .map(|n| n.to_string())
        .map_err(|_| ConvertError::new(format!("the integer {whole} is too large for JSON")))
}

fn split_sign(text: &str) -> (bool, &str) {
    match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    }
}

/// A YAML float as a JSON number: `.5` becomes `0.5`, `5.` becomes `5.0`, `+1e3` loses its
/// plus. `None` for `.inf` and `.nan`, which JSON cannot hold.
pub(super) fn float_text(text: &str) -> Option<String> {
    let (negative, body) = split_sign(text);
    if body
        .strip_prefix('.')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_alphabetic()))
    {
        return None;
    }
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => (&body[..at], &body[at..]),
        None => (body, ""),
    };
    let mut out = String::with_capacity(text.len() + 2);
    if negative {
        out.push('-');
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    out.push_str(if whole.is_empty() { "0" } else { whole });
    if mantissa.contains('.') {
        out.push('.');
        out.push_str(if fraction.is_empty() { "0" } else { fraction });
    }
    out.push_str(exponent);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(text: &str) -> String {
        int_text(text, &mut false).unwrap()
    }

    #[test]
    fn integers_become_decimal_without_losing_digits() {
        assert_eq!(int("42"), "42");
        assert_eq!(int("+42"), "42");
        assert_eq!(int("-7"), "-7");
        assert_eq!(int("0"), "0");
        assert_eq!(int("-0"), "0");
        assert_eq!(int("0o17"), "15");
        assert_eq!(int("0x1F"), "31");
        assert_eq!(int("-0x10"), "-16");
        assert_eq!(
            int("123456789012345678901234567890"),
            "123456789012345678901234567890"
        );
    }

    #[test]
    fn a_leading_zero_is_octal_like_the_api_server_reads_it() {
        let mut octal = false;
        assert_eq!(int_text("0644", &mut octal).unwrap(), "420");
        assert!(octal);
        let mut octal = false;
        assert_eq!(int_text("089", &mut octal).unwrap(), "89");
        assert!(!octal);
    }

    #[test]
    fn huge_hex_is_refused() {
        let hex = format!("0x{}", "f".repeat(40));
        assert!(int_text(&hex, &mut false).is_err());
    }

    #[test]
    fn floats_become_json_numbers() {
        for (yaml, json) in [
            ("3.5", "3.5"),
            (".5", "0.5"),
            ("-.5", "-0.5"),
            ("5.", "5.0"),
            ("+1e3", "1e3"),
            ("1.5E-3", "1.5E-3"),
            ("1.e3", "1.0e3"),
            (
                "0.1000000000000000055511151231257827",
                "0.1000000000000000055511151231257827",
            ),
        ] {
            assert_eq!(float_text(yaml).as_deref(), Some(json), "{yaml}");
        }
        for special in [".inf", "-.Inf", "+.INF", ".nan", ".NaN"] {
            assert_eq!(float_text(special), None, "{special}");
        }
    }
}
