//! [`parse_k9s_forwards`]: the pure grammar of the k9s forward annotations.

/// Longest annotation read; the rest is ignored (and reported).
pub const MAX_ANNOTATION_LEN: usize = 4096;

/// Most entries read from one annotation; the rest are ignored (and reported).
pub const MAX_ENTRIES: usize = 64;

/// Longest text of a bad entry kept for display.
const MAX_SHOWN: usize = 48;

/// One forward an annotation describes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SuggestedForward {
    /// The container the port belongs to; `None` is the pod's default container.
    pub container: Option<String>,
    /// The local port to listen on (fixed, never Auto: an annotation names it).
    pub local_port: u16,
    /// The container port to forward to.
    pub remote_port: u16,
    /// Listed in `k9scli.io/auto-port-forwards`: k9s starts it by itself; here it is preselected
    /// in the dialog, nothing more.
    pub auto: bool,
}

/// An entry that was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseIssue {
    /// The entry as written, cleaned of control characters and cut to a displayable length.
    pub entry: String,
    /// Why it was skipped.
    pub reason: String,
}

/// What an annotation yielded: the good entries and the skipped ones.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedForwards {
    /// The forwards, in the order written, without duplicates.
    pub forwards: Vec<SuggestedForward>,
    /// The entries that were skipped.
    pub issues: Vec<ParseIssue>,
}

/// Parses the value of `k9scli.io/port-forwards` (`auto` false) or `k9scli.io/auto-port-forwards`
/// (`auto` true).
///
/// Grammar: comma-separated entries, whitespace around entries and parts ignored,
///
/// ```text
/// entry     = [ container "::" ] ports
/// ports     = localPort ":" containerPort | port
/// ```
///
/// where a lone `port` is both the local and the container port, ports run from 1 to 65535 and
/// `container` is a DNS label (letters, digits, `-`, `.`; no container means the default one).
///
/// | Annotation | Meaning |
/// |---|---|
/// | `web::8080:80` | container `web`, local 8080 to container port 80 |
/// | `8080:80` | the default container, local 8080 to 80 |
/// | `web::9000` | container `web`, 9000 to 9000 |
/// | `web::8080:80, 8081:81` | two entries |
///
/// A malformed entry is skipped and reported in [`ParsedForwards::issues`]; the others are kept.
/// An exact repeat is dropped. Input over [`MAX_ANNOTATION_LEN`] bytes or [`MAX_ENTRIES`] entries
/// is cut and reported. Never panics.
pub fn parse_k9s_forwards(annotation: &str, auto: bool) -> ParsedForwards {
    let mut out = ParsedForwards::default();
    let text = cut(annotation, MAX_ANNOTATION_LEN);
    if text.len() < annotation.len() {
        out.issues.push(ParseIssue {
            entry: String::new(),
            reason: format!(
                "the annotation is longer than {MAX_ANNOTATION_LEN} bytes: the rest is ignored"
            ),
        });
    }
    for (index, raw) in text.split(',').enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        if index >= MAX_ENTRIES {
            out.issues.push(ParseIssue {
                entry: String::new(),
                reason: format!("more than {MAX_ENTRIES} entries: the rest are ignored"),
            });
            break;
        }
        match entry(raw, auto) {
            Ok(forward) => {
                if !out.forwards.contains(&forward) {
                    out.forwards.push(forward);
                }
            }
            Err(reason) => out.issues.push(ParseIssue {
                entry: shown(raw),
                reason,
            }),
        }
    }
    out
}

fn entry(raw: &str, auto: bool) -> Result<SuggestedForward, String> {
    let raw = raw.trim();
    let (container, ports) = match raw.split_once("::") {
        Some((container, ports)) => (Some(container.trim()), ports),
        None => (None, raw),
    };
    let container = match container {
        None | Some("") => None,
        Some(name) if valid_container(name) => Some(name.to_owned()),
        Some(_) => return Err("not a container name".to_owned()),
    };
    let (local, remote) = match ports.split_once(':') {
        Some((local, remote)) => (port(local)?, port(remote)?),
        None => {
            let both = port(ports)?;
            (both, both)
        }
    };
    Ok(SuggestedForward {
        container,
        local_port: local,
        remote_port: remote,
        auto,
    })
}

fn port(text: &str) -> Result<u16, String> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("`{}` is not a port number", shown(text)));
    }
    text.parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| "ports run from 1 to 65535".to_owned())
}

fn valid_container(name: &str) -> bool {
    name.len() <= 63
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
}

/// `text` cut to at most `max` bytes on a character boundary.
fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// `text` safe to display: control characters dropped, length bounded.
fn shown(text: &str) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    if clean.chars().count() > MAX_SHOWN {
        let head: String = clean.chars().take(MAX_SHOWN).collect();
        format!("{head}…")
    } else {
        clean.to_owned()
    }
}
