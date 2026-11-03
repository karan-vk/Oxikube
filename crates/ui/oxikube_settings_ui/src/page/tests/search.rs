//! The fuzzy search over the model, the layer badge's words, and the search's time budget.

use std::time::{Duration, Instant};

use oxikube_settings::SettingSource;

use crate::page::test_support::fixture_model;
use crate::page::{LayerBadge, MatchedIn, SearchHit, SearchIndex, SettingsPageModel};

fn titles(model: &SettingsPageModel, hits: &[SearchHit]) -> Vec<String> {
    hits.iter()
        .map(|hit| {
            let section = model.section(hit.section).expect("hit section");
            match hit.field {
                Some(field) => section.fields[field].path.to_string(),
                None => format!("[{}]", section.key),
            }
        })
        .collect()
}

#[test]
fn a_blank_query_finds_nothing() {
    let index = SearchIndex::new(&fixture_model());
    assert!(index.search("").is_empty());
    assert!(index.search("   ").is_empty());
    assert_eq!(index.len(), 17, "3 sections and 14 fields");
}

#[test]
fn a_fuzzy_title_query_finds_the_field_first_with_its_match_ranges() {
    let model = fixture_model();
    let hits = SearchIndex::new(&model).search("fnt sz");
    let found = titles(&model, &hits);
    assert_eq!(
        found.first().map(String::as_str),
        Some("terminal.font_size")
    );
    let first = &hits[0];
    assert_eq!(first.matched_in, MatchedIn::Title);
    assert!(
        !first.title_ranges.is_empty(),
        "the title match is highlighted"
    );
}

#[test]
fn section_titles_match_too() {
    let model = fixture_model();
    let hits = SearchIndex::new(&model).search("manifest");
    assert_eq!(
        titles(&model, &hits).first().map(String::as_str),
        Some("[editor]")
    );
}

#[test]
fn a_word_from_a_description_finds_its_field() {
    let model = fixture_model();
    let hits = SearchIndex::new(&model).search("clipboard");
    let found = titles(&model, &hits);
    assert_eq!(found, ["terminal.copy_on_select"]);
    assert_eq!(hits[0].matched_in, MatchedIn::Description);
    assert!(hits[0].title_ranges.is_empty());
}

#[test]
fn letters_scattered_through_prose_are_not_a_match() {
    let model = fixture_model();
    // Every letter of "zqx" is nowhere; "lsd" is in many descriptions only one letter per word.
    assert!(SearchIndex::new(&model).search("zqx").is_empty());
    let hits = SearchIndex::new(&model).search("lsd");
    assert!(
        hits.iter()
            .all(|hit| hit.matched_in != MatchedIn::Description),
        "{:?}",
        titles(&model, &hits)
    );
}

#[test]
fn a_path_query_finds_the_field() {
    let model = fixture_model();
    let hits = SearchIndex::new(&model).search("editor.tab");
    assert_eq!(
        titles(&model, &hits).first().map(String::as_str),
        Some("editor.tab_size")
    );
}

#[test]
fn a_title_match_ranks_above_a_description_match() {
    let model = fixture_model();
    let hits = SearchIndex::new(&model).search("cursor");
    let found = titles(&model, &hits);
    let shape = found
        .iter()
        .position(|p| p == "terminal.cursor_shape")
        .expect("shape");
    let blink = found
        .iter()
        .position(|p| p == "terminal.cursor_blink")
        .expect("blink");
    assert!(shape < 2 && blink < 2, "{found:?}");
}

#[test]
fn filtering_the_index_is_well_inside_a_frame() {
    // A page of 40 sections with 10 fields each, every field with a long description: ten times
    // the fixture and about twice the real schema.
    let mut model_sections = Vec::new();
    let base = fixture_model();
    for copy in 0..40 {
        for section in base.sections() {
            let mut section = section.clone();
            section.key = format!("{}{copy}", section.key).into();
            model_sections.push(section);
        }
    }
    let index = SearchIndex::new(&SettingsPageModel::from_sections(model_sections));
    let queries = [
        "f",
        "fo",
        "fon",
        "font",
        "font s",
        "font si",
        "font siz",
        "font size",
    ];
    // The best of several rounds: a test thread preempted by its neighbours measures the
    // machine's load, not the search.
    let per_keystroke = (0..5)
        .map(|_| {
            let started = Instant::now();
            for query in queries {
                let _ = index.search(query);
            }
            started.elapsed() / 8
        })
        .min()
        .expect("five rounds");
    // Debug build on a shared CI runner: generous. The release figure is in the PR.
    assert!(
        per_keystroke < Duration::from_millis(40),
        "{per_keystroke:?} per keystroke"
    );
}

#[test]
fn the_badge_names_the_layer_and_explains_the_precedence() {
    let default = LayerBadge::new(SettingSource::Default);
    assert_eq!(default.label(), "Default");
    assert!(!default.is_override());
    assert!(default.tooltip().contains("Built-in default"));

    let user = LayerBadge::new(SettingSource::User);
    assert_eq!(user.label(), "User");
    assert!(user.is_override());
    assert!(user.tooltip().contains("your settings.json"));

    let cluster = LayerBadge::new(SettingSource::Cluster);
    assert_eq!(cluster.label(), "Cluster");
    assert!(cluster.tooltip().contains("clusters.<id>"));
    for badge in [default, user, cluster] {
        assert!(badge.tooltip().contains("Layers apply in order"));
    }
}
