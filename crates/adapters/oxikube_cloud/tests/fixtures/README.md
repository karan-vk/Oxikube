# Recorded CLI output

Each folder holds what one CLI version printed, one file per command, named after it:
`aws-2.15/`, `gcloud-472.0.0/`, `az-2.61/`. The adapters' tests replay these through
`oxikube_cloud::testing::RecordedRunner`; nothing here is ever sent anywhere.

## Re-recording (by hand; never run in CI)

1. Install the CLI version named by the folder (or make a new `<cli>-<version>/` folder).
2. Run the command with the flags the adapter uses (see the adapter's `tests.rs` for the exact
   argument lists) against a throwaway account, with `-o json` / `--format=json` /
   `--output json`, and save stdout to the file the tests name.
3. Anonymise before committing: AWS account ids become `111111111111`, `222222222222`,
   `333333333333`; subscription, tenant and object ids become `1111...`-style or
   `00000000-0000-0000-0000-00000000000N` UUIDs; emails become `@example.com`; endpoints use
   the documentation ranges (`198.51.100.0/24`); certificates are short dummy base64.
4. `cargo test -p oxikube_cloud --test fixtures` fails when a file no longer parses or still
   holds a 12-digit id, a real-looking UUID or a credential prefix (`AKIA`, `ya29.`, `eyJ`).
5. When the CLI changed its output, update the parsers and the expected values in the
   adapter tests in the same commit, and note the new CLI version in
   `docs/testing/cloud-smoke.md`.
