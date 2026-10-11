//! The chunked pass without a window: plan order, chunk sizes, the cap, cancellation, and that a
//! pass in chunks finds exactly what a one-shot validation finds (E10-S11).

use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;
use serde_json::json;

use super::*;
use crate::view::validation::{KnownSchemas, validate_text};
use crate::yaml::parse_shared;

fn pod_schema() -> Arc<JsonSchema> {
    Arc::new(JsonSchema::from_value(&json!({
        "type": "object",
        "properties": {
            "apiVersion": {"type": "string"},
            "kind": {"type": "string"},
            "metadata": {"type": "object"},
            "spec": {"type": "object", "properties": {
                "replicas": {"type": "integer"},
                "nodeName": {"type": "string"}
            }}
        }
    })))
}

fn pod() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// A `kind: List` of `pods` pods; every `bad`-th has `replicas: many` (a type mismatch).
fn dump(pods: usize, bad: usize) -> String {
    let mut out = String::from("apiVersion: v1\nkind: List\nitems:\n");
    for i in 0..pods {
        let replicas = if i % bad == bad - 1 {
            "many".to_owned()
        } else {
            i.to_string()
        };
        out.push_str(&format!(
            "- apiVersion: v1\n  kind: Pod\n  metadata:\n    name: pod-{i}\n  spec:\n    nodeName: worker-{i}\n    replicas: {replicas}\n"
        ));
    }
    out
}

fn known() -> KnownSchemas {
    let mut known = KnownSchemas::new();
    known.insert(pod(), Some(pod_schema()));
    known
}

fn units_of(text: &str) -> (Arc<ParseResult>, Vec<Unit>) {
    let parsed = Arc::new(parse_shared(Arc::from(text)));
    let units = validate::units(&parsed);
    (parsed, units)
}

#[test]
fn a_small_buffer_is_one_chunk() {
    let text = dump(10, 3);
    let parsed = Arc::new(parse_shared(Arc::from(text.as_str())));
    let plan = plan(4, parsed, Some(0..10), LanguageMode::Auto, false);
    assert_eq!(plan.version, 4);
    assert_eq!(plan.chunks.len(), 1);
    assert_eq!(plan.chunks[0].len(), 10);
}

#[test]
fn an_empty_buffer_still_has_a_chunk() {
    let plan = plan(
        1,
        Arc::new(parse_shared(Arc::from(""))),
        None,
        LanguageMode::Auto,
        false,
    );
    assert_eq!(plan.chunks, vec![Vec::<Unit>::new()]);
}

#[test]
fn the_visible_objects_come_first_then_the_rest_in_order() {
    let text = dump(100, 7);
    let (_, units) = units_of(&text);
    let middle = units[50].span.clone();
    let chunks = chunk(
        units.clone(),
        Some(middle.start + 1..middle.start + 2),
        1_000,
    );
    assert_eq!(chunks[0], vec![units[50].clone()]);
    let rest: Vec<Unit> = chunks[1..].concat();
    let expected: Vec<Unit> = units
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 50)
        .map(|(_, u)| u.clone())
        .collect();
    assert_eq!(rest, expected, "the rest keep buffer order");
    // About 110 bytes a pod and 1 000 bytes a chunk: about ten pods each.
    assert!(chunks[1..].iter().all(|c| c.len() <= 10), "{chunks:?}");
    assert!(chunks.len() > 9);
}

#[test]
fn a_large_buffer_is_chunked_by_size() {
    let text = dump(LARGE_BUFFER_BYTES / 100, 50);
    assert!(text.len() >= LARGE_BUFFER_BYTES);
    let parsed = Arc::new(parse_shared(Arc::from(text.as_str())));
    let plan = plan(1, parsed, None, LanguageMode::Auto, false);
    let expected = text.len() / CHUNK_BYTES;
    assert!(
        (expected..=expected + 1).contains(&plan.chunks.len()),
        "{} chunks for {} bytes",
        plan.chunks.len(),
        text.len()
    );
}

