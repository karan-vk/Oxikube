//! The few POSIX calls the local PTY needs: signalling a process group, reaping the child with
//! its exact exit status, probing whether a pid is alive, and reading the directory a process
//! works in. Other platforms get stand-ins (ConPTY is E28-S04).

use std::path::PathBuf;

use oxikube_ports::exec::ExitStatus;

/// Sends `SIGKILL` to the child's process group. The child is a session leader
/// (`portable-pty` calls `setsid`), so its group is its pid and also holds everything the shell
/// started.
#[cfg(unix)]
pub(super) fn kill_group(pid: u32) {
    // SAFETY: plain syscalls with integer arguments; a stale pid at worst signals nothing.
    unsafe {
        if libc::killpg(pid as libc::pid_t, libc::SIGKILL) != 0 {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
}

/// Blocks until the child ends and reaps it. Done here rather than through `portable-pty`,
/// whose status keeps the signal only as a localised description.
#[cfg(unix)]
pub(super) fn wait_for_exit(
    _child: &mut (dyn portable_pty::Child + Send + Sync),
    pid: u32,
) -> ExitStatus {
    loop {
        let mut raw: libc::c_int = 0;
        // SAFETY: `raw` is a valid out pointer for the call's duration.
        let result = unsafe { libc::waitpid(pid as libc::pid_t, &mut raw, 0) };
        if result < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return ExitStatus {
                message: Some("the shell's exit status is unknown".into()),
                ..ExitStatus::default()
            };
        }
        if libc::WIFEXITED(raw) {
            return ExitStatus::with_code(libc::WEXITSTATUS(raw));
        }
        if libc::WIFSIGNALED(raw) {
            return ExitStatus::killed_by(signal_name(libc::WTERMSIG(raw)));
        }
        // Stopped or continued: not an exit, keep waiting.
    }
}

/// Whether a process with `pid` exists.
#[cfg(unix)]
pub(super) fn process_exists(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process can be signalled.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The conventional name of a signal without the `SIG` prefix (`"KILL"`, `"HUP"`).
#[cfg(unix)]
fn signal_name(signal: i32) -> String {
    let name = match signal {
        libc::SIGHUP => "HUP",
        libc::SIGINT => "INT",
        libc::SIGQUIT => "QUIT",
        libc::SIGILL => "ILL",
        libc::SIGABRT => "ABRT",
        libc::SIGFPE => "FPE",
        libc::SIGKILL => "KILL",
        libc::SIGBUS => "BUS",
        libc::SIGSEGV => "SEGV",
        libc::SIGPIPE => "PIPE",
        libc::SIGALRM => "ALRM",
        libc::SIGTERM => "TERM",
        other => return format!("SIG{other}"),
    };
    name.to_owned()
}

/// The foreground process group of the terminal `master` drives (what the shell runs, or the
/// shell itself at its prompt): its leader's pid.
#[cfg(unix)]
pub(super) fn foreground_group(master: &dyn portable_pty::MasterPty) -> Option<u32> {
    master
        .process_group_leader()
        .and_then(|pid| u32::try_from(pid).ok())
}

/// The directory process `pid` works in (`/proc/<pid>/cwd`).
#[cfg(target_os = "linux")]
pub(super) fn working_directory(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

/// The directory process `pid` works in (`proc_pidinfo(PROC_PIDVNODEPATHINFO)`).
#[cfg(target_os = "macos")]
pub(super) fn working_directory(pid: u32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt as _;

    let pid = libc::c_int::try_from(pid).ok()?;
    let mut info = std::mem::MaybeUninit::<libc::proc_vnodepathinfo>::zeroed();
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_vnodepathinfo>()).ok()?;
    // SAFETY: `info` is a writable buffer of exactly `size` bytes; the kernel fills at most that.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // SAFETY: zero-initialised, then filled in full by the call above.
    let info = unsafe { info.assume_init() };
    // `vip_path` is a NUL-terminated `char[MAXPATHLEN]`, split in rows by `libc`.
    let bytes: Vec<u8> = info
        .pvi_cdir
        .vip_path
        .iter()
        .flatten()
        .map(|&c| c as u8)
        .take_while(|&b| b != 0)
        .collect();
    (!bytes.is_empty()).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(&bytes)))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub(super) fn working_directory(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(not(unix))]
pub(super) fn working_directory(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(not(unix))]
pub(super) fn foreground_group(_master: &dyn portable_pty::MasterPty) -> Option<u32> {
    None
}

#[cfg(not(unix))]
pub(super) fn kill_group(_pid: u32) {}

#[cfg(not(unix))]
pub(super) fn wait_for_exit(
    child: &mut (dyn portable_pty::Child + Send + Sync),
    _pid: u32,
) -> ExitStatus {
    match child.wait() {
        Ok(status) => ExitStatus::with_code(status.exit_code() as i32),
        Err(_) => ExitStatus::default(),
    }
}

#[cfg(not(unix))]
pub(super) fn process_exists(_pid: u32) -> bool {
    true
}
