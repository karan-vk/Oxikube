use oxikube_domain::helm::group_hits;

use super::hit;
use crate::charts::model::{Card, count_line};

#[test]
fn a_card_says_what_the_acceptance_criteria_ask() {
    let charts = group_hits(vec![
        hit("bitnami", "nginx", "15.0.0", "1.25.0", "Web server"),
        hit("bitnami", "nginx", "14.0.0", "1.24.0", "Web server"),
    ]);
    let card = Card::of(&charts[0]);
    assert_eq!(card.reference, "bitnami/nginx");
    assert_eq!(card.name, "nginx");
    assert_eq!(card.repo, "bitnami");
    assert_eq!(card.version, "15.0.0");
    assert_eq!(card.app_version, "1.25.0");
    assert_eq!(card.description, "Web server");
    assert_eq!(card.versions, 2);
    assert_eq!(card.initial, 'N');
    assert_eq!(card, Card::of(&charts[0]), "the colour is stable");
    assert!(card.hue < 360);
}

#[test]
fn the_count_line_pluralises() {
    assert_eq!(count_line(1, 1), "1 chart in 1 repository");
    assert_eq!(count_line(0, 2), "0 charts in 2 repositories");
}
