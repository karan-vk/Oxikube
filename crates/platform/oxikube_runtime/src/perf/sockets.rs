//! How many network (IPv4/IPv6) sockets this process holds: the "no network before the first
//! frame" check of the startup budget (E05-S13, docs/PERFORMANCE.md "Startup").
//!
//! [`inet_socket_count`] counts the open file descriptors that are `AF_INET` / `AF_INET6` sockets
//! (TCP or UDP, bound, connected or not). Unix-domain sockets (the windowing system, IPC) and
//! every other descriptor are not counted.
//!
//! - Linux: the inodes behind `/proc/self/fd/*` links of the form `socket:[inode]`, matched with
//!   the inode column of `/proc/self/net/{tcp,tcp6,udp,udp6}`. No `unsafe`.
//! - macOS: each descriptor listed in `/dev/fd` that `fstat` reports as a socket, asked for its
//!   family with `getsockname`.
//! - Anywhere else: `None`.
//!
//! A count is a directory listing and a syscall or two per descriptor (tens of µs for a fresh
//! process). Take it once, after the first frame, never inside a frame.

/// The number of IPv4/IPv6 sockets open in this process; `None` where the OS offers no reader
/// here, or when it failed.
pub fn inet_socket_count() -> Option<usize> {
    imp::count()
}

/// Inodes of the IPv4/IPv6 sockets in one `/proc/net/{tcp,udp}[6]` table (column 10, after the
/// header line).
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn parse_proc_net_inodes(text: &str) -> impl Iterator<Item = u64> + '_ {
    text.lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().nth(9)?.parse().ok())
}

/// The inode of a `/proc/self/fd/<n>` link target `socket:[inode]`.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn socket_inode(link: &str) -> Option<u64> {
    link.strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::HashSet;

    pub fn count() -> Option<usize> {
        let mut sockets = HashSet::new();
        for entry in std::fs::read_dir("/proc/self/fd").ok()?.flatten() {
            if let Ok(target) = std::fs::read_link(entry.path())
                && let Some(inode) = target.to_str().and_then(super::socket_inode)
            {
                sockets.insert(inode);
            }
        }
        if sockets.is_empty() {
            return Some(0);
        }
        let mut inet = HashSet::new();
        for table in ["tcp", "tcp6", "udp", "udp6"] {
            // A missing table (IPv6 disabled) has no sockets in it.
            if let Ok(text) = std::fs::read_to_string(format!("/proc/self/net/{table}")) {
                inet.extend(super::parse_proc_net_inodes(&text));
            }
        }
        Some(sockets.intersection(&inet).count())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn count() -> Option<usize> {
        let mut fds: Vec<libc::c_int> = std::fs::read_dir("/dev/fd")
            .ok()?
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
            .collect();
        // The listing's own descriptor is closed by now; probing it again is harmless (`fstat`
        // fails and it is skipped).
        fds.sort_unstable();
        Some(fds.into_iter().filter(|&fd| is_inet_socket(fd)).count())
    }

    fn is_inet_socket(fd: libc::c_int) -> bool {
        // SAFETY: `stat` is plain old data that `fstat` fills in; an all-zero value is a valid
        // initial state and nothing reads it unless the call succeeded.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: `stat` is a valid, writable `libc::stat`; an invalid `fd` makes the call fail
        // with EBADF, it is never undefined behaviour.
        if unsafe { libc::fstat(fd, &mut stat) } != 0
            || (stat.st_mode & libc::S_IFMT) != libc::S_IFSOCK
        {
            return false;
        }
        // SAFETY: as above, `sockaddr_storage` is plain old data and zero is a valid value.
        let mut addr: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        // SAFETY: `addr` is large enough for any address family and `len` says so; the kernel
        // writes at most `len` bytes and updates `len`.
        let named = unsafe {
            libc::getsockname(
                fd,
                (&mut addr as *mut libc::sockaddr_storage).cast::<libc::sockaddr>(),
                &mut len,
            )
        };
        named == 0
            && matches!(
                libc::c_int::from(addr.ss_family),
                libc::AF_INET | libc::AF_INET6
            )
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod imp {
    pub fn count() -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_net_tables_and_fd_links() {
        let tcp = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 424242 1 0000000000000000 100 0 0 10 0\n";
        assert_eq!(parse_proc_net_inodes(tcp).collect::<Vec<_>>(), [424242]);
        assert_eq!(socket_inode("socket:[424242]"), Some(424242));
        assert_eq!(socket_inode("pipe:[1]"), None);
        assert_eq!(socket_inode("/dev/null"), None);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn counts_tcp_and_udp_sockets_but_not_unix_ones() {
        let before = inet_socket_count().expect("a reader on this OS");
        let (_a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        assert_eq!(
            inet_socket_count(),
            Some(before),
            "unix sockets do not count"
        );

        let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        // Other tests in this binary may open sockets concurrently, so only a lower bound holds.
        assert!(inet_socket_count().unwrap() >= before + 2);
        drop((tcp, udp));
    }
}