#[test]
fn chunks_add_up_to_the_one_shot_pass() {
    let text = dump(400, 9);
    let (parsed, units) = units_of(&text);
    let plan = Plan {
        version: 2,
        parsed: parsed.clone(),
        mode: LanguageMode::Auto,
        language: Language::Yaml,
        base: validate::syntax_diagnostics(&parsed, &ValidateOptions::default())
            .iter()
            .map(to_editor)
            .collect(),
        chunks: chunk(units, Some(5_000..5_100), 2_000),
    };
    assert!(plan.chunks.len() > 5);
    let mut progress = Progress::start(&plan);
    let token = CancelToken::new();
    for (i, c) in plan.chunks.iter().enumerate() {
        assert!(!progress.is_complete());
        let partial = progress.validation();
        assert_eq!(
            partial.coverage,
            Coverage {
                done: i,
                total: plan.chunks.len()
            }
        );
        let findings = check_chunk(&parsed, c, &known(), progress.room(), &token).expect("runs");
        progress.add(findings);
    }
    assert!(progress.is_complete());
    let chunked = progress.validation();
    let whole = validate_text(2, Arc::from(text.as_str()), &known());
    assert_eq!(chunked.diagnostics, whole.diagnostics);
    assert_eq!(chunked.missing, whole.missing);
    assert!(chunked.coverage.is_complete() && whole.coverage.is_complete());
    assert_eq!(whole.diagnostics.len(), 400 / 9);
}

#[test]
fn unknown_kinds_are_reported_per_chunk() {
    let text = dump(5, 2);
    let (parsed, units) = units_of(&text);
    let findings = check_chunk(
        &parsed,
        &units,
        &KnownSchemas::new(),
        1_000,
        &CancelToken::new(),
    )
    .expect("runs");
    assert_eq!(findings.missing, vec![pod()]);
    assert!(findings.diagnostics.is_empty());
}

#[test]
fn the_cap_ends_the_pass() {
    // Every pod is wrong: the cap (1 000) is reached long before the last chunk.
    let text = dump(3_000, 1);
    let (parsed, units) = units_of(&text);
    let plan = Plan {
        version: 1,
        parsed: parsed.clone(),
        mode: LanguageMode::Auto,
        language: Language::Yaml,
        base: validate::syntax_diagnostics(&parsed, &ValidateOptions::default())
            .iter()
            .map(to_editor)
            .collect(),
        chunks: chunk(units, None, 20_000),
    };
    let mut progress = Progress::start(&plan);
    let mut checked = 0;
    for c in &plan.chunks {
        if progress.is_complete() {
            break;
        }
        let room = progress.room();
        progress.add(check_chunk(&parsed, c, &known(), room, &CancelToken::new()).expect("runs"));
        checked += 1;
    }
    assert!(
        checked < plan.chunks.len(),
        "{checked} of {}",
        plan.chunks.len()
    );
    let shown = progress.validation();
    assert_eq!(
        shown.diagnostics.len(),
        ValidateOptions::DEFAULT_MAX_DIAGNOSTICS
    );
    assert!(shown.coverage.is_complete());
}

#[test]
fn a_cancelled_chunk_stops_and_reports_nothing() {
    let text = dump(200, 3);
    let (parsed, units) = units_of(&text);
    let token = CancelToken::new();
    token.cancel();
    assert_eq!(
        check_chunk(&parsed, &units, &known(), 1_000, &token),
        Err(Cancelled)
    );
}

#[test]
fn a_chunk_cancelled_midway_stops_at_the_next_unit() {
    let text = dump(200, 3);
    let (parsed, units) = units_of(&text);
    let token = CancelToken::new();
    let mut asked = 0;
    let schema = pod_schema();
    // The schema lookup runs once per unit: cancel while the 10th is being checked.
    let result = validate::validate_units(
        &parsed,
        &units,
        |_| {
            asked += 1;
            if asked == 10 {
                token.cancel();
            }
            Some(schema.clone())
        },
        &ValidateOptions::default(),
        &token,
    );
    assert_eq!(result, Err(Cancelled));
    assert!(
        asked <= 11,
        "stopped within one unit of the cancel, asked {asked}"
    );
}
