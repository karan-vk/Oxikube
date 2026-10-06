//! Kubeconfig sources management (E06-S05): which kubeconfig files and folders the catalog
//! reads, kept in the user's settings and changed from the sources screen.
//!
//! [`KubeconfigSourcesService`] adds, pastes, removes and reloads sources. The list lives in
//! settings (`kubeconfig.sources`, entries `{ kind: default|file|dir, path }`) behind a
//! [`SourceListStore`]; the service pushes every change to the
//! [`ClusterSourcePort`](oxikube_ports::ClusterSourcePort) with `set_user_sources`, which reloads
//! with the tolerant loader, and reads back per-source statuses
//! ([`SourceRow`]: found with N contexts, missing, blank, or the parse problem) so the screen
//! can show an error next to the one source that has it while the others keep loading.
//!
//! | File | Holds |
//! |---|---|
//! | `service` | [`KubeconfigSourcesService`]: `rows`, `add`, `remove`, `reload`, `apply_stored` |
//! | `list` | [`SourceListStore`] and the in-memory [`MemorySourceList`] |
//! | `row` | [`SourceRow`], the list entry joined with its status |
//! | `name` | the file name of a pasted kubeconfig and the "is this ours" path check |
//! | `commands` | the handlers of `kubeconfig::AddSource`, `RemoveSource` and `Reload` |
//!
//! # Pasted kubeconfigs are files, with `0600`
//!
//! The story asks for a pasted kubeconfig to be stored at `<config dir>/kubeconfigs/<name>.yaml`,
//! so it is: validated first (parsed, no network), written atomically with owner-only
//! permissions through [`FsPort::write_private`](oxikube_ports::FsPort::write_private), and listed
//! as a file source. That is a kubeconfig on disk like the one in `~/.kube`, and it can hold the
//! same credentials, so the paste dialog says so. The keychain route of the adapter
//! (`KubeconfigSources::add_pasted`, E03) stays available and is not used by this screen. The name
//! can never leave the directory ([`pasted_file_name`] allows letters, digits, `-`, `_`, `.`).
//!
//! # Removing
//!
//! Only files Oxikube stored (directly inside the kubeconfigs directory) are deleted. For a file
//! or folder the user keeps elsewhere the entry is removed and nothing else is touched. The
//! screen's confirmation text says which of the two it is ([`KubeconfigSourcesService::deletes_file_on_remove`]).
//!
//! # No secrets in output
//!
//! Nothing here logs or returns kubeconfig text: errors use fixed wording and rows carry paths,
//! counts and notes only.

mod commands;
mod list;
mod name;
mod row;
mod service;

#[cfg(test)]
mod tests;

pub use commands::register_commands;
pub use list::{MemorySourceList, SourceListStore};
pub use name::{MAX_NAME_LEN, is_stored_in, pasted_file_name};
pub use row::SourceRow;
pub use service::{KubeconfigSourcesService, SourceChange};
