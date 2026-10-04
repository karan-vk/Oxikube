//! Ready-made fmt layers with redaction wired in.

use crate::{RedactingFields, RedactingMakeWriter};
use tracing::Subscriber;
use tracing_subscriber::fmt::{self, MakeWriter, format::Format};
use tracing_subscriber::registry::LookupSpan;

/// A human-readable fmt layer writing to `make_writer`, with redaction at both levels:
/// [`RedactingFields`] for field names and values, [`RedactingMakeWriter`] over every
/// formatted line. ANSI is off so colour escapes cannot hide a key from the scrubber.
pub fn redacting_layer<S, W>(
    make_writer: W,
) -> fmt::Layer<S, RedactingFields, Format, RedactingMakeWriter<W>>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'w> MakeWriter<'w> + 'static,
{
    fmt::layer()
        .fmt_fields(RedactingFields)
        .with_writer(RedactingMakeWriter::new(make_writer))
        .with_ansi(false)
}

/// A JSON fmt layer writing to `make_writer`, scrubbed at the writer.
///
/// The JSON formatter visits event fields itself, so no fields formatter can redact by name
/// here and the writer is the only redaction point. That holds only because the text patterns
/// catch every name [`is_sensitive_field`](oxikube_domain::redact::is_sensitive_field) accepts
/// in its `"<name>":<value>` form (enforced by
/// `every_sensitive_field_name_is_caught_as_text` in `oxikube_domain`), understand JSON-escaped
/// quotes, and read escaped `\n` as a line break (a manifest's `data:` block inside a JSON
/// string). A new secret-bearing field name therefore needs a text pattern, not only an entry
/// in `SENSITIVE_FIELDS`.
pub fn redacting_json_layer<S, W>(
    make_writer: W,
) -> fmt::Layer<S, fmt::format::JsonFields, Format<fmt::format::Json>, RedactingMakeWriter<W>>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'w> MakeWriter<'w> + 'static,
{
    fmt::layer()
        .json()
        .with_writer(RedactingMakeWriter::new(make_writer))
        .with_ansi(false)
}
