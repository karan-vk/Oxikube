//! The `ls` parser against the fixture corpus.

use super::{parse_find, parse_ls};
use crate::files::entry::{FileEntry, FileKind};

/// 2024-03-06 00:00:00 UTC: the "now" the fixtures' dates are read against.
const NOW: i64 = 1_709_683_200;

fn corpus(name: &str) -> String {
    let path = format!(
        "{}/src/files/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn entries(name: &str) -> Vec<FileEntry> {
    parse_ls(&corpus(name), NOW).entries
}

fn named<'a>(entries: &'a [FileEntry], name: &str) -> &'a FileEntry {
    entries
        .iter()
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no entry {name:?} in {:?}",
                entries.iter().map(|e| &e.name).collect::<Vec<_>>()
            )
        })
}

#[test]
fn gnu_full_time_gives_size_mode_owner_and_an_exact_time() {
    let list = entries("gnu_full_time.txt");
    assert_eq!(list.len(), 6, "total, . and .. are not entries");
    let app = named(&list, "app");
    assert_eq!(app.kind, FileKind::File);
    assert_eq!(app.size, Some(1_234_567));
    assert_eq!(app.mode, "rwxr-xr-x");
    assert_eq!((app.owner.as_str(), app.group.as_str()), ("root", "root"));
    // 2024-01-15 23:59:59 +01:00 = 22:59:59 UTC.
    assert_eq!(app.modified, Some(1_705_359_599));
    // A negative offset moves the other way: 2024-02-01 00:00 -05:30 = 05:30 UTC.
    assert_eq!(named(&list, "config").modified, Some(1_706_765_400));
    assert_eq!(named(&list, "config").kind, FileKind::Directory);
    assert_eq!(named(&list, "su").mode, "rwsr-xr-x");
    assert_eq!(named(&list, "tmp").mode, "rwxrwxrwt");
    assert_eq!(named(&list, "empty.log").size, Some(0));
    assert_eq!(named(&list, "empty.log").owner, "www-data");
    assert!(named(&list, ".bash_logout").is_hidden());
}

#[test]
fn gnu_without_full_time_infers_the_year_and_takes_the_zone_to_be_utc() {
    let list = entries("gnu_classic.txt");
    assert_eq!(list.len(), 4);
    // "Mar  5 12:34" read on 2024-03-06 is this year.
    assert_eq!(named(&list, "nginx.conf").modified, Some(1_709_642_040));
    // "Oct 11  2022" has a year and no time.
    assert_eq!(named(&list, "mime.types").modified, Some(1_665_446_400));
    // "Dec 31 23:00" is in the future of a March "now": last year's.
    assert_eq!(named(&list, "selinux-file").modified, Some(1_704_063_600));
    // A trailing `.` (SELinux) or `+` (ACL) is not part of the mode.
    assert_eq!(named(&list, "selinux-file").mode, "rw-r--r--");
    assert_eq!(named(&list, "acl-file").mode, "rw-r--r--");
}

#[test]
fn busybox_columns_links_and_devices() {
    let list = entries("busybox.txt");
    assert_eq!(list.len(), 4);
    let sh = named(&list, "sh");
    assert_eq!(sh.kind, FileKind::Symlink);
    assert_eq!(sh.link_target.as_deref(), Some("busybox"));
    assert!(sh.is_navigable());
    let null = named(&list, "null");
    assert_eq!(null.kind, FileKind::Special);
    assert_eq!(null.size, None, "device numbers are not a size");
    assert_eq!(named(&list, "busybox").size, Some(829_784));
    assert!(!named(&list, "busybox").is_navigable());
}

#[test]
fn busybox_ctime_dates() {
    let list = entries("busybox_ctime.txt");
    assert_eq!(list.len(), 1);
    // Wed Jan  3 01:02:03 2024
    assert_eq!(list[0].modified, Some(1_704_243_723));
    assert_eq!(list[0].name, "hello.txt");
}

#[test]
fn alpine_root() {
    let list = entries("alpine.txt");
    let names: Vec<_> = list.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [".dockerenv", "bin", "dev", "etc", "lib", "proc", "var"]
    );
    assert_eq!(named(&list, "lib").link_target.as_deref(), Some("/usr/lib"));
    assert_eq!(named(&list, "proc").mode, "r-xr-xr-x");
    assert_eq!(named(&list, "proc").size, Some(0));
}

#[test]
fn symlink_targets_keep_spaces_and_arrows() {
    let list = entries("symlinks.txt");
    assert_eq!(
        named(&list, "current").link_target.as_deref(),
        Some("releases/7")
    );
    assert_eq!(
        named(&list, "data").link_target.as_deref(),
        Some("/mnt/data with space")
    );
    assert_eq!(named(&list, "loop").link_target.as_deref(), Some("loop"));
    let arrow = named(&list, "a");
    assert_eq!(arrow.link_target.as_deref(), Some("b -> c"));
}

#[test]
fn names_with_spaces_unicode_and_odd_starts_survive() {
    let list = entries("spaces_unicode.txt");
    let names: Vec<_> = list.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "my file.txt",
            "two  spaces  here",
            "日本語のファイル.txt",
            "émoji 🎉.md",
            " leading-space",
            "-dash-first",
            "arrow -> in name.txt",
        ]
    );
    // Only a link is split at " -> ".
    assert_eq!(list[6].kind, FileKind::File);
    assert_eq!(list[6].link_target, None);
}

#[test]
fn entries_ls_could_not_stat_are_listed_by_name_and_counted() {
    let parsed = parse_ls(&corpus("denied.txt"), NOW);
    assert_eq!(parsed.unreadable, 2);
    let locked = named(&parsed.entries, "locked");
    assert_eq!(locked.kind, FileKind::Directory);
    assert_eq!(locked.size, None);
    assert!(locked.mode.is_empty());
    assert_eq!(named(&parsed.entries, "secret.key").kind, FileKind::File);
    assert_eq!(named(&parsed.entries, "readable").size, Some(10));
    assert_eq!(parsed.entries.len(), 4);
}

#[test]
fn an_empty_directory_has_no_entries() {
    let parsed = parse_ls(&corpus("empty.txt"), NOW);
    assert!(parsed.entries.is_empty());
    assert_eq!(parsed.unreadable, 0);
    assert!(parse_ls("", NOW).entries.is_empty());
}

#[test]
fn lines_that_are_not_entries_are_skipped_not_fatal() {
    let parsed = parse_ls(&corpus("garbage.txt"), NOW);
    assert!(parsed.entries.is_empty(), "{:?}", parsed.entries);
}

#[test]
fn crlf_output_is_read() {
    let parsed = parse_ls("-rw-r--r-- 1 root root 5 Mar  5 12:34 a.txt\r\n", NOW);
    assert_eq!(parsed.entries[0].name, "a.txt");
}

#[test]
fn find_fallback_reads_kinds_and_strips_the_directory() {
    let output =
        "d /etc/ssl\nf /etc/passwd\nl /etc/localtime\nf /etc//double\nnoise\nd /etc/with space\n";
    let list = parse_find(output, "/etc");
    let got: Vec<_> = list.iter().map(|e| (e.name.as_str(), e.kind)).collect();
    assert_eq!(
        got,
        [
            ("ssl", FileKind::Directory),
            ("passwd", FileKind::File),
            ("localtime", FileKind::Symlink),
            ("double", FileKind::File),
            ("with space", FileKind::Directory),
        ]
    );
    let root = parse_find("d /bin\nf /init\n", "/");
    assert_eq!(
        root.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["bin", "init"]
    );
}
