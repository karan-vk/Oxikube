//! `kubectl get` output as headers and cells, for the Table API parity tests (E04-S04).

use std::process::Command;

use serde_json::Value;

/// Runs `kubectl get <resource> -n <namespace> [-o wide]` on `context` and splits its table:
/// upper-case headers and, per row, one string per column. Columns are cut at the offsets
/// where headers start (kubectl left-aligns with at least two spaces between columns), so
/// empty cells and headers with spaces (`NOMINATED NODE`) survive.
pub fn kubectl_get(
    context: &str,
    namespace: &str,
    resource: &str,
    wide: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let mut command = Command::new("kubectl");
    command.args(["--context", context, "get", resource, "-n", namespace]);
    if wide {
        command.args(["-o", "wide"]);
    }
    let output = command.output().expect("run kubectl");
    assert!(
        output.status.success(),
        "kubectl get failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("utf-8 kubectl output");
    let mut lines = text.lines();
    let header = lines.next().expect("kubectl header line");
    let starts = column_starts(header);
    let cut = |line: &str| -> Vec<String> {
        starts
            .iter()
            .enumerate()
            .map(|(i, &start)| {
                let end = starts.get(i + 1).copied().unwrap_or(usize::MAX);
                let end = end.min(line.len());
                line.get(start.min(end)..end)
                    .unwrap_or("")
                    .trim()
                    .to_owned()
            })
            .collect()
    };
    let headers = cut(header);
    let rows = lines.filter(|l| !l.trim().is_empty()).map(cut).collect();
    (headers, rows)
}

/// Byte offsets where header columns start: the first non-space after a run of two or more
/// spaces (or the line start).
fn column_starts(header: &str) -> Vec<usize> {
    let bytes = header.as_bytes();
    let mut starts = vec![0];
    let mut spaces = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b' ' {
            spaces += 1;
        } else {
            if spaces >= 2 {
                starts.push(i);
            }
            spaces = 0;
        }
    }
    starts
}

/// A Table cell as kubectl prints it: strings verbatim, numbers and booleans in decimal,
/// `null` as nothing.
pub fn render_cell(cell: &Value) -> String {
    match cell {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}
