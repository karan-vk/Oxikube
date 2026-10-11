//! Cost of building the embedded templates (each one is read once for its kind and document
//! count) and of filling the namespace in: `cargo run -p oxikube_editor --profile release-fast
//! --example templates_bench`.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::time::Instant;

use oxikube_domain::session::NamespaceSelection;
use oxikube_editor::templates::embedded;

fn main() {
    let runs = 200;
    let started = Instant::now();
    let mut count = 0;
    for _ in 0..runs {
        count = embedded().len();
    }
    let build = started.elapsed() / runs;
    let templates = embedded();
    let selection = NamespaceSelection::single("shop");
    let started = Instant::now();
    for _ in 0..runs {
        for template in &templates {
            std::hint::black_box(template.prefill(&selection));
        }
    }
    let fill = started.elapsed() / runs;
    println!("{count} templates: build {build:?}, prefill all {fill:?}");
}
