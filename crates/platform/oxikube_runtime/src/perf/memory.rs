//! Resident memory (RSS) of this process, for the memory budget in `docs/PERFORMANCE.md`.
//!
//! One call, [`read`], per OS and without a third-party sampler crate:
//!
//! - Linux: `/proc/self/status` (`VmRSS` and `VmHWM`, both in kB); that file needs no page-size
//!   lookup, unlike `/proc/self/statm`, and carries the peak (high-water mark) too.
//! - macOS: `task_info(MACH_TASK_BASIC_INFO)` for the current resident size (bytes) and
//!   `getrusage(RUSAGE_SELF).ru_maxrss` for the peak (**bytes** on macOS, **KiB** on Linux and
//!   most other Unixes: [`ru_maxrss_bytes`] owns that unit).
//! - Anywhere else: `None`; callers write `null` and the gate reports the metric as missing.
//!
//! A reading is a syscall or a small file read, so it belongs on the `oxikube-perf` flush thread
//! (`--perf`) or between scripted frames outside the timed region (headless scenarios), never in a
//! frame on the UI thread.

/// Bytes in one MiB. Reports use MiB (`rss_mib`) so the unit is unambiguous.
pub const MIB: f64 = 1024.0 * 1024.0;

/// One RSS reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryReading {
    /// Resident set size now, bytes.
    pub rss_bytes: u64,
    /// Highest resident set size of the process so far (OS high-water mark), bytes. At least
    /// `rss_bytes` when the OS reports one.
    pub peak_rss_bytes: Option<u64>,
}

/// Bytes to MiB, rounded to three decimals like every other reported number.
pub fn bytes_to_mib(bytes: u64) -> f64 {
    super::stats::round_ms(bytes as f64 / MIB)
}

/// Reads this process's resident memory; `None` when the OS offers no reader here or it failed.
pub fn read() -> Option<MemoryReading> {
    imp::read()
}

/// Converts a raw `ru_maxrss` to bytes. `unit` is the size of one raw unit: 1 on macOS (the
/// kernel reports bytes), 1024 elsewhere (KiB).
pub fn ru_maxrss_bytes(raw: u64, unit: u64) -> u64 {
    raw.saturating_mul(unit)
}

