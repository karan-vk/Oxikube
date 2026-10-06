//! Startup cost of settings: parse `default.json` and a typical user `settings.json`, resolve
//! every setting, and (separately) one hot reload, reporting the median and worst of 200 runs;
//! then the cost of starting the file watcher (50 runs).
//!
//! `cargo run --release -p oxikube_settings --example settings_load` (budget: < 30 ms on the
//! main thread at startup, E05-S13).
#![allow(clippy::print_stdout)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use oxikube_settings::{Settings, SettingsStore};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A section shaped like a real feature's settings: scalars, a list and a map.
#[derive(Default, Serialize, Deserialize, JsonSchema)]
struct SectionContent {
    font_family: Option<String>,
    font_size: Option<f32>,
    line_height: Option<f32>,
    enabled: Option<bool>,
    mode: Option<String>,
    limit: Option<u32>,
    items: Option<Vec<String>>,
    env: Option<BTreeMap<String, String>>,
}

macro_rules! sections {
    ($($name:ident = $key:literal),+ $(,)?) => {$(
        #[derive(PartialEq)]
        #[allow(dead_code)]
        struct $name(f32, u32, usize);

        impl Settings for $name {
            const KEY: Option<&'static str> = Some($key);
            type Content = SectionContent;

            fn from_content(c: SectionContent) -> Self {
                Self(
                    c.font_size.unwrap_or_default(),
                    c.limit.unwrap_or_default(),
                    c.items.map_or(0, |items| items.len()),
                )
            }
        }
    )+
    fn register_all(store: &mut SettingsStore) {
        $(store.register_setting::<$name>();)+
        // The per-cluster settings (E06-S08) resolve once per cluster with overrides.
        store.register_setting::<oxikube_settings::ClusterSettings>();
    }
    const KEYS: &[&str] = &[$($key),+];
    };
}

sections!(
    Terminal = "terminal",
    Editor = "editor",
    Logs = "logs",
    Events = "events",
    Metrics = "metrics",
    Helm = "helm",
    PortForward = "port_forward",
    Cloud = "cloud",
    Workspace = "workspace",
    Theme = "theme",
);

fn section_text(indent: &str, n: usize) -> String {
    format!(
        "{{\n{indent}  // Comment explaining the section.\n{indent}  \"font_family\": \"JetBrains Mono\",\n{indent}  \"font_size\": {}.5,\n{indent}  \"line_height\": 1.4,\n{indent}  \"enabled\": true,\n{indent}  \"mode\": \"auto\", // trailing comment\n{indent}  \"limit\": {},\n{indent}  \"items\": [\"a\", \"b\", \"c\"],\n{indent}  \"env\": {{ \"KUBECONFIG\": \"/tmp/x\", \"LANG\": \"C\" }},\n{indent}}}",
        10 + n,
        1000 * n
    )
}

fn files() -> (String, String) {
    let mut defaults = String::from("// defaults\n{\n");
    let mut user = String::from("// My settings\n{\n");
    for (n, key) in KEYS.iter().enumerate() {
        defaults.push_str(&format!("  \"{key}\": {},\n", section_text("  ", n)));
        if n % 2 == 0 {
            user.push_str(&format!("  \"{key}\": {},\n", section_text("  ", n + 1)));
        }
    }
    user.push_str("  \"clusters\": {\n");
    for c in 0..20 {
        user.push_str(&format!(
            "    \"{c:016x}\": {{ \"terminal\": {{ \"font_size\": {c} }}, \"logs\": {{ \"limit\": 5 }}, \"display_name\": \"cluster {c}\", \"colour\": \"#e5484d\", \"read_only\": true, \"default_namespace\": \"ns-{c}\", \"accessible_namespaces\": [\"a\", \"b\"], \"prometheus\": {{ \"url\": \"https://prom.example.com\", \"auth_secret\": \"p{c}\" }} }},\n"
        ));
    }
    user.push_str("  },\n");
    defaults.push_str("  \"clusters\": {},\n}\n");
    user.push_str("}\n");
    (defaults, user)
}

fn stats(mut samples: Vec<Duration>) -> String {
    samples.sort();
    format!(
        "median {:?}, p95 {:?}, max {:?}",
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100],
        samples[samples.len() - 1]
    )
}

fn main() {
    let (defaults, user) = files();
    println!(
        "{} settings types, default.json {} bytes, settings.json {} bytes ({} lines, 20 clusters)",
        KEYS.len(),
        defaults.len(),
        user.len(),
        user.lines().count()
    );

    let mut startup = Vec::new();
    let mut reload = Vec::new();
    for i in 0..200 {
        let started = Instant::now();
        let mut store = SettingsStore::without_registered(&defaults).unwrap();
        register_all(&mut store);
        store.set_user_settings(&user).unwrap();
        startup.push(started.elapsed());
        assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());

        let edited = user.replace("\"limit\": 5", &format!("\"limit\": {}", 6 + i));
        let started = Instant::now();
        store.set_user_settings(&edited).unwrap();
        reload.push(started.elapsed());
    }
    println!("startup (parse + resolve): {}", stats(startup));
    println!("hot reload (parse + resolve + diff): {}", stats(reload));

    // Starting hot reload: the `notify` watcher (FSEvents on macOS, inotify on Linux) and its
    // debounce thread. `init` does this on the UI thread at start-up, so it counts towards the
    // 30 ms settings + keymap + theme budget (E05-S13).
    let root = std::env::temp_dir().join(format!("settings-load-{}", std::process::id()));
    let mut watch = Vec::new();
    for i in 0..50 {
        let dir = root.join(i.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, &user).unwrap();
        let started = Instant::now();
        let watcher = oxikube_settings::watcher::SettingsFileWatcher::spawn(
            path,
            user.clone(),
            oxikube_settings::watcher::DEFAULT_DEBOUNCE,
            |_| true,
        )
        .unwrap();
        watch.push(started.elapsed());
        drop(watcher);
    }
    let _ = std::fs::remove_dir_all(root);
    println!("starting the file watcher: {}", stats(watch));
}
