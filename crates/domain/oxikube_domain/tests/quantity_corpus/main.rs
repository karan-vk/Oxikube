//! Ports the apimachinery `quantity_test.go` tables (see `apimachinery.rs` for provenance) and
//! checks [`Quantity`] against them: parsed value, format and `String()` text.
//!
//! Rows listed in [`DIVERGENCES`] are intentional differences from Go and are checked against
//! Oxikube's own expectation instead.

mod apimachinery;

use apimachinery::{B, D, Row, X};
use oxikube_domain::{Quantity, QuantityFormat};

/// Why a row differs from Go.
const NO_SATURATION: &str = "no int64 saturation: Go clamps BinarySI above 2^63-1 \
    (apimachinery TODO #141166); Oxikube keeps the exact value";
const EXPONENT_PAST_EXA: &str = "Go prints DecimalSI values past 10^18 without a suffix \
    (`1000E` becomes `1`, a different value); Oxikube falls back to an exponent";

/// Intentional differences from Go: Oxikube's `(input, mantissa, exponent, format, text)` and
/// the reason. Every other corpus row must match Go exactly.
const DIVERGENCES: &[(Row, &str)] = &[
    (("9Ei", 9 << 60, 0, B, "9Ei"), NO_SATURATION),
    (("-9Ei", -(9 << 60), 0, B, "-9Ei"), NO_SATURATION),
    (("+9Ei", 9 << 60, 0, B, "9Ei"), NO_SATURATION),
    (
        (
            "9223372036854775807Ki",
            (i64::MAX as i128) << 10,
            0,
            B,
            "9223372036854775807Ki",
        ),
        NO_SATURATION,
    ),
    (
        (
            "-9223372036854775807Ki",
            -((i64::MAX as i128) << 10),
            0,
            B,
            "-9223372036854775807Ki",
        ),
        NO_SATURATION,
    ),
    (
        (
            "+9223372036854775807Ki",
            (i64::MAX as i128) << 10,
            0,
            B,
            "9223372036854775807Ki",
        ),
        NO_SATURATION,
    ),
    (("10Ei", 1 << 60, 1, B, "10Ei"), NO_SATURATION),
    (("100Ei", 1 << 60, 2, B, "100Ei"), NO_SATURATION),
    (("1024Ei", 1 << 70, 0, B, "1024Ei"), NO_SATURATION),
    (("1000E", 1, 21, D, "1e21"), EXPONENT_PAST_EXA),
    (("5000E", 5, 21, D, "5e21"), EXPONENT_PAST_EXA),
    (("1000000E", 1, 24, D, "1e24"), EXPONENT_PAST_EXA),
    (("-1000E", -1, 21, D, "-1e21"), EXPONENT_PAST_EXA),
    (
        (
            "100000000000000000000000000000000000000000000",
            1,
            44,
            D,
            "100e42",
        ),
        EXPONENT_PAST_EXA,
    ),
];

fn parse(table: &str, input: &str) -> Quantity {
    Quantity::parse(input).unwrap_or_else(|e| panic!("{table} {input:?}: {e}"))
}

/// Checks one parsed row (Go's, or Oxikube's override from [`DIVERGENCES`]).
fn check(table: &str, go_row: &Row) {
    let row = DIVERGENCES
        .iter()
        .find(|(row, _)| row.0 == go_row.0)
        .map_or(go_row, |(row, _)| row);
    let (input, mantissa, exponent, format, text) = *row;
    let q = parse(table, input);
    assert_eq!(
        (q.mantissa(), q.exponent()),
        (mantissa, exponent),
        "{table} {input:?}: value"
    );
    assert_eq!(q.format(), format, "{table} {input:?}: format");
    assert_eq!(q.to_string(), text, "{table} {input:?}: String()");
    // The printed text always reads back as the same value.
    assert_eq!(parse(table, text), q, "{table} {input:?}: round trip");
}

fn check_all(table: &str, rows: &[Row]) {
    assert!(!rows.is_empty(), "{table} is empty");
    for row in rows {
        check(table, row);
    }
}

#[test]
fn quantity_parse() {
    assert!(Quantity::parse("").is_err());
    check_all("TestQuantityParse", apimachinery::QUANTITY_PARSE);
}

#[test]
fn quantity_parse_negative() {
    check_all(
        "TestQuantityParse(-)",
        apimachinery::QUANTITY_PARSE_NEGATIVE,
    );
}

#[test]
fn quantity_parse_plus() {
    check_all("TestQuantityParse(+)", apimachinery::QUANTITY_PARSE_PLUS);
}

#[test]
fn quantity_parse_invalid() {
    for input in apimachinery::INVALID {
        assert!(Quantity::parse(input).is_err(), "{input:?} parsed");
    }
}

#[test]
fn parse_quantity_string() {
    check_all(
        "TestParseQuantityString",
        apimachinery::PARSE_QUANTITY_STRING,
    );
}

#[test]
fn parse_quantity() {
    check_all("TestParseQuantity", apimachinery::PARSE_QUANTITY);
}

#[test]
fn issue_rows_and_corner_cases() {
    check_all("EXTRA", apimachinery::EXTRA);
}

#[test]
fn quantity_parse_emit() {
    for (input, expect) in apimachinery::QUANTITY_PARSE_EMIT {
        assert_eq!(
            parse("TestQuantityParseEmit", input).to_string(),
            *expect,
            "{input:?}"
        );
    }
}

#[test]
fn quantity_string() {
    for &(nanos, format, expect, alternate) in apimachinery::QUANTITY_STRING {
        let q = Quantity::from_nanos(nanos).with_format(format);
        assert_eq!(q.to_string(), expect, "{nanos}n {format:?}");
        // Canonical text is reused as-is when parsed back.
        assert_eq!(parse("TestQuantityString", expect).to_string(), expect);
        if !alternate.is_empty() {
            let alt = parse("TestQuantityString", alternate);
            assert_eq!(alt.to_string(), expect, "alternate {alternate:?}");
            assert_eq!(alt, q, "alternate {alternate:?}");
        }
        if nanos != 0 {
            let neg = Quantity::from_nanos(-nanos).with_format(format);
            assert_eq!(
                neg.to_string(),
                format!("-{expect}"),
                "-{nanos}n {format:?}"
            );
        }
    }
}

#[test]
fn divergences_are_corpus_rows() {
    let tables = [
        apimachinery::QUANTITY_PARSE,
        apimachinery::QUANTITY_PARSE_NEGATIVE,
        apimachinery::QUANTITY_PARSE_PLUS,
        apimachinery::PARSE_QUANTITY_STRING,
        apimachinery::PARSE_QUANTITY,
        apimachinery::EXTRA,
    ];
    for (row, reason) in DIVERGENCES {
        let go = tables
            .iter()
            .flat_map(|t| t.iter())
            .find(|go| go.0 == row.0)
            .unwrap_or_else(|| panic!("divergence {:?} is not a corpus row", row.0));
        assert_ne!(
            go, row,
            "{:?} is listed as a divergence but matches Go",
            row.0
        );
        assert!(!reason.is_empty());
    }
}

#[test]
fn formats_cover_every_variant() {
    // Keeps the short aliases honest.
    assert_eq!(
        [B, D, X],
        [
            QuantityFormat::BinarySI,
            QuantityFormat::DecimalSI,
            QuantityFormat::DecimalExponent
        ]
    );
}
