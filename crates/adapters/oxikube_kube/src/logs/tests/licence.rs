//! Licence headers: every file `THIRD_PARTY_NOTICES.md` lists as derived from kdash keeps the
//! MIT copyright and permission text in its header.

const DERIVED: [(&str, &str); 3] = [
    ("follow/mod.rs", include_str!("../follow/mod.rs")),
    ("follow/resume.rs", include_str!("../follow/resume.rs")),
    ("dedup.rs", include_str!("../dedup.rs")),
];

#[test]
fn kdash_derived_files_carry_the_mit_header() {
    for (name, source) in DERIVED {
        let header: String = source
            .lines()
            .take_while(|line| line.starts_with("//") && !line.starts_with("//!"))
            .collect::<Vec<_>>()
            .join("\n");
        for needle in [
            "kdash-rs/kdash",
            "Copyright (c) 2021 Deepu K Sasidharan",
            "Permission is hereby granted, free of charge",
            "THE SOFTWARE IS PROVIDED \"AS IS\"",
        ] {
            assert!(
                header.contains(needle),
                "logs/{name} header lacks {needle:?}"
            );
        }
    }
}
