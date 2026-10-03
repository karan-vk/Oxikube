//! The redaction catalogue: pattern sources, the marker, and sensitive field names.
//!
//! Sources are built from shared fragments with `concat!` so [`PATTERNS`] and the compiled
//! regexes in `scrubber` can never drift apart.

/// The text that replaces every redacted value.
pub const MARKER: &str = "[redacted]";

/// A documented redaction pattern, for reviewers and the `insta` snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pattern {
    /// Stable kebab-case name.
    pub name: &'static str,
    /// What it protects.
    pub description: &'static str,
    /// The regular expression source, or a prose description for hand-written scanners.
    pub source: &'static str,
}

/// A value: the marker itself (keeps redaction idempotent), a JSON-escaped quoted string, a
/// double- or single-quoted string, a `Debug`-printed byte array, or a bare word. A bare word
/// may contain escape pairs and quotes that are followed by more value (`a"b`, `a\"b`) so a
/// secret with a quote in it is consumed whole, but a trailing quote (a JSON string's closing
/// quote) is left alone.
macro_rules! value {
    () => {
        concat!(
            r#"(?:\[redacted\]|"#,
            esc_quoted!(),
            r#"|"(?:[^"\\]|\\.)*"|'[^']*'|[A-Za-z_]\w*\(\[[0-9, \t]*\]\)|\[[0-9][0-9, \t]*\]|(?:\\.|[^\s,;}\])"'\\]|["'][^\s,;}\])"'\\])+)"#
        )
    };
}

/// A quoted string inside a JSON string (`\"abc\"`). Content may hold an escaped backslash
/// pair or a twice-escaped quote (`\\\"`) without ending the string.
macro_rules! esc_quoted {
    () => {
        r#"\\"(?:[^"\\]|\\\\\\"|\\[^"])*\\""#
    };
}

/// One Authorization parameter. A quoted string (plain or JSON-escaped) is atomic when it opens
/// the parameter or follows `=` (`nonce="a, b"`), so spaces, commas and braces inside it stay
/// inside. Anywhere else a quote is part of the value only when more value follows it: a
/// trailing quote is a JSON string's closing quote and must survive.
macro_rules! atom {
    () => {
        concat!(
            r#"(?:(?:"#,
            esc_quoted!(),
            r#"|"(?:[^"\\]|\\.)*")(?:=(?:"#,
            esc_quoted!(),
            r#"|"(?:[^"\\]|\\.)*")|\\.|[^\s,}\])"'\\]|["'][^\s,}\])"'\\])*|(?:=(?:"#,
            esc_quoted!(),
            r#"|"(?:[^"\\]|\\.)*")|\\.|[^\s,}\])"'\\]|["'][^\s,}\])"'\\])+)"#
        )
    };
}

/// Separator between a key and its value: optional closing quote, `:` or `=`, optional
/// `Some(` from `Debug` output. Horizontal whitespace only, so a bare `key:` never reaches into
/// the next line.
macro_rules! sep {
    () => {
        r#"(?:\\?["'])?[ \t]*[:=][ \t]*(?:some\()?"#
    };
}

pub(super) const PEM_SRC: &str =
    r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|\z)";

pub(super) const AUTHORIZATION_SRC: &str = concat!(
    r"(?i)(?P<key>\bauthorization)(?P<sep>",
    sep!(),
    r")(?P<val>\[[^\]]*\]|[A-Za-z][\w-]*[ \t]+(?:\[redacted\]|",
    atom!(),
    r")(?:,[ \t]*",
    atom!(),
    r")*|",
    value!(),
    r")"
);

pub(super) const SECRET_FIELD_SRC: &str = concat!(
    r"(?i)(?P<key>\b[\w-]*(?:token|password|passwd)\b|\bclient[-_](?:key|certificate)[-_]data\b|\bclient[-_]secret\b)(?P<sep>",
    sep!(),
    r")(?P<val>",
    value!(),
    r")"
);

pub(super) const URL_USERINFO_SRC: &str =
    r#"(?P<pre>\b[A-Za-z][A-Za-z0-9+.-]*://[^\s/:@"'\\]*:)(?P<pw>[^\s/"'\\]+)@"#;

pub(super) const BEARER_SRC: &str = r"(?i)\bbearer[ \t]+(?P<tok>[A-Za-z0-9._~+/=-]{8,})";

