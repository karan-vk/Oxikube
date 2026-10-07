//! Command-line flags. Hand-rolled (a handful of flags) so the binary does not pull in an argument
//! parser and a GUI launch with unexpected platform arguments still starts.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

/// Usage text for `--help` and argument errors.
pub const USAGE: &str = "\
Usage: oxikube [OPTIONS]

Options:
  --perf                  Record frame times, feed throughput, notify counts and RSS to
                          <data dir>/oxikube/perf/*.jsonl and print p50/p95/p99 on exit
                          (data dir: ~/.local/share on Linux, ~/Library/Application Support on macOS)
  --perf-dir <DIR>        Write the --perf JSONL under DIR instead (implies --perf)
  --perf-duration <SECS>  Quit after SECS seconds (implies --perf)
  --perf-table <CONTEXT>  Connect CONTEXT, open its pods table and scroll it while recording
                          (implies --perf; docs/PERFORMANCE.md \"Resource table\")
  --perf-scroll <ROWS>    Rows --perf-table scrolls per frame (default 3; 0: keep it still)
  --perf-logs <CONTEXT>/<NAMESPACE>/<POD>
                          Connect CONTEXT and open the pod's log view while recording
                          (implies --perf; docs/PERFORMANCE.md \"Log viewer\")
  --perf-logs-wrap        Wrap the lines of --perf-logs
  --perf-logs-paused      Pause --perf-logs's autoscroll (the lines arrive off screen)
  --perf-logs-workload    Treat the last part of --perf-logs as a Deployment: open the merged log
                          of its pods (workload::ViewLogs) instead of one pod's
  --perf-scenario <NAME>  Run a headless perf scenario and exit: startup, scroll-10k (or
                          table-scroll-10k), palette, logs-stream, editor-5mb (needs --features
                          perf-scenarios; use `cargo xtask perf`)
  --perf-report <FILE>    Where --perf-scenario writes its JSON sample (default: stdout)
  --perf-no-probe         Run --perf-scenario without the frame hook (overhead measurement)
  -h, --help              Print this help";

/// Hidden tooling flags (not in [`USAGE`]): what `cargo xtask gen-settings-schema` runs, because
/// this binary links every crate that registers settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Print {
    /// `--print-settings-schema`: the `settings.schema.json` text.
    SettingsSchema,
    /// `--print-settings-crates`: the crates that registered settings, one per line.
    SettingsCrates,
}

/// Parsed flags.
#[derive(Debug, Default, PartialEq)]
pub struct Args {
    pub print: Option<Print>,
    pub perf: bool,
    pub perf_dir: Option<PathBuf>,
    pub perf_duration: Option<Duration>,
    pub perf_scenario: Option<String>,
    pub perf_report: Option<PathBuf>,
    pub perf_no_probe: bool,
    pub perf_table: Option<String>,
    pub perf_scroll: Option<usize>,
    pub perf_logs: Option<String>,
    pub perf_logs_wrap: bool,
    pub perf_logs_paused: bool,
    pub perf_logs_workload: bool,
}

/// What `main` should do.
#[derive(Debug, PartialEq)]
pub enum Parsed {
    Run(Args),
    Help,
}

/// Parses everything after the program name.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Parsed, String> {
    let mut out = Args::default();
    let mut args = args.into_iter();
    while let Some(raw) = args.next() {
        let raw = raw
            .into_string()
            .map_err(|a| format!("argument is not UTF-8: {a:?}"))?;
        // Old macOS LaunchServices passes a process serial number; ignore it.
        if raw.starts_with("-psn_") {
            continue;
        }
        let (flag, inline) = match raw.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
            _ => (raw, None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            match inline.clone() {
                Some(v) => Ok(v),
                None => args
                    .next()
                    .and_then(|v| v.into_string().ok())
                    .ok_or_else(|| format!("{name} needs a value")),
            }
        };
        match flag.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "--print-settings-schema" => out.print = Some(Print::SettingsSchema),
            "--print-settings-crates" => out.print = Some(Print::SettingsCrates),
            "--perf" => out.perf = true,
            "--perf-no-probe" => out.perf_no_probe = true,
            "--perf-dir" => {
                out.perf = true;
                out.perf_dir = Some(value("--perf-dir")?.into());
            }
            "--perf-duration" => {
                out.perf = true;
                let v = value("--perf-duration")?;
                let secs: f64 = v
                    .parse()
                    .ok()
                    .filter(|s: &f64| s.is_finite() && *s > 0.0)
                    .ok_or_else(|| {
                        format!("--perf-duration: `{v}` is not a positive number of seconds")
                    })?;
                out.perf_duration = Some(Duration::from_secs_f64(secs));
            }
            "--perf-table" => {
                out.perf = true;
                out.perf_table = Some(value("--perf-table")?);
            }
            "--perf-scroll" => {
                let v = value("--perf-scroll")?;
                let rows = v
                    .parse()
                    .map_err(|_| format!("--perf-scroll: `{v}` is not a number of rows"))?;
                out.perf_scroll = Some(rows);
            }
            "--perf-logs" => {
                out.perf = true;
                out.perf_logs = Some(value("--perf-logs")?);
            }
            "--perf-logs-wrap" => out.perf_logs_wrap = true,
            "--perf-logs-paused" => out.perf_logs_paused = true,
            "--perf-logs-workload" => out.perf_logs_workload = true,
            "--perf-scenario" => out.perf_scenario = Some(value("--perf-scenario")?),
            "--perf-report" => out.perf_report = Some(value("--perf-report")?.into()),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    if out.perf_no_probe && out.perf_scenario.is_none() {
        return Err("--perf-no-probe only applies to --perf-scenario".into());
    }
    if out.perf_report.is_some() && out.perf_scenario.is_none() {
        return Err("--perf-report only applies to --perf-scenario".into());
    }
    if out.perf_scroll.is_some() && out.perf_table.is_none() {
        return Err("--perf-scroll only applies to --perf-table".into());
    }
    if (out.perf_logs_wrap || out.perf_logs_paused || out.perf_logs_workload)
        && out.perf_logs.is_none()
    {
        return Err(
            "--perf-logs-wrap, --perf-logs-paused and --perf-logs-workload only apply to \
             --perf-logs"
                .into(),
        );
    }
    if let Some(value) = &out.perf_logs
        && value
            .rsplitn(3, '/')
            .filter(|part| !part.is_empty())
            .count()
            < 3
    {
        return Err(format!(
            "--perf-logs: `{value}` is not CONTEXT/NAMESPACE/POD"
        ));
    }
    Ok(Parsed::Run(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Args, String> {
        match parse(args.iter().map(OsString::from))? {
            Parsed::Run(a) => Ok(a),
            Parsed::Help => Err("help".into()),
        }
    }

    #[test]
    fn no_args_is_a_plain_launch() {
        assert_eq!(run(&[]).unwrap(), Args::default());
        assert_eq!(run(&["-psn_0_12345"]).unwrap(), Args::default());
    }

    #[test]
    fn perf_flags() {
        let a = run(&["--perf", "--perf-duration", "30", "--perf-dir=/tmp/p"]).unwrap();
        assert!(a.perf);
        assert_eq!(a.perf_duration, Some(Duration::from_secs(30)));
        assert_eq!(a.perf_dir, Some(PathBuf::from("/tmp/p")));
        // Duration and dir imply --perf.
        assert!(run(&["--perf-duration=0.5"]).unwrap().perf);
    }

    #[test]
    fn scenario_flags() {
        let a = run(&[
            "--perf-scenario",
            "startup",
            "--perf-report",
            "out.json",
            "--perf-no-probe",
        ])
        .unwrap();
        assert_eq!(a.perf_scenario.as_deref(), Some("startup"));
        assert_eq!(a.perf_report, Some(PathBuf::from("out.json")));
        assert!(a.perf_no_probe && !a.perf);
    }

    #[test]
    fn table_drive_flags() {
        let a = run(&["--perf-table", "kind-oxikube", "--perf-scroll=5"]).unwrap();
        assert!(a.perf, "--perf-table implies --perf");
        assert_eq!(a.perf_table.as_deref(), Some("kind-oxikube"));
        assert_eq!(a.perf_scroll, Some(5));
        assert_eq!(
            run(&["--perf-table=kind-oxikube"]).unwrap().perf_scroll,
            None
        );
        assert!(
            run(&["--perf-scroll", "3"])
                .unwrap_err()
                .contains("--perf-table")
        );
        assert!(
            run(&["--perf-table", "x", "--perf-scroll", "many"])
                .unwrap_err()
                .contains("number of rows")
        );
        assert!(USAGE.contains("--perf-table"));
    }

    #[test]
    fn logs_drive_flags() {
        let a = run(&["--perf-logs", "kind-oxikube/shop/web-0", "--perf-logs-wrap"]).unwrap();
        assert!(a.perf, "--perf-logs implies --perf");
        assert_eq!(a.perf_logs.as_deref(), Some("kind-oxikube/shop/web-0"));
        assert!(a.perf_logs_wrap && !a.perf_logs_paused && !a.perf_logs_workload);
        let w = run(&[
            "--perf-logs",
            "kind-oxikube/shop/web",
            "--perf-logs-workload",
        ])
        .unwrap();
        assert!(w.perf_logs_workload);
        assert!(run(&["--perf-logs-workload"]).is_err(), "needs --perf-logs");
        assert!(
            run(&["--perf-logs-paused"])
                .unwrap_err()
                .contains("--perf-logs")
        );
        assert!(
            run(&["--perf-logs", "web-0"])
                .unwrap_err()
                .contains("CONTEXT/NAMESPACE/POD")
        );
        assert!(USAGE.contains("--perf-logs"));
    }

    #[test]
    fn hidden_print_flags_are_accepted_but_not_advertised() {
        assert_eq!(
            run(&["--print-settings-schema"]).unwrap().print,
            Some(Print::SettingsSchema)
        );
        assert_eq!(
            run(&["--print-settings-crates"]).unwrap().print,
            Some(Print::SettingsCrates)
        );
        assert!(!USAGE.contains("--print-settings"));
    }

    #[test]
    fn errors_and_help() {
        assert!(run(&["--bogus"]).unwrap_err().contains("unknown argument"));
        assert!(
            run(&["--perf-duration"])
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(
            run(&["--perf-duration", "-1"])
                .unwrap_err()
                .contains("positive")
        );
        assert!(run(&["--perf-report", "x"]).is_err());
        assert!(run(&["--perf-no-probe"]).is_err());
        assert_eq!(parse([OsString::from("--help")]).unwrap(), Parsed::Help);
    }
}
