# Cloud discovery: manual smoke checklist

CI tests cloud discovery on recorded CLI output and fakes; it cannot log in to a cloud. Run this
checklist against real accounts before a release that touches `oxikube_cloud`, the
`CloudDiscoveryService` or the Cloud section. Use a throwaway or read-only account, and never
paste account ids, ARNs, tenant ids or tokens into an issue or fixture (anonymise first, see
`crates/adapters/oxikube_cloud/tests/fixtures/README.md`).

Open the section from the catalog toolbar "Cloud" button or, in an empty catalog, the
"Discover cloud clusters" button.

## Versions tested

| Date | Tester | Oxikube | aws | gcloud (+ gke-gcloud-auth-plugin) | az (+ kubelogin) | OS |
|---|---|---|---|---|---|---|
| | | | | | | |

## Per provider

Do each row for AWS (EKS, `aws`), Google Cloud (GKE, `gcloud`) and Azure (AKS, `az`).

| # | Step | Expected result | Pass |
|---|---|---|---|
| 1 | CLI not installed (rename it on `PATH`) | Provider is hidden, or shows "not installed" with the install hint; nothing hangs | |
| 2 | Installed, not logged in (`aws sso logout`, `gcloud auth revoke`, `az logout`) | Provider shows "needs login" with the login command (`aws sso login`, `gcloud auth login`, `az login`); no cluster list | |
| 3 | Log in, press Refresh | Spinner, then clusters with name, region, version, status; the CLI process count stays small (see Activity Monitor) | |
| 4 | Press Refresh again within the cache TTL | Cached list shows at once; a background refresh follows | |
| 5 | Add a running cluster | It appears in the catalog at once; `<config dir>/kubeconfigs/<cli>.yaml` exists with mode 0600 and holds an `exec` user, no token | |
| 6 | Add the same cluster again | "already in the catalog"; the file is unchanged | |
| 7 | Connect to the added cluster | Connects through the exec plugin; namespaces list | |
| 8 | MFA / SSO prompt | The browser or terminal prompt opens (interactive mode); Oxikube waits, then connects; cancelling shows an auth error, not a hang | |
| 9 | Auth plugin missing (`gke-gcloud-auth-plugin`, `kubelogin`; `aws` needs none) | Hint with the install command; adding is refused with the same text | |
| 10 | Session expired while connected (expire the SSO session) | The connect or the next call fails with an auth message naming the login command; logging in and retrying works | |
| 11 | Slow CLI (throttle the network, or put a `sleep 20` wrapper first on `PATH`) | The UI stays responsive; Cancel stops the refresh and the CLI processes exit | |
| 12 | Several profiles / projects / subscriptions, one without access | The accessible ones list; the other shows its own error; the section does not fail as a whole | |
| 13 | Remove the added cluster | It leaves the catalog; an open session is disconnected; the file is deleted with its last cluster | |
| 14 | Search the app logs and `kubeconfigs/` for the token or password you used | Nothing found | |

## Agent tool

With an agent connected: ask it to call `cloud.discover_clusters` with `{"provider": "aws"}`.
It must answer from the cache (no CLI process starts), list name, region, version, status and
`in_catalog`, and show no endpoint, certificate or exec argument. With `"refresh": true` one
refresh runs.

## Notes

Record failures with the CLI version, the step number and the (redacted) error text.