pub(super) const JWT_SRC: &str = r"eyJ[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]*";

pub(super) const DATA_FLOW_SRC: &str = concat!(
    r"(?i)(?P<key>\b(?:string)?data)(?P<sep>",
    sep!(),
    r#")(?P<open>(?:\\?["'])?\{)(?P<body>(?:"#,
    esc_quoted!(),
    r#"|"(?:[^"\\]|\\.)*"|[^{}])*)\}"#
);

/// One `key: value` entry inside a `data` map (flow body or a single block line).
pub(super) const DATA_PAIR_SRC: &str = concat!(
    r#"(?P<k>\\?"[^"\\]*\\?"|[\w.-]+)(?P<s>[ \t]*[:=][ \t]*)(?P<val>"#,
    value!(),
    r")"
);

/// A line that opens a multi-line `data` / `stringData` block.
pub(super) const DATA_HEADER_SRC: &str =
    r#"(?i)^[ \t]*"?(?:string)?data"?[ \t]*:[ \t]*\{?[ \t]*\r?$"#;

/// Every documented pattern, in application order.
pub const PATTERNS: &[Pattern] = &[
    Pattern {
        name: "pem-private-key",
        description: "PEM private key blocks",
        source: PEM_SRC,
    },
    Pattern {
        name: "authorization-header",
        description: "Authorization / Proxy-Authorization values: any scheme, quoted, bracketed or comma-separated parameter forms",
        source: AUTHORIZATION_SRC,
    },
    Pattern {
        name: "secret-field",
        description: "*token, *password, *passwd, client-key-data, client-certificate-data, client-secret values",
        source: SECRET_FIELD_SRC,
    },
    Pattern {
        name: "url-userinfo",
        description: "the password in scheme://user:password@host URLs (proxy URLs)",
        source: URL_USERINFO_SRC,
    },
    Pattern {
        name: "bearer-token",
        description: "`Bearer <token>` anywhere in text (plain alphabetic words under 20 chars are kept)",
        source: BEARER_SRC,
    },
    Pattern {
        name: "jwt",
        description: "JWT-shaped strings: eyJ header, payload, signature",
        source: JWT_SRC,
    },
    Pattern {
        name: "secret-data-flow",
        description: "values inside an inline data / stringData map; each entry matched by secret-data-pair",
        source: DATA_FLOW_SRC,
    },
    Pattern {
        name: "secret-data-pair",
        description: "one key/value entry of a data map",
        source: DATA_PAIR_SRC,
    },
    Pattern {
        name: "secret-data-block",
        description: "entries indented under a multi-line data / stringData header line (hand-written scanner; header regex shown, entries use secret-data-pair)",
        source: DATA_HEADER_SRC,
    },
];

/// The pattern catalogue, for reviewers and snapshot tests.
pub fn patterns() -> &'static [Pattern] {
    PATTERNS
}

/// Tracing field names whose values are always redacted, in addition to every name ending in
/// one of [`SENSITIVE_SUFFIXES`]. Matched case-insensitively; `-` and `_` spellings are both
/// listed.
pub const SENSITIVE_FIELDS: &[&str] = &[
    "client-key-data",
    "client_key_data",
    "client-certificate-data",
    "client_certificate_data",
    "client-secret",
    "client_secret",
    "secret-data",
    "secret_data",
    "string-data",
    "string_data",
    "stringdata",
];

/// Suffixes that mark a field name as secret-bearing (`id_token`, `db.password`, `http.authorization`).
pub const SENSITIVE_SUFFIXES: &[&str] = &["token", "password", "passwd", "authorization"];

/// Whether a tracing field named `name` must have its value redacted outright.
///
/// Used by `oxikube_logging`'s fields formatter; the text patterns still run over the values of
/// every other field. `max_tokens` is not sensitive; `id_token` and `http.authorization` are.
pub fn is_sensitive_field(name: &str) -> bool {
    SENSITIVE_FIELDS
        .iter()
        .any(|f| name.eq_ignore_ascii_case(f))
        || SENSITIVE_SUFFIXES.iter().any(|s| {
            name.len() >= s.len()
                && name.is_char_boundary(name.len() - s.len())
                && name[name.len() - s.len()..].eq_ignore_ascii_case(s)
        })
}
