//! A fields formatter that redacts secret-looking field names and scrubs the rest.

use oxikube_domain::redact::{MARKER, is_sensitive_field, redact};
use std::borrow::Cow;
use std::fmt::{self, Debug};
use tracing::field::{Field, Visit};
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::format::{FormatFields, Writer};

/// Formats event and span fields like the default `name=value` formatter, except that:
///
/// - the value of a field whose name [`is_sensitive_field`] is replaced by [`MARKER`] without
///   being formatted at all, and
/// - every other value is passed through [`redact`], so a header map or a derived `Debug` dump
///   in a field is scrubbed here, before any framing. A `&str` value is redacted before it is
///   quoted, while its line breaks are still real.
///
/// `message` is written bare and `log.*` bridge fields are skipped, as the default does.
#[derive(Debug, Clone, Copy, Default)]
pub struct RedactingFields;

impl<'writer> FormatFields<'writer> for RedactingFields {
    fn format_fields<R: RecordFields>(&self, writer: Writer<'writer>, fields: R) -> fmt::Result {
        let mut visitor = RedactingVisitor {
            writer,
            result: Ok(()),
            first: true,
        };
        fields.record(&mut visitor);
        visitor.result
    }
}

struct RedactingVisitor<'a> {
    writer: Writer<'a>,
    result: fmt::Result,
    first: bool,
}

/// A field value as recorded.
enum Value<'v> {
    /// A `&str` field: redacted raw, before `Debug` quoting escapes its newlines (a YAML
    /// manifest's `data:` block must be seen line by line).
    Str(&'v str),
    /// Anything else, formatted with `Debug` and then redacted.
    Debug(&'v dyn Debug),
}

impl RedactingVisitor<'_> {
    fn write_field(&mut self, name: &str, value: Value<'_>) -> fmt::Result {
        if !self.first {
            self.writer.write_char(' ')?;
        }
        self.first = false;
        let shown: Cow<'_, str> = if is_sensitive_field(name) {
            Cow::Borrowed(MARKER)
        } else {
            match value {
                // Quoted like the default formatter, except for a bare `message`.
                Value::Str(text) if name == "message" => redact(text),
                Value::Str(text) => Cow::Owned(format!("{:?}", redact(text))),
                Value::Debug(value) => {
                    let text = format!("{value:?}");
                    match redact(&text) {
                        Cow::Borrowed(_) => Cow::Owned(text),
                        Cow::Owned(scrubbed) => Cow::Owned(scrubbed),
                    }
                }
            }
        };
        if name == "message" {
            self.writer.write_str(&shown)
        } else {
            write!(self.writer, "{name}={shown}")
        }
    }

    fn record(&mut self, field: &Field, value: Value<'_>) {
        let name = field.name();
        if self.result.is_err() || name.starts_with("log.") {
            return;
        }
        let name = name.strip_prefix("r#").unwrap_or(name);
        self.result = self.write_field(name, value);
    }
}

impl Visit for RedactingVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, Value::Str(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.record(field, Value::Debug(value));
    }
}
