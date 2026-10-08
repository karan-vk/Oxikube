//! CPU time of this process (user + system), for the windowed scenarios' CPU figure and the idle
//! CPU budget (ADR 0016: < 1 % with two clusters connected).
//!
//! [`process_time`] is `getrusage(RUSAGE_SELF)` on Unix: every thread of the process, the UI
//! thread, Tokio's workers and GPUI's background executor alike. CPU % over an interval is the
//! difference of two readings divided by the wall time ([`percent`]), 100 % being one core busy
//! for the whole interval (`top`'s convention). Elsewhere it is `None`.
//!
//! A reading is a syscall: take it at the edges of a measured phase, never inside a frame.

use std::time::Duration;

/// User plus system CPU time consumed by the process so far; `None` where the OS offers no reader.
pub fn process_time() -> Option<Duration> {
    imp::process_time()
}

/// CPU used between two readings as a percentage of one core over `wall`, rounded to two
/// decimals. `None` without both readings or with no wall time.
pub fn percent(before: Option<Duration>, after: Option<Duration>, wall: Duration) -> Option<f64> {
    let used = after?.checked_sub(before?)?;
    if wall.is_zero() {
        return None;
    }
    let pct = used.as_secs_f64() / wall.as_secs_f64() * 100.0;
    Some((pct * 100.0).round() / 100.0)
}

#[cfg(unix)]
mod imp {
    use std::time::Duration;

    pub fn process_time() -> Option<Duration> {
        // SAFETY: `getrusage` writes one `rusage` into the zeroed struct we own; RUSAGE_SELF is a
        // valid `who` on every Unix, and the struct outlives the call.
        let usage = unsafe {
            let mut usage: libc::rusage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
                return None;
            }
            usage
        };
        Some(timeval(usage.ru_utime)? + timeval(usage.ru_stime)?)
    }

    fn timeval(tv: libc::timeval) -> Option<Duration> {
        let secs = u64::try_from(tv.tv_sec).ok()?;
        let micros = u64::try_from(tv.tv_usec).ok()?;
        Some(Duration::from_secs(secs) + Duration::from_micros(micros))
    }
}

#[cfg(not(unix))]
mod imp {
    use std::time::Duration;

    pub fn process_time() -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_of_one_core() {
        let s = |ms| Some(Duration::from_millis(ms));
        assert_eq!(percent(s(100), s(150), Duration::from_secs(1)), Some(5.0));
        assert_eq!(percent(s(0), s(2_000), Duration::from_secs(1)), Some(200.0));
        assert_eq!(percent(s(10), s(10), Duration::from_secs(10)), Some(0.0));
        assert_eq!(percent(None, s(1), Duration::from_secs(1)), None);
        assert_eq!(percent(s(1), s(2), Duration::ZERO), None);
        assert_eq!(
            percent(s(5), s(1), Duration::from_secs(1)),
            None,
            "never negative"
        );
    }

    #[test]
    fn busy_work_moves_the_reading() {
        let Some(before) = process_time() else {
            assert!(!cfg!(unix), "Unix has a reader");
            return;
        };
        let started = std::time::Instant::now();
        let mut x = 0u64;
        while started.elapsed() < Duration::from_millis(50) {
            x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(7));
        }
        let after = process_time().unwrap();
        assert!(after > before, "50 ms of spinning shows as CPU time");
    }
}
