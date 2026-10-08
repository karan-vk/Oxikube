//! `--perf-scenario-window` in the binary (feature `perf-window`): which run, the environment it
//! starts in, and what happens with its summary (written next to the JSONL, printed, the app
//! quits; a script that could not run exits 1). The scenarios themselves are
//! `oxikube::perf_window`.

#[cfg(feature = "perf-window")]
mod imp {
    use std::path::PathBuf;
    use std::process::ExitCode;

    use gpui::{AnyWindowHandle, App};
    use oxikube::perf_window::{self, Scenario, WindowRun};
    use oxikube::startup::{Boot, StartupEnv};

    /// What `main` carries from the command line to the window.
    pub struct Window {
        run: WindowRun,
        report: Option<PathBuf>,
    }

    /// The run `--perf-scenario-window` asks for, or the exit code for a bad name.
    pub fn parse(
        name: &str,
        exec: Option<&str>,
        report: Option<PathBuf>,
    ) -> Result<Window, ExitCode> {
        let Some(scenario) = Scenario::parse(name) else {
            let names: Vec<&str> = Scenario::ALL.iter().map(|s| s.name()).collect();
            eprintln!(
                "oxikube: unknown windowed scenario `{name}` ({})",
                names.join(", ")
            );
            return Err(ExitCode::from(2));
        };
        let exec = exec.and_then(perf_window::run::ExecTarget::parse);
        Ok(Window {
            run: WindowRun { scenario, exec },
            report,
        })
    }

    /// The environment the app starts in for the run.
    pub fn env(window: &Window, boot: Boot) -> StartupEnv {
        perf_window::startup_env(&window.run, boot)
    }

    /// Runs the scenario in `handle`; writes and prints the summary, then quits.
    pub fn start(window: Window, handle: AnyWindowHandle, cx: &mut App) {
        let Window { run, report } = window;
        let scenario = run.scenario;
        perf_window::start(
            run,
            handle,
            move |result, cx| match result {
                Ok(summary) => {
                    let path = report.or_else(|| {
                        crate::perf_mode::jsonl_path()
                            .map(|jsonl| perf_window::run::default_summary_path(&jsonl, scenario))
                    });
                    for line in summary.lines() {
                        eprintln!("oxikube --perf-scenario-window: {line}");
                    }
                    if let Some(path) = path {
                        match write(&path, &summary) {
                            Ok(()) => eprintln!(
                                "oxikube --perf-scenario-window: wrote {}",
                                path.display()
                            ),
                            Err(err) => eprintln!(
                                "oxikube --perf-scenario-window: cannot write {}: {err:#}",
                                path.display()
                            ),
                        }
                    }
                    cx.quit();
                }
                Err(err) => {
                    eprintln!(
                        "oxikube --perf-scenario-window: `{}` could not run: {err:#}",
                        scenario.name()
                    );
                    crate::perf_mode::finish();
                    oxikube::startup::shutdown();
                    std::process::exit(1);
                }
            },
            cx,
        );
    }

    fn write(
        path: &std::path::Path,
        summary: &oxikube_runtime::perf::windowed::WindowedSummary,
    ) -> anyhow::Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(summary)? + "\n")?;
        Ok(())
    }
}

#[cfg(not(feature = "perf-window"))]
mod imp {
    use std::path::PathBuf;
    use std::process::ExitCode;

    use gpui::{AnyWindowHandle, App};
    use oxikube::startup::{Boot, StartupEnv};

    /// Never built without the feature.
    pub enum Window {}

    /// Without the feature there is nothing to run.
    pub fn parse(_: &str, _: Option<&str>, _: Option<PathBuf>) -> Result<Window, ExitCode> {
        eprintln!(
            "oxikube: --perf-scenario-window needs a build with `--features perf-window` \
             (`cargo xtask perf --windowed` builds one)"
        );
        Err(ExitCode::from(2))
    }

    pub fn env(window: &Window, _: Boot) -> StartupEnv {
        match *window {}
    }

    pub fn start(window: Window, _: AnyWindowHandle, _: &mut App) {
        match window {}
    }
}

pub use imp::{Window, env, parse, start};
