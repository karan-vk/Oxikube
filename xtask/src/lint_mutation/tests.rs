use super::scan_source;

fn clean(src: &str) {
    let found = scan_source(src);
    assert!(found.is_empty(), "expected no violations, got {found:?}");
}

fn flagged(src: &str, needle: &str) {
    let found = scan_source(src);
    assert!(
        found.iter().any(|m| m.contains(needle)),
        "expected a violation containing {needle:?}, got {found:?}"
    );
}

#[test]
fn read_only_code_passes() {
    clean(
        "use oxikube_ports::ResourceReader;
         async fn list(r: std::sync::Arc<dyn ResourceReader>) { r.list().await; r.get().await; }",
    );
}

#[test]
fn implementing_the_writer_trait_is_allowed() {
    clean(
        "use oxikube_ports::ResourceWriter;
         struct World;
         #[async_trait]
         impl ResourceWriter for World {
             async fn create(&self) {}
             async fn delete(&self) { self.create().await; }
         }",
    );
}

#[test]
fn holding_a_writer_is_a_violation() {
    flagged(
        "struct V { w: std::sync::Arc<dyn ResourceWriter> }",
        "ResourceWriter",
    );
    flagged("fn f<W: ResourceWriter>(_w: W) {}", "ResourceWriter");
}

#[test]
fn naming_mutation_is_a_violation() {
    flagged("fn f(m: &oxikube_app::guard::Mutation) {}", "Mutation");
    flagged("fn f(m: &Mutation) {}", "Mutation");
}

#[test]
fn mutating_call_on_a_typed_handle_is_a_violation() {
    flagged(
        "async fn f(p: std::sync::Arc<dyn ResourcePort>) { p.delete().await; }",
        "`.delete()`",
    );
    flagged(
        "struct V { port: Box<dyn ResourcePort> }
         impl V { async fn go(&self) { self.port.scale().await; } }",
        "`.scale()`",
    );
}

#[test]
fn mutating_call_on_a_writer_getter_is_a_violation() {
    flagged(
        "async fn f(x: X) { x.writer().patch().await; }",
        "`.patch()`",
    );
    flagged(
        "async fn f(x: X) { let w = x.writer(); w.evict().await; }",
        "`.evict()`",
    );
}

#[test]
fn mutating_names_on_unrelated_receivers_pass() {
    clean(
        "fn f(map: &mut std::collections::HashMap<u8, u8>, v: &mut Vec<u8>, s: Store) {
             map.remove(&1); v.create(); s.delete(); s.patch();
         }",
    );
}

#[test]
fn reading_through_a_resource_port_passes() {
    clean("async fn f(p: std::sync::Arc<dyn ResourcePort>) { p.list().await; p.get().await; }");
}

#[test]
fn strings_and_comments_are_ignored() {
    clean(
        r#"// uses ResourceWriter and Mutation in prose; w.delete()
           /// docs mention `Mutation`
           fn f() -> &'static str { "ResourceWriter Mutation x.writer().delete()" }"#,
    );
}

#[test]
fn cfg_test_items_are_exempt() {
    clean(
        "#[cfg(test)]
         mod tests { fn t(m: &Mutation, w: &dyn ResourceWriter) { w.writer().delete(); } }
         #[cfg(test)]
         fn helper(m: &Mutation) {}",
    );
}

#[test]
fn mutating_call_inside_a_macro_is_found() {
    flagged(
        "async fn f(p: std::sync::Arc<dyn ResourcePort>) { assert!(p.delete().await.is_ok()); }",
        "`.delete()`",
    );
}

#[test]
fn unparseable_source_is_reported_not_skipped() {
    flagged("fn broken( {", "does not parse");
}

#[test]
fn module_resolution_skips_cfg_test_files() {
    let dir = std::env::temp_dir().join(format!("lint_mutation_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("a")).expect("create fixture dir");
    std::fs::write(dir.join("lib.rs"), "mod a;\nmod b;\n#[cfg(test)]\nmod t;\n")
        .expect("write lib");
    std::fs::write(dir.join("a/mod.rs"), "mod inner;").expect("write a");
    std::fs::write(dir.join("a/inner.rs"), "").expect("write inner");
    std::fs::write(dir.join("b.rs"), "").expect("write b");
    std::fs::write(dir.join("t.rs"), "fn t(m: &Mutation) {}").expect("write t");
    let files = super::modules::module_files(&dir.join("lib.rs"));
    let names: Vec<_> = files
        .iter()
        .map(|f| {
            f.strip_prefix(&dir)
                .expect("under dir")
                .display()
                .to_string()
        })
        .collect();
    std::fs::remove_dir_all(&dir).expect("cleanup");
    assert_eq!(names, ["lib.rs", "a/mod.rs", "a/inner.rs", "b.rs"]);
}