/// Parses `/proc/self/status` text: `VmRSS:` and `VmHWM:` lines are in kB (KiB).
///
/// `None` without a `VmRSS` line (kernel threads and zombies have none).
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn parse_proc_status(text: &str) -> Option<MemoryReading> {
    fn kib(text: &str, key: &str) -> Option<u64> {
        let rest = text
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))?;
        let mut parts = rest.split_whitespace();
        let value: u64 = parts.next()?.parse().ok()?;
        match parts.next() {
            Some("kB") | None => value.checked_mul(1024),
            Some(_) => None,
        }
    }
    let rss_bytes = kib(text, "VmRSS")?;
    let peak_rss_bytes = kib(text, "VmHWM").map(|peak| peak.max(rss_bytes));
    Some(MemoryReading {
        rss_bytes,
        peak_rss_bytes,
    })
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{MemoryReading, parse_proc_status};

    pub(super) fn read() -> Option<MemoryReading> {
        parse_proc_status(&std::fs::read_to_string("/proc/self/status").ok()?)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{MemoryReading, ru_maxrss_bytes};

    pub(super) fn read() -> Option<MemoryReading> {
        let rss_bytes = current_rss_bytes()?;
        // The kernel's high-water mark can lag a just-taken task_info reading by a page or two;
        // keep the invariant peak >= current.
        let peak = peak_rss_bytes().map(|peak| peak.max(rss_bytes));
        Some(MemoryReading {
            rss_bytes,
            peak_rss_bytes: peak,
        })
    }

    #[allow(deprecated)] // libc steers to `mach2`; one call is not worth another dependency
    fn current_rss_bytes() -> Option<u64> {
        // SAFETY: `mach_task_basic_info` is plain integers, so all-zero is a valid value;
        // `task_info` writes at most `count` natural_t words into it, and `count` is the size of
        // that struct in natural_t words, which is what MACH_TASK_BASIC_INFO expects.
        unsafe {
            let mut info: libc::mach_task_basic_info = std::mem::zeroed();
            let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
            let status = libc::task_info(
                libc::mach_task_self(),
                libc::MACH_TASK_BASIC_INFO,
                (&raw mut info).cast::<libc::integer_t>(),
                &mut count,
            );
            // The struct is `packed(4)`: copy the field out instead of referencing it.
            let resident = info.resident_size;
            (status == libc::KERN_SUCCESS).then_some(resident)
        }
    }

    fn peak_rss_bytes() -> Option<u64> {
        // SAFETY: `rusage` is plain integers (all-zero is valid); `getrusage` fills it for
        // RUSAGE_SELF and returns 0 on success.
        let usage = unsafe {
            let mut usage: libc::rusage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
                return None;
            }
            usage
        };
        // macOS reports ru_maxrss in bytes.
        Some(ru_maxrss_bytes(u64::try_from(usage.ru_maxrss).ok()?, 1))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod imp {
    use super::MemoryReading;

    pub(super) fn read() -> Option<MemoryReading> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "Name:\toxikube\nUmask:\t0022\nVmPeak:\t  900000 kB\nVmSize:\t  800000 kB\n\
VmHWM:\t  123456 kB\nVmRSS:\t  100000 kB\nRssAnon:\t   90000 kB\nThreads:\t12\n";

    #[test]
    fn proc_status_kb_become_bytes() {
        let r = parse_proc_status(STATUS).unwrap();
        assert_eq!(r.rss_bytes, 100_000 * 1024);
        assert_eq!(r.peak_rss_bytes, Some(123_456 * 1024));
    }

    #[test]
    fn proc_status_vmrss_is_not_confused_with_rssanon_and_peak_never_below_current() {
        let r = parse_proc_status("VmRSS:\t2048 kB\nVmHWM:\t1024 kB\n").unwrap();
        assert_eq!(r.rss_bytes, 2048 * 1024);
        assert_eq!(r.peak_rss_bytes, Some(2048 * 1024));
    }

    #[test]
    fn proc_status_without_peak_or_rss() {
        let r = parse_proc_status("VmRSS:\t512 kB\n").unwrap();
        assert_eq!(r.peak_rss_bytes, None);
        assert_eq!(parse_proc_status("Name:\tkthreadd\nThreads:\t1\n"), None);
        assert_eq!(parse_proc_status("VmRSS:\tlots kB\n"), None);
        assert_eq!(parse_proc_status("VmRSS:\t5 MB\n"), None);
    }

    #[test]
    fn ru_maxrss_units() {
        // macOS: bytes. Linux: KiB.
        assert_eq!(ru_maxrss_bytes(104_857_600, 1), 104_857_600);
        assert_eq!(ru_maxrss_bytes(102_400, 1024), 104_857_600);
        assert_eq!(ru_maxrss_bytes(u64::MAX, 1024), u64::MAX);
    }

    #[test]
    fn mib_rounding() {
        assert_eq!(bytes_to_mib(0), 0.0);
        assert_eq!(bytes_to_mib(1024 * 1024), 1.0);
        assert_eq!(bytes_to_mib(150 * 1024 * 1024 + 524_288), 150.5);
    }

    /// The real reader on the platforms that have one: a plausible figure that follows a
    /// 64 MiB allocation that is actually touched.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn live_reading_is_plausible_and_tracks_allocation() {
        let before = read().expect("rss reader on this OS");
        assert!(before.rss_bytes > 1024 * 1024, "{before:?}");
        let peak = before.peak_rss_bytes.expect("peak on this OS");
        assert!(peak >= before.rss_bytes, "{before:?}");

        let mut block = vec![0u8; 64 * 1024 * 1024];
        for page in block.iter_mut().step_by(4096) {
            *page = 1;
        }
        let block = std::hint::black_box(block);
        let after = read().unwrap();
        assert!(
            after.rss_bytes >= before.rss_bytes + 32 * 1024 * 1024,
            "before {before:?}, after {after:?}"
        );
        assert!(after.peak_rss_bytes.unwrap() >= 64 * 1024 * 1024);
        drop(block);
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    #[test]
    fn unsupported_os_reads_none() {
        assert_eq!(read(), None);
    }
}
