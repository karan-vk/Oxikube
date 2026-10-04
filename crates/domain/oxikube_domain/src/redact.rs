//! Secret redaction: one pure function every log, audit record, crash report and error text
//! passes through before it can leave the process.
//!
//! [`redact`] takes any text and returns it with secret-bearing substrings replaced by
//! [`MARKER`] (`[redacted]`). It is pure (no I/O, no globals beyond lazily compiled patterns),
//! idempotent (`redact(redact(x)) == redact(x)`), and cheap: a byte-level scan decides which
//! patterns could possibly match, and text with no candidate substring is returned borrowed
//! without touching `regex` at all. `oxikube_logging` wraps it in a tracing writer and fields
//! formatter; adapters call it directly for error text and manual `Debug` impls; audit and crash
//! reporting must reuse it rather than copy the patterns.
//!
//! # Redacted patterns
//!
//! The authoritative list is [`PATTERNS`] (snapshotted with `insta` so any change shows up in
//! review). Values are replaced; keys, separators and quote style stay so output remains
//! readable and structurally valid (`"token": "[redacted]"`).
//!
//! | Pattern | Matches |
//! |---|---|
//! | `pem-private-key` | `-----BEGIN ... PRIVATE KEY-----` blocks (to the END line, or to the end of the text when truncated) |
//! | `authorization-header` | the value of any key ending in `authorization` (`Authorization`, `Proxy-Authorization`, `proxy_authorization`; any scheme, `["Basic ..."]` arrays, comma-separated parameter lists such as SigV4) in header, YAML, JSON and `key=value` forms |
//! | `secret-field` | values of keys ending in `token` (`token`, `id-token`, `refresh_token`, ...), `password` / `passwd`, `secret` (`client-secret`, `secret`), `apikey` / `api_key` / `api-key`; `client-key-data`, `client-certificate-data`; scalar `secret_data` / `string_data` / `stringData` values (every [`SENSITIVE_FIELDS`] name and [`SENSITIVE_SUFFIXES`] suffix, so a JSON log line's `"<name>":<value>` is covered) |
//! | `url-userinfo` | the password in `scheme://user:password@host` URLs (proxy URLs); the user and host stay |
//! | `scheme-credential` | `Bearer <token>` and `Basic <credentials>` anywhere in text (a plain word such as "bearer authentication" or "basic auth" is left alone) |
//! | `jwt` | JWT-shaped strings: `eyJ…` header, payload and signature, base64url separated by dots |
//! | `secret-data-flow` | every value inside a `data` / `stringData` / `secret_data` map written inline (`data: {a: b}`, `"data":{"a":"b"}`, Rust `Debug` maps) |
//! | `secret-data-block` | every entry under a multi-line `data:` / `stringData:` block (YAML or pretty JSON), with lines ending at real newlines or escaped `\n` (a manifest inside a JSON log line or a `Debug`-quoted string); a `\n` escaped one level deeper than the header's break is inside a quoted value and does not end the entry, and the block ends with the string that holds it |
//!
//! Field forms handled: YAML `key: value`, JSON `"key":"value"` (also JSON escaped inside a
//! string, `\"key\":\"value\"`), `key=value`, and Rust `Debug` output (`key: Some("value")`,
//! byte arrays such as `ByteString([1, 2])`). An unquoted value runs to whitespace; a `, ; ) } ]`
//! inside it (`p4ss;w0rd`) is part of the secret unless it is followed by whitespace, the end,
//! another closer or a quote (`token=abc}`, `token=abc, next`).
//!
//! The bias is deliberate: when in doubt, redact. The `data` rule also scrubs `ConfigMap` data;
//! `token: connection refused` and `secret: not found` lose their value. Long non-secret
//! base64 or hex (a UID, a digest, `certificate-authority-data`) is untouched here because no
//! pattern keys on shape alone except `jwt` and PEM; the kube adapter additionally masks long
//! opaque runs in external error text, where nothing of that shape is worth showing.
//!
//! # Adding a secret-bearing field
//!
//! **New secret-bearing fields must be added here.** When a later story introduces a credential
//! (a new kubeconfig auth field, an API key header, a tool secret), extend [`patterns`] and the
//! pre-check in `scrubber`, add a case to `tests/redact.rs`, accept the changed `insta`
//! snapshot, and keep this table in sync. Tracing field *names* are checked by
//! [`is_sensitive_field`]; every name it accepts must also be caught by the text patterns in
//! `"<name>":<value>` form (the JSON log layer relies on that), which
//! `every_sensitive_field_name_is_caught_as_text` in `tests/redact.rs` enforces.
//!
//! # Not covered
//!
//! Redaction is a backstop, not a licence to log secrets: `Debug` impls of credential-holding
//! types must still print names and hosts only (or use [`Redacted`]). Secrets split across
//! separate writes, or hidden by ANSI escapes inside a key, are out of reach for a text
//! scrubber; `oxikube_logging` formats with ANSI off for that reason.

mod blocks;
mod patterns;
mod scrubber;
mod values;
mod wrapper;

pub use patterns::{
    MARKER, PATTERNS, Pattern, SENSITIVE_FIELDS, SENSITIVE_SUFFIXES, is_sensitive_field, patterns,
};
pub use scrubber::redact;
pub use wrapper::Redacted;
