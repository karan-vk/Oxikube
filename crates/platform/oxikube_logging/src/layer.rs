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

/// A JSON fmt layer writing to `make_writer`, scrubbed at the writer. JSON framing owns the
/// field visitor, so the writer is the only (and sufficient) redaction point; patterns
/// understand JSON-escaped quotes inside message strings.
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
