# ADR 0015: Pasted kubeconfigs are stored as owner-only files

- **Status:** Accepted (2026-10-06)
- **Deciders:** story E06-S05
- **Related:** non-negotiable 5 (no secrets on disk), E03-S02 (`KubeconfigSources::add_pasted`), issue #99

## Context

E03-S02 stored a pasted kubeconfig in the OS keychain (`SecretStorePort`), because a kubeconfig
can hold tokens and keys. The sources screen story (E06-S05) asks for the opposite: a pasted
kubeconfig is stored at `<config dir>/kubeconfigs/<name>.yaml`, `0600`, and listed as a file
source. Users expect to see, back up and edit such a file like the ones in `~/.kube`, and a
keychain entry cannot be a file source (no path, no folder source, no `kubectl --kubeconfig`).

## Decision

The sources screen stores pasted text as a file through `FsPort::write_private`:

- validated first (parsed through `ClusterSourcePort::validate_kubeconfig`, no network);
- written atomically, created with mode `0600` (parent directories `0700`) so the content is
  never readable by others, not even for a moment;
- the name is limited to letters, digits, `-`, `_` and `.` (no separators, no leading dot), so it
  cannot leave the directory; an existing name is never overwritten;
- listed in `kubeconfig.sources` as a `file` entry; removing the entry deletes the file, and only
  files directly inside the kubeconfigs directory are ever deleted;
- the paste dialog says where the file goes and that it may hold credentials, like the files in
  `~/.kube`.

This is a kubeconfig on disk of the same sensitivity as the user's own `~/.kube/config`, not a
new class of secret: Oxikube still never persists tokens it obtained itself, logs and audit entries
carry no kubeconfig text, and the pasted text is never put in a `Debug` output. The keychain route
of the adapter (`KubeconfigSources::add_pasted`) stays and is not used by this screen; a later
story may add a "keep sensitive fields in the keychain" option on top.

## Consequences

A pasted kubeconfig survives a keychain reset and can be handled like any file. Credentials inside
it are exactly as exposed as those of the user's other kubeconfigs (owner-only permissions on
unix; on Windows the user profile's own access rules apply). Reviewers should reject any code that
writes kubeconfig text anywhere else.

## Alternatives

- Keychain only (E03): satisfies non-negotiable 5 to the letter but cannot be a file or folder
  source, is invisible to other tools, and fails the story's acceptance criterion.
- Encrypt the file with a key in the keychain: extra moving parts for a file whose original the user
  usually still has; revisit if a "sensitive fields in the keychain" story comes.
