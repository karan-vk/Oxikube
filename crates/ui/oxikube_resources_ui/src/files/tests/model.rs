//! The model alone: sorting (directories first, natural order), the hidden-file filter, the
//! selection, and a 10 000-entry directory.

use oxikube_app::files::{FileEntry, FileKind, Listing, ListingSource};

use crate::files::model::{FileColumn, FileSort, FilesModel};

fn entry(name: &str, kind: FileKind, size: u64, modified: i64) -> FileEntry {
    FileEntry {
        name: name.to_owned(),
        kind,
        size: Some(size),
        mode: "rw-r--r--".to_owned(),
        modified: Some(modified),
        owner: "root".to_owned(),
        group: "root".to_owned(),
        link_target: None,
    }
}

fn listing(entries: Vec<FileEntry>) -> Listing {
    Listing {
        path: "/d".to_owned(),
        entries,
        unreadable: 0,
        source: ListingSource::Ls,
    }
}

fn names(model: &FilesModel) -> Vec<String> {
    model.rows().map(|row| row.entry.name.clone()).collect()
}

fn sample() -> Listing {
    listing(vec![
        entry("file10.txt", FileKind::File, 5, 300),
        entry("file2.txt", FileKind::File, 50, 100),
        entry("Zeta", FileKind::Directory, 4096, 200),
        entry("alpha", FileKind::Directory, 4096, 400),
        entry(".hidden", FileKind::File, 1, 500),
        entry("link", FileKind::Symlink, 7, 250),
    ])
}

#[test]
fn directories_come_first_then_names_in_natural_order_with_dot_files_hidden() {
    let mut model = FilesModel::new();
    model.set_listing(&sample());
    assert_eq!(
        names(&model),
        ["alpha", "Zeta", "file2.txt", "file10.txt", "link"]
    );
    assert_eq!((model.len(), model.total(), model.hidden()), (5, 6, 1));
    model.set_show_hidden(true);
    assert_eq!(model.len(), 6);
    assert_eq!(names(&model)[2], ".hidden");
    model.set_show_hidden(false);
    assert_eq!(model.len(), 5);
}

#[test]
fn sorting_by_size_and_time_keeps_directories_first_and_reverses_within_each_group() {
    let mut model = FilesModel::new();
    model.set_listing(&sample());
    model.set_sort(FileSort {
        column: FileColumn::Size,
        descending: false,
    });
    assert_eq!(
        names(&model),
        ["alpha", "Zeta", "file10.txt", "link", "file2.txt"]
    );
    model.set_sort(FileSort {
        column: FileColumn::Size,
        descending: true,
    });
    assert_eq!(
        names(&model),
        ["Zeta", "alpha", "file2.txt", "link", "file10.txt"]
    );
    model.set_sort(FileSort {
        column: FileColumn::Modified,
        descending: true,
    });
    assert_eq!(
        names(&model),
        ["alpha", "Zeta", "file10.txt", "link", "file2.txt"]
    );
}

#[test]
fn the_selection_follows_the_entry_through_a_sort_and_clears_when_it_is_gone() {
    let mut model = FilesModel::new();
    model.set_listing(&sample());
    model.select(3);
    assert_eq!(
        model.selected().map(|e| e.name.as_str()),
        Some("file10.txt")
    );
    model.set_sort(FileSort {
        column: FileColumn::Name,
        descending: true,
    });
    assert_eq!(
        model.selected().map(|e| e.name.as_str()),
        Some("file10.txt")
    );
    assert_eq!(
        model
            .selected_index()
            .and_then(|i| model.row(i))
            .map(|r| r.entry.name.as_str()),
        Some("file10.txt")
    );
    model.set_listing(&listing(vec![entry("other", FileKind::File, 1, 1)]));
    assert!(model.selected().is_none());
}

#[test]
fn the_cells_are_made_once_with_sizes_modes_times_and_link_targets() {
    let mut model = FilesModel::new();
    let mut link = entry("current", FileKind::Symlink, 9, 1_709_642_040);
    link.link_target = Some("releases/7".into());
    model.set_listing(&listing(vec![
        link,
        entry("big.bin", FileKind::File, 1_572_864, 1_709_642_040),
        entry("sub", FileKind::Directory, 4096, 0),
    ]));
    let rows: Vec<_> = model.rows().collect();
    assert_eq!(rows[0].cell(FileColumn::Name), "sub");
    assert_eq!(
        rows[0].cell(FileColumn::Size),
        "",
        "a directory shows no size"
    );
    assert_eq!(rows[0].cell(FileColumn::Mode), "drw-r--r--");
    assert_eq!(rows[1].cell(FileColumn::Name), "big.bin");
    assert_eq!(rows[1].cell(FileColumn::Size), "1.5 MiB");
    assert_eq!(rows[1].cell(FileColumn::Modified), "2024-03-05 12:34");
    assert_eq!(rows[2].cell(FileColumn::Name), "current -> releases/7");
}

#[test]
fn ten_thousand_entries_sort_and_filter_within_a_frame_budget() {
    let entries: Vec<FileEntry> = (0..10_000u32)
        .map(|i| {
            let kind = if i % 50 == 0 {
                FileKind::Directory
            } else {
                FileKind::File
            };
            let name = if i % 97 == 0 {
                format!(".dot{i}")
            } else {
                format!("entry-{i:05}")
            };
            entry(&name, kind, u64::from(i) * 7 % 100_000, i64::from(i))
        })
        .collect();
    let mut model = FilesModel::new();
    let started = std::time::Instant::now();
    model.set_listing(&listing(entries));
    model.set_sort(FileSort {
        column: FileColumn::Size,
        descending: true,
    });
    model.set_show_hidden(true);
    let elapsed = started.elapsed();
    assert_eq!(model.total(), 10_000);
    assert_eq!(model.len(), 10_000);
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "listing, sorting and filtering 10 000 entries took {elapsed:?}"
    );
    model.set_show_hidden(false);
    assert!(model.hidden() > 0);
}

#[test]
fn landing_a_prepared_listing_is_pointer_moves_and_the_sort_follows_the_model() {
    use crate::files::model::PreparedListing;

    let entries: Vec<FileEntry> = (0..10_000u32)
        .map(|i| {
            entry(
                &format!("entry-{i:05}"),
                FileKind::File,
                u64::from(i),
                i64::from(i),
            )
        })
        .collect();
    let listing = listing(entries);
    let mut model = FilesModel::new();
    // Made where the listing was read (off the UI thread), for the sort and filter in force.
    let prepared = PreparedListing::new(&listing, model.sort(), model.show_hidden());
    let started = std::time::Instant::now();
    model.install(&prepared);
    let landing = started.elapsed();
    assert_eq!(model.len(), 10_000);
    assert!(
        landing < std::time::Duration::from_millis(2),
        "landing a prepared listing on the UI thread took {landing:?}"
    );

    // A listing prepared for another sort is sorted again as it lands.
    let by_size = PreparedListing::new(
        &listing,
        FileSort {
            column: FileColumn::Size,
            descending: true,
        },
        false,
    );
    model.install(&by_size);
    assert_eq!(
        model.row(0).map(|r| r.entry.name.as_str()),
        Some("entry-00000")
    );
    assert_eq!(model.sort().column, FileColumn::Name);
}
