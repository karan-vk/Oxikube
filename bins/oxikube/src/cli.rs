//! Command-line flags. Hand-rolled (four flags) so the binary does not pull in an argument parser
//! and a GUI launch with unexpected platform arguments still starts.

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
  --perf-scenario <NAME>  Run a headless perf scenario and exit: startup, scroll-10k, palette,
                          logs-stream, editor-5mb (needs --features perf-scenarios; use
                          `cargo xtask perf`)
  --perf-report <FILE>    Where --perf-scenario writes its JSON sample (default: stdout)
  --perf-no-probe         Run --perf-scenario without the frame hook (overhead measurement)
  -h, --help              Print this help";

/// Parsed flags.
#[derive(Debug, Default, PartialEq)]
pub struct Args {
    pub perf: bool,
    pub perf_dir: Option<PathBuf>,
    pub perf_duration: Option<Duration>,
    pub perf_scenario: Option<String>,
    pub perf_report: Option<PathBuf>,
    pub perf_no_probe: bool,
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
