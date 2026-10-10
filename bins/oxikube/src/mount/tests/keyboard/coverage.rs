//! The meta-test: the E11 acceptance scenarios this suite guards, each with the tests that prove
//! it. A scenario with no test fails here, so a refactor that deletes or renames one has to say so.

/// (the scenario, the file in this directory, the test function).
const SCENARIOS: &[(&str, &str, &str)] = &[
    // A: palette (done-when 1)
    (
        "A open, type, confirm runs the command on the bus, audited",
        "palette.rs",
        "open_type_confirm_runs_the_command_through_the_bus_and_the_audit_log",
    ),
    (
        "A a mutating command is hidden in a read-only session",
        "palette.rs",
        "a_read_only_session_hides_mutations_until_the_palette_lifts_it",
    ),
    (
        "A escape returns the keys to the table",
        "palette.rs",
        "escape_closes_the_palette_and_the_table_has_its_keys_back",
    ),
    (
        "A the palette never confirms a delete",
        "palette.rs",
        "delete_from_the_palette_opens_the_tables_confirmation_and_deletes_nothing",
    ),
    (
        "A every bus command is listed with its binding",
        "palette.rs",
        "show_all_lists_every_command_the_bus_registered_with_its_binding",
    ),
    // B: jump (done-when 2)
    (
        "B :pods kube-system shows that namespace's rows",
        "colon.rs",
        "pods_kube_system_opens_the_table_with_that_namespaces_rows_only",
    ),
    (
        "B :deploy",
        "colon.rs",
        "deploy_opens_the_deployments_table_by_its_short_name",
    ),
    (
        "B :pod app=nginx forwards the selector",
        "colon.rs",
        "pod_with_a_selector_forwards_it_to_the_feed_and_narrows_the_rows",
    ),
    (
        "B a CRD short name",
        "colon.rs",
        "a_custom_resource_opens_by_its_discovered_short_name",
    ),
    (
        "B an unknown alias is an error",
        "colon.rs",
        "an_unknown_alias_stays_in_the_bar_as_an_error_and_nothing_navigates",
    ),
    // C: vim (done-when 4)
    (
        "C j k g g shift-g in a table",
        "vim.rs",
        "j_k_gg_and_shift_g_move_the_cursor_of_the_real_table",
    ),
    (
        "C d d meets the guard",
        "vim.rs",
        "d_d_opens_the_guards_confirmation_and_deletes_nothing_by_itself",
    ),
    (
        "C / and : under vim",
        "vim.rs",
        "slash_and_colon_keep_their_jobs_under_vim",
    ),
    (
        "C the setting rebinds live",
        "vim.rs",
        "switching_base_keymap_in_settings_rebinds_without_a_restart",
    ),
    // D: history
    (
        "D - [ ] after a few jumps",
        "history.rs",
        "bracket_keys_replay_the_jumps_and_dash_flips_between_the_last_two_views",
    ),
];

/// The source of each file of the suite.
const SOURCES: [(&str, &str); 4] = [
    ("palette.rs", include_str!("palette.rs")),
    ("colon.rs", include_str!("colon.rs")),
    ("vim.rs", include_str!("vim.rs")),
    ("history.rs", include_str!("history.rs")),
];

fn source(file: &str) -> &'static str {
    SOURCES
        .iter()
        .find(|(name, _)| *name == file)
        .unwrap_or_else(|| panic!("{file} is not a file of the keyboard suite"))
        .1
}

/// Whether `file` declares a `#[gpui::test]` called `name`.
fn has_test(file: &str, name: &str) -> bool {
    let source = source(file);
    let declaration = format!("fn {name}(");
    source.match_indices(&declaration).any(|(at, _)| {
        let before = source[..at].trim_end();
        before.ends_with("#[gpui::test]")
    })
}

#[test]
fn every_e11_acceptance_scenario_has_a_test() {
    let missing: Vec<_> = SCENARIOS
        .iter()
        .filter(|(_, file, name)| !has_test(file, name))
        .map(|(scenario, file, name)| format!("{scenario}: {file}::{name}"))
        .collect();
    assert!(
        missing.is_empty(),
        "scenarios without a test:\n{}",
        missing.join("\n")
    );
}

#[test]
fn every_test_of_the_suite_is_listed_as_a_scenario() {
    // The other direction: a new test must be named in `SCENARIOS`, so the list stays the map.
    for (file, source) in SOURCES {
        for (at, _) in source.match_indices("#[gpui::test]") {
            let rest = &source[at..];
            let name = rest
                .split("fn ")
                .nth(1)
                .and_then(|after| after.split('(').next())
                .expect("a test function")
                .trim();
            assert!(
                SCENARIOS.iter().any(|(_, f, n)| *f == file && *n == name),
                "{file}::{name} is not in SCENARIOS"
            );
        }
    }
}
