# Argo CD integration research

Research date: 2026-10-03. Everything read-only via `curl`/`gh api` (no clones). Two parts as delivered.

---

## PART 1 of 2 (inventory, API surface, auth)

### 0. Versions (verified via gh api, 2026-10-03)
- Argo CD: latest stable v3.5.3 (2026-09-14). Also v3.4.9 (2026-09-14), v3.3.14 (2026-08-12). v3.6.0-rc1 is out (2026-09-16, prerelease). Target the 3.x REST API; there is no v4.
- Argo Rollouts v1.10.0 (2026-08-27). Image Updater v1.3.0 (2026-08-13). argocd-agent (argoproj-labs) is active ("redefining the multi cluster story", pushed 2026-10-02).
- swagger.json (assets/swagger.json @v3.5.3) is Swagger 2.0 with 82 paths and 270 definitions. No securityDefinitions are declared, but Bearer auth works.

### 1. Feature -> UI location -> REST endpoint(s) -> CLI -> CRD-direct?
Notes:
- All REST paths are under /api/v1.
- Most application calls accept the optional query params `appNamespace` and `project`. Passing `project` also avoids 403-vs-404 info leaks.
- "CRD-direct" means doable with only the k8s API on argoproj.io/v1alpha1. "Partial" means it needs extra live-cluster reads. "No" means it needs argocd-server or repo-server.

UI routes (ui/src/app/app.tsx): /applications, /applicationsets, /settings, /user-info, /help.

| Feature | UI location | REST | CLI | CRD-direct |
|---|---|---|---|---|
| List apps | /applications (tiles, table "list", summary views) | GET /applications (query: name, refresh, projects, resourceVersion, selector, repo, appNamespace, project) | `app list -o json\|yaml\|wide\|name`, `-l selector` | Yes: list/watch applications.argoproj.io |
| List filters | Sidebar: projects, sync, health, autoSync, operation, namespaces, targetRevision, clusters, labels, annotations, repos, search, favorites | Client-side (selector/projects/repo are server-side params) | `app list -l`, `--project`, `-r repo` | Yes (client-side) |
| Live app updates | Applications list and detail | GET /stream/applications (Watch, params as List + resourceVersion) | n/a | Yes: k8s watch |
| App detail, resource tree | Tree, Network, Pods, List views | GET /applications/{app}/resource-tree; GET /stream/applications/{app}/resource-tree; GET /applications/{app}/managed-resources | `app get -o tree\|tree=detailed`, `app resources` | Partial: `status.resources` has only top-level managed resources (see §4) |
| Sync/health status, conditions | Status panel, conditions bar | `status.sync/health/conditions` in GET /applications/{name} | `app get` | Yes |
| Refresh / hard refresh | Refresh button (hard refresh = Ctrl+click/menu) | GET /applications/{name}?refresh=normal\|hard | `app get --refresh\|--hard-refresh` | Yes: annotation `argocd.argoproj.io/refresh: normal\|hard` |
| Sync (opts) | Sync panel: revision, prune, dry-run, apply-only, force, replace, server-side apply, create ns, prune-last, respect-ignore-diff, retry, strategy apply\|hook, selective resources | POST /applications/{name}/sync. Body fields: revision, revisions, sourcePositions, dryRun, prune, strategy{apply{force},hook{force}}, resources[], syncOptions[], retryStrategy{limit,backoff,refresh}, infos[], manifests | `app sync` with `--prune --dry-run --force --replace --server-side --apply-out-of-sync-only --resource G:K:N --label --retry-* --revision(s) --source-names/positions --strategy --preview-changes --async -l -o json` | Yes: set `operation.sync` (§4). The server-side RBAC and sync-window pre-checks are skipped (see §4) |
| Terminate op | "Terminate" | DELETE /applications/{name}/operation | `app terminate-op` | Yes: patch `status.operationState.phase=Terminating`. The Freelens extension does this CRD-direct (verified in its README) |
| History + rollback | Deployment history panel | POST /applications/{name}/rollback {id, prune, dryRun}; GET /applications/{name}/revisions/{rev}/metadata, /chartdetails, /ocimetadata | `app history`, `app rollback ID` | Yes: `status.history[]` plus an `operation.sync` with the old revision/source. The server also refuses rollback when auto-sync is on (the Freelens extension mirrors this) |
| Diff (live vs target) | Resources diff (compact/inline/side-by-side, "show only changed") | GET /applications/{name}/managed-resources (returns per-resource targetState, liveState, normalizedLiveState, predictedLiveState); GET /applications/{appName}/server-side-diff?targetManifests= | `app diff [--local --revision --server-side-diff --refresh --hard-refresh]` (text only, exit code 1 on diff; Secrets are skipped) | No. Target manifests need repo-server rendering. A live-vs-last-applied diff is possible client-side |
| Manifests | App manifests tab | GET /applications/{name}/manifests?revision=&noCache=&sourcePositions=; POST /applications/manifestsWithFiles (stream, upload local files) | `app manifests [--source live\|git]` | No (repo-server) |
| Resource live manifest | Node "Details > Manifest", edit | GET /applications/{name}/resource?namespace&resourceName&version&group&kind | `app get-resource` | Yes: GET the resource from the destination cluster directly |
| Patch / delete resource | Node actions | POST /applications/{name}/resource?patchType=; DELETE /applications/{name}/resource?force&orphan | `app patch-resource`, `app delete-resource` | Yes (with dest-cluster credentials) |
| Resource actions (Lua) | Node menu (restart Deployment, Rollout promote/abort/etc.) | GET /applications/{name}/resource/actions (list/discovery); POST .../resource/actions/v2 {action, group, kind, version, namespace, resourceName, resourceActionParameters} (v1 also exists) | `app actions list`, `app actions run` | No. The Lua is evaluated server-side. Re-implement the common ones as k8s patches (`kubectl rollout restart` equivalent) |
| Events | Events tab | GET /applications/{name}/events?resourceNamespace&resourceName&resourceUID | n/a | Yes: k8s Events API filtered by involvedObject |
| Pod logs | Logs tab and fullscreen | GET /applications/{name}/logs (stream; params namespace, podName, container, follow, tailLines, sinceSeconds, sinceTime, untilTime, previous, filter, matchCase, group, kind, resourceName; add `download=true` for a plain file); GET /applications/{name}/pods/{podName}/logs (older) | `app logs` | Yes: k8s pod log API directly. Argo adds multi-pod aggregation by resource and a regex filter |
| Terminal (exec) | Pod terminal tab | WebSocket GET /terminal?pod&container&appName&projectName&namespace&appNamespace (not under /api). Cookie auth `argocd.token`. Requires `exec.enabled: "true"` in argocd-cm and RBAC `exec, create` | none | Yes: k8s exec directly. Argo adds RBAC mediation only |
| Edit spec / params | Parameters tab (Helm values/params, Kustomize images/name-prefix etc., Directory recurse/jsonnet, Plugin env), Source edit, Summary edit | PUT /applications/{name}/spec?validate=; PATCH /applications/{name} {patch, patchType}; PUT /applications/{application.metadata.name}; POST /applications (create/upsert) | `app set`, `app unset`, `app edit`, `app patch`, `app create`, `app add-source`, `app remove-source` | Yes (validation is lost) |
| App details from repo (for create UI) | Create-app panel: chart/path/revision pickers, Helm values schema, parameters preview | POST /repositories/{source.repoURL}/appdetails; GET /repositories/{repo}/refs; /apps; /helmcharts; /oci-tags | `repo`, `app create` | No (repo-server) |
| Delete (cascade/propagation) | Delete dialog: Foreground / Background / Non-cascading | DELETE /applications/{name}?cascade=&propagationPolicy=foreground\|background | `app delete --cascade --propagation-policy -y` | Yes: finalizers `resources-finalizer.argocd.argoproj.io` (foreground) or `.../background`, then a k8s DELETE. Non-cascade = remove finalizers then delete (§4) |
| Delete confirmation (v3) | Confirm dialog | `argocd.argoproj.io/deletion-approved: <ISO ts>` annotation set via server | `app confirm-deletion` | Yes: annotation |
| Pause/resume auto-sync | Sync policy toggle | spec.syncPolicy.automated via spec update | `app set --sync-policy none\|automated`, `--auto-prune --self-heal` | Yes |
| Sync windows | Project windows; app shows blocked state | GET /applications/{name}/syncwindows; GET /projects/{name}/syncwindows | `proj windows list\|add\|...` | Yes (read; the evaluation is client-side cron logic) |
| Hooks / waves | Op state panel, resource tree (hook nodes with phase) | `status.operationState.syncResult.resources[]` has hookPhase, syncPhase, and wave via the live annotation | `app wait`, `app get --show-operation` | Yes (read) |
| Links | App/resource link icons | GET /applications/{name}/links; GET /applications/{name}/resource/links; GET /projects/{name}/links | n/a | Partial: `status.summary.externalURLs` and `link.argocd.argoproj.io/*` annotations are readable |
| Wait/health | n/a | poll Get | `app wait [--sync --health --operation --suspended --degraded]` | Yes (a watch) |
| ApplicationSets list/detail | /applicationsets (alpha UI since v3.5.0; tiles, table, summary) | GET /applicationsets; GET /applicationsets/{name}; GET /applicationsets/{name}/resource-tree; GET /applicationsets/{name}/events; GET /stream/applicationsets | `appset list\|get` | Yes |
| AppSet create/delete/preview | n/a | POST /applicationsets (upsert flag); DELETE /applicationsets/{name}; POST /applicationsets/generate (dry-run render). There is NO PUT/update RPC | `appset create [--upsert --dry-run]`, `appset delete`, `appset generate` | Create/delete/update: Yes (CRD apply). Generate preview: No (the controller does the generation; `argocd appset generate` calls the server) |
| AppSet generators & strategy | n/a | Spec: clusterDecisionResource, clusters, git, list, matrix, merge, plugin, pullRequest, scmProvider, selector. Also rollingSync (progressive syncs, beta since v3.3), goTemplate, preservedFields, ignoreApplicationDifferences, templatePatch | n/a | Yes (read/write the CR). Generated apps are found by ownerReferences |
| Projects | /settings/projects: roles, JWT tokens, groups, policies, sync windows, events, source/dest restrictions | GET/POST /projects; GET/DELETE /projects/{name}; PUT /projects/{project.metadata.name}; GET /projects/{name}/detailed (adds global project inheritance); /events; /globalprojects; /links; /syncwindows; POST /projects/{project}/roles/{role}/token; DELETE .../token/{iat} | `proj create\|get\|list\|edit\|set\|delete`, `proj add-source\|add-destination\|allow-cluster-resource\|deny-...\|add-orphaned-ignore\|add-signature-key\|add-source-namespace\|add-destination-service-account`, `proj role *`, `proj windows *`, `proj source-integrity git policies *` | Yes for all of the spec (AppProject CR). JWT token creation needs the server (it signs them) |
| Repositories | /settings/repos (connect git/helm/oci; https user/pass, ssh key, GitHub App, TLS client cert, proxy, `enableOCI`, `insecureOCIForceHttp`, project-scoped) | GET/POST /repositories; GET/PUT/DELETE /repositories/{repo}; POST /repositories/{repo}/validate; also /write-repositories (write-back repos) | `repo add\|list\|get\|rm` | Partial: they are Secrets labeled `argocd.argoproj.io/secret-type: repository`. Create/delete is possible CRD-direct. Connection status/validation needs the server |
| Repo creds templates | same page, "credential templates" | /repocreds, /write-repocreds (CRUD) | `repocreds add\|list\|rm` | Partial: Secret label `repo-creds` |
| Clusters | /settings/clusters | GET/POST /clusters; GET/PUT/DELETE /clusters/{id.value}; POST .../invalidate-cache; POST .../rotate-auth | `cluster add\|list\|get\|rm\|set\|rotate-auth` | Partial: Secret label `cluster`. Status/info (server version, apps count, cache info, connection state) comes from the controller's Redis cache |
| Certificates (TLS/SSH known hosts) | /settings/certs | GET/POST/DELETE /certificates | `cert add-tls\|add-ssh\|list\|rm` | Partial: ConfigMaps argocd-tls-certs-cm, argocd-ssh-known-hosts-cm |
| GPG keys | /settings/gpgkeys | GET/POST/DELETE /gpgkeys; GET /gpgkeys/{keyID} | `gpg add\|list\|get\|rm` | Partial: ConfigMap argocd-gpg-keys-cm |
| Accounts | /settings/accounts, /user-info | GET /account; GET /account/{name}; PUT /account/password; POST /account/{name}/token; DELETE /account/{name}/token/{id}; GET /account/can-i/{resource}/{action}/{subresource} | `account list\|get\|update-password\|generate-token\|delete-token\|can-i\|get-user-info\|session-token` | No (the server owns bcrypt/JWT) |
| Settings / RBAC / overrides | /settings (overview) | GET /settings (UNAUTHENTICATED; returns url, oidcConfig, dexConfig, execEnabled, appsInAnyNamespaceEnabled, hydratorEnabled, impersonationEnabled, installationID, trackingMethod, kustomizeVersions, resourceOverrides, plugins, uiBanner*, passwordPattern, userLoginsDisabled, statusBadge*, controllerNamespace, appLabelKey); GET /settings/plugins | `admin settings *`, `configure` | Yes: ConfigMaps argocd-cm and argocd-rbac-cm (readable if RBAC allows) |
| RBAC policy | n/a (ConfigMap) | n/a | `admin settings rbac can\|validate`, `account can-i` | Yes: `argocd-rbac-cm` (policy.csv, policy.default, scopes) |
| Notifications | no dedicated UI | GET /notifications/services, /templates, /triggers (list only, no mutation) | `admin notifications template get\|notify`, `admin notifications trigger get\|run` | Yes: argocd-notifications-cm + secret; subscriptions are annotations `notifications.argoproj.io/subscribe.<trigger>.<service>: <recipients>` |
| Image Updater | none in the stock UI | none | none (own binary) | Yes: ImageUpdater CR (group argocd-image-updater.argoproj.io, CRD file imageupdaters.yaml verified). v1.x is CRD-driven. It writes back via the Argo CD API or git |
| UI extensions | Resource tab, system-level sidebar page, status panel widget, top bar action (JS served at /extensions.js) | Proxy extensions: /extensions/<name>/... (RBAC `extensions, invoke`; config in argocd-cm `extension.config`) | none | N/A. The JS UI extensions cannot be hosted natively; only backend proxy calls could be |
| Badges | n/a | /api/badge | n/a | n/a |
| Version | footer | GET /version (unauthenticated) | `argocd version` | Yes: the argocd-server image tag/pods |
| Session | Login page | POST /session {username,password} -> {token}; DELETE /session; GET /session/userinfo | `login`, `logout`, `relogin`, `context`, `configure` | N/A |
| Admin / CLI-only | none | none | `admin export\|import\|initial-password\|redis-initial-password\|dashboard\|cluster *\|app diff-reconcile-results\|generate-spec\|get-reconcile-results\|proj generate-allow-list\|settings validate\|resource-overrides health\|ignore-differences\|list-actions\|run-action`, `completion`, `plugin` (kubectl-style plugin discovery). The old `argocd-util` is folded into `argocd admin` | Mostly via k8s |

Other v3.x features to surface:
- Source Hydrator: `status.sourceHydrator`, annotation `argocd.argoproj.io/hydrate`.
- Multiple sources: `spec.sources[]`.
- OCI sources.
- Source integrity / GPG.
- Server-Side Diff (stable since v3.1).
- Per-app `skip-reconcile` annotation (`argocd.argoproj.io/skip-reconcile`).
- Applications in any namespace (`appNamespace`; `appsInAnyNamespaceEnabled` in settings).
- Impersonation / destination service accounts.
- Pre/PostDelete hooks (finalizers prefixed `PreDeleteFinalizerName` / `PostDeleteFinalizerName`).

### 2. API surface (REST via grpc-gateway on the same port as gRPC and gRPC-web)
Services and .proto files (in repo): server/{account,application,applicationset,certificate,cluster,events,gpgkey,notification,project,repocreds,repository,session,settings,version}/*.proto, plus reposerver/repository/repository.proto (internal).

Counts of REST paths per service:
- ApplicationService: about 35
- ApplicationSetService: 8
- ProjectService: 13
- RepositoryService: about 16, including the write-repositories variants
- RepoCredsService: 8 including write-repocreds
- ClusterService: 7
- AccountService: 6
- SessionService: 3
- SettingsService: 2
- NotificationService: 3
- CertificateService: 3
- GPGKeyService: 4
- VersionService: 1

The full list of method+path pairs is in the table above. Items not in the table:
- DELETE /applications/{name}/operation (terminate).
- GET /applications/{name}/syncwindows.
- POST /applications/manifestsWithFiles (client stream).

Transport details (all verified in source):
- One port serves HTTP/1.1 REST, HTTP/2 gRPC (cmux matches `content-type: application/grpc`) and gRPC-web (improbable-eng grpcweb wrapper, e.g. `application/grpc-web+proto`). TLS is on by default with a self-signed cert (`--insecure` flag on the CLI; `--plaintext` when TLS is disabled on the server).
- gRPC-web is the escape hatch behind HTTP/1.1-only proxies (`--grpc-web`, `--grpc-web-root-path`). It is only needed for gRPC clients. REST works through any proxy.
- Streaming REST endpoints (Watch, WatchResourceTree, PodLogs, appset Watch) use `github.com/argoproj/pkg/grpc/http` forwarders. Wire format depends on the Accept header:
  - `Accept: text/event-stream` (exact match): Server-Sent Events. Content-Type text/event-stream, framing `data: {json} \n\n`, keepalive comment lines `:\n`. The web UI uses this via EventSource.
  - Otherwise: grpc-gateway default, newline-delimited JSON. Each line is `{"result": {...}}`, or `{"error": {...}}` on a stream error (the swagger envelope is "Stream result of ...", with `error` as runtimeStreamError).
  - Watch events are `{type: ADDED|MODIFIED|DELETED|BOOKMARK?, application: <Application>}`. Log events are `{content, timeStamp, timeStampStr, last, podName}`.
  - Dropped/closed streams must be re-established by the client, passing `resourceVersion` to resume.
- An ordinary unary List can be shaped with a `fields` filter. The Argo forwarders have `UnaryForwarderWithFieldProcessor` and `processApplicationListField`, so a `fields` query param is accepted (verified in forwarder_overwrite.go). The exact param name was not directly confirmed; test live.
- Terminal: WebSocket at /terminal (a separate mux route, not under /api). Required query params: pod, container, appName, projectName, namespace (appNamespace optional). Auth: cookie `argocd.token` (common.AuthCookieName). Bearer in a WS upgrade from a non-browser client has NOT been verified; the server code reads only the cookie (`getToken(r)` -> JoinCookies). So a Rust client must set `Cookie: argocd.token=<jwt>`. Messages are JSON `{operation, data, rows, cols}` (TerminalMessage struct verified). Operations "stdin"/"stdout"/"resize" are from memory of the protocol, so verify in a spike. It sends pings; there is a `TerminalCommand{Code}` message for reconnect.
- Pod exec requires exec.enabled=true in argocd-cm (settings `execEnabled` tells you) plus RBAC `exec, create, <project>/<app>`.
- Other mux routes: /api/webhook (git webhooks), /api/badge, /api/dex/* (Dex reverse proxy), /auth/login + /auth/callback (SSO for the browser UI), /download (CLI binaries), /swagger-ui, /extensions.js, /extensions/*.
- RBAC (rbac.md): resources applications, applicationsets, clusters, projects, repositories, accounts, certificates, gpgkeys, logs, exec, extensions, with actions get/create/update/delete/sync/action/override/invoke. Fine-grained sub-resource form `update/<group>/<kind>/<ns>/<name>`. Use GET /account/can-i/{resource}/{action}/{subresource} to pre-flight and gray out UI actions.

### 3. Auth matrix
| Method | How it works (verified unless noted) | Desktop-client implication |
|---|---|---|
| Local user | POST /api/v1/session {"username","password"} -> {"token": JWT}. Then `Authorization: Bearer <jwt>` (preferred), gRPC metadata key `token`, or cookie `argocd.token`. Lockout: 5 failed attempts by default (`ARGOCD_SESSION_FAILURE_MAX_FAIL_COUNT`) | Implement natively; never store the password. `admin` bootstrap password lives in Secret `argocd-initial-admin-secret` (`argocd admin initial-password`) |
| API token (account) | `argocd account generate-token --account X` or POST /api/v1/account/{name}/token. Needs `accounts.X: apiKey` in argocd-cm. Long-lived JWT, revocable (DELETE .../token/{id}). Env `ARGOCD_AUTH_TOKEN`; flag `--auth-token` | Best for headless/stored credentials. Store in OS keychain |
| Project role token | POST /projects/{project}/roles/{role}/token -> JWT scoped to the project role's policies (revoke by `iat`) | Useful for least-privilege |
| SSO / OIDC / Dex | CLI `argocd login --sso`: OAuth2 authorization-code + PKCE (S256), NOT device flow. Starts a local HTTP listener on `--sso-port` (default 8085), redirect URL `http://localhost:8085/auth/callback` (override with `--callback`). Reads GET /api/v1/settings (oidcConfig: issuer, clientID, cliClientID, scopes, ...) to build the request, requests `access_type=offline`, stores the ID token as `auth-token` plus a `refresh-token` in `~/.config/argocd/config`. Config: `oidc.config` with `cliClientID`, `enablePKCEAuthentication`, `refreshTokenThreshold`, `requestedScopes` | Reproduce natively in Rust (`openidconnect`/`oauth2` crates, loopback listener, open system browser). The IdP must allow the loopback redirect URI for the CLI client, or the admin must set `cliClientID`. Dex-issued tokens don't refresh via the embedded-web flow (docs warning), and the server can renew tokens (response header `renewTokenKey`). The Dex CLI client id `argo-cd-cli` is from memory; verify |
| Existing CLI session reuse | Read `~/.config/argocd/config` (YAML: current-context, contexts[{name,server,user}], servers[{server, insecure, grpc-web, grpc-web-root-path, plain-text, client-certificate-data, client-certificate-key-data, core}], users[{name, auth-token, refresh-token}], prompts-enabled). Verified struct in util/localconfig | Offer "import from argocd CLI config" for zero-friction onboarding. Treat as read-mostly; don't write back |
| mTLS client cert | `--client-crt/--client-crt-key`; config fields above | Support optional |
| Extra headers | `-H/--header` on all requests (for auth proxies) | Support custom headers per profile |
| Core mode (no server) | `argocd login --core` or `--core` flag. No Argo CD auth at all; Kubernetes RBAC only (§4b) | Our k8s credentials are the identity |
| Port-forward | `--port-forward [--port-forward-namespace]` finds a pod labeled for argocd-server and port-forwards to it (random local port) | Implement with kube-rs `Api<Pod>::portforward` (or Service proxy) and then use the Remote-server mode over loopback (plaintext/insecure TLS) |
| TLS | self-signed default; `--insecure`, `--server-crt`, `--plaintext`, `--grpc-web` | Per-profile TLS options; pin the cert; allow a custom CA |
| Anonymous | `users.anonymous.enabled` in argocd-cm: GET endpoints work without a token. /api/version and /api/v1/settings are open anyway | Detect via userinfo (`loggedIn: false`) |

SESSION HYGIENE: JWT expiry (default 24h for local/SSO sessions) must be handled with re-login prompts; for OIDC use the refresh token.

---

## PART 2 of 2 (core mode, CRD-direct vs server matrix, Rust options, other clients, Rollouts, recommendation, risks)

Corrections to part 1 (unverified items there):
- The 24h JWT default expiry is from memory.
- The Watch event-type list (`BOOKMARK?`) is not confirmed.

### 4. CRD-direct mode (Application, ApplicationSet, AppProject in argoproj.io/v1alpha1)

#### 4a. What the CRs carry (types verified in pkg/apis/application/v1alpha1/types.go @v3.5.3)
ApplicationStatus fields: resources[], sync, health, history[], conditions[], reconciledAt, operationState, observedAt (deprecated), sourceType, sourceTypes, summary (externalURLs + images), resourceHealthSource (inline|appTree), controllerNamespace, sourceHydrator.
- `status.resources[]` is `ResourceStatus`: group, version, kind, namespace, name, status (sync), health, hook, requiresPruning, syncWave, `requiresDeletionConfirmation`. It lists only the top-level resources the app manages. It does NOT include child objects (ReplicaSets, Pods, Endpoints, etc.).
- If `resourceHealthSource` is `appTree`, per-resource health is NOT inline in `status.resources`; it moves to the tree in Redis. A CRD-only client then cannot show per-resource health. Watch for this field (it is an operator opt-in flag, controller-side).
- `status.operationState`: operation, phase (Running/Succeeded/Failed/Error/Terminating), message, syncResult (resources[] with hookPhase/syncPhase, revision(s), source(s)), startedAt, finishedAt, retryCount.
- `status.history[]`: id, revision(s), source(s), deployedAt, deployStartedAt, initiatedBy.
- `status.conditions[]`: type, message, lastTransitionTime (e.g. OutOfSync, ComparisonError, SyncError, InvalidSpecError, OrphanedResourceWarning...).
- `spec.operation` (Operation): sync{revision, revisions, prune, dryRun, syncOptions[], syncStrategy{apply{force}|hook{force}}, resources[], source, sources, manifests}, initiatedBy{username|automated}, info[], retry{limit, backoff, refresh}.
- ApplicationSetStatus: applicationStatus[] (progressive sync per app), conditions, health, resources, resourcesCount.

#### 4b. Actions achievable purely with the k8s API (the same patterns the Freelens extension and the k9s plugin use; the k9s plugin comment says all it needs is get/patch on applications.argoproj.io)
- Refresh: annotate `argocd.argoproj.io/refresh: normal` or `hard` (constant AnnotationKeyRefresh verified). The controller consumes and removes it.
- Hydrate: `argocd.argoproj.io/hydrate`.
- Sync: write `operation` onto the Application (a merge patch). The controller (not the server) executes it and writes status.operationState. This is what `kubectl patch app X --type merge -p '{"operation":{"sync":{...}}}'` does; if one is already running, the apply fails or is ignored.
- Terminate: set `status.operationState.phase: Terminating` (Freelens extension does it; Application CRD has no status subresource so a normal patch works).
- Rollback: `operation.sync` with `revision`/`source(s)` taken from `status.history[id]` (the server's Rollback RPC does this; also blocks when auto-sync is on).
- Edit spec (params, source, syncPolicy, destination): ordinary patch/update.
- Delete: cascade = add finalizer `resources-finalizer.argocd.argoproj.io` (foreground, default) or `resources-finalizer.argocd.argoproj.io/background`, then DELETE. Non-cascading = remove all finalizers (`{"metadata":{"finalizers":null}}`) first, then DELETE. Delete confirmation = annotation `argocd.argoproj.io/deletion-approved: <ISO timestamp>` (plus sync option `Delete=confirm`). All verified in app_deletion.md and sync-options.md.
- Pause auto-sync: spec.syncPolicy.automated: null.
- Create/update/delete Application, ApplicationSet, AppProject: CR CRUD. Projects: roles/policies/groups/windows/destinations are plain spec fields. ApplicationSet refresh: annotation `argocd.argoproj.io/application-set-refresh` (set by webhook; controller removes it).
- Repos/clusters/repo-creds: Secrets labeled `argocd.argoproj.io/secret-type: repository|repo-creds|cluster`, plus ConfigMaps for certs, GPG, known hosts, `argocd-cm`, `argocd-rbac-cm`, `argocd-cmd-params-cm`, `argocd-notifications-cm`.

#### 4c. What is NOT possible CRD-direct
- Target (git/helm/oci) manifests, diff against desired state, server-side-diff, manifest preview/generation.
- Repo browsing (refs, apps in repo, helm charts, OCI tags, appdetails + Helm values schema).
- Full resource tree. Tree nodes (children, pods) live in the controller's Redis cache (key `app|resources-tree|<app>`, optional shard suffix; managed resources `app|managed-resources|<app>`; compressed per `--redis-compress`, default gzip). Workaround: rebuild the tree client-side from `status.resources` plus live objects and ownerReferences, using our own cluster access (we already do this for Workloads views). Per-resource health for children is computed from live objects client-side (not Argo's Lua health). The result will approximate but not match Argo's tree.
- Resource actions (Lua), custom health checks and discoveries (`resource.customizations.*` in argocd-cm).
- Token generation (accounts, project roles), password changes, session/RBAC evaluation, `can-i`.
- Repo/cluster connection state and validation, cluster cache info, cache invalidation, credential rotation (`rotate-auth`).
- ApplicationSet `generate` preview.
- Server-side validation on spec edits (project permission, repo access, destination existence): a CRD-direct write bypasses it; the controller later raises InvalidSpecError / ComparisonError conditions instead.
- Pre-flight RBAC: with the server, each action has an Argo RBAC check. CRD-direct only has Kubernetes RBAC on the Argo CRs. Sync-window checks for manual sync are done in the server; whether the controller re-checks a CRD-written operation was not verified in this session. Treat as risk.

Capability matrix (Y=yes, P=partial, N=no):
| Capability | CRD-direct | Server API | `--core` local server |
|---|---|---|---|
| List/watch apps, appsets, projects | Y | Y | Y |
| Status, history, conditions, op state | Y | Y | Y |
| Refresh/hard refresh | Y (annotation) | Y | Y |
| Sync (all options) / terminate / rollback | Y (operation) | Y | Y |
| Edit spec | Y (no validation) | Y | Y |
| Delete cascade/non-cascade | Y | Y | Y |
| Resource tree (children, pods) | P (client-side rebuild) | Y | Y (reads Redis via port-forward) |
| Live manifest of a managed resource | Y (dest cluster API; only if we can reach the dest cluster) | Y | Y |
| Diff live vs target | N | Y | Y |
| Target manifests | N | Y | Y |
| Repo browsing / appdetails | N | Y | Y |
| Pod logs | Y (kube API) | Y | Y |
| Pod exec | Y (kube API) | Y (websocket) | Y |
| Resource actions (Lua) | N (re-implement a few) | Y | Y |
| Repos/clusters/creds CRUD | P (Secrets) | Y | Y |
| Account/token mgmt | N | Y | N |
| Notifications config | Y (CM/Secret) | list only | list only |
| RBAC enforcement | k8s RBAC only | Argo RBAC | k8s RBAC only |

#### 4d. `argocd --core` mechanics (verified in cmd/argocd/commands/headless/headless.go)
- It does NOT talk directly to k8s from the CLI. `MaybeStartLocalServer` starts an in-process Argo CD API server on a free localhost port (or `--port`/`--address` for `argocd admin dashboard`) and points the client at it. It sets env `ARGOCD_FAKE_IN_CLUSTER_CONFIG=true` and builds clientsets from the user's kubeconfig (`--kube-context`, current namespace; `kubectl config set-context --current --namespace=argocd`).
- The local server needs two backends, both reached by kube port-forward (`kubeutil.PortForward`):
  1. Redis: pod found by label app.kubernetes.io/name=`argocd-redis` (or `argocd-redis-ha-haproxy` when HA), port 6379. Password read from the argocd-redis Secret (`SetOptionalRedisPasswordFromKubeConfig`). Flags: `--redis-name`, `--redis-haproxy-name`, `--redis-compress`.
  2. repo-server: service labeled for the repo-server component -> pod app label, port 8081 (`--repo-server-name`). Used for manifest generation, diffs, appdetails.
- It also runs a miniredis for the server's own cache; the application state cache is forwarded to the real Redis. A dry-run controller-runtime client is used for some operations.
- Result: full Argo CD API and `argocd admin dashboard` (full web UI at http://localhost:8080) with only kubeconfig credentials; no Argo login; authorization = Kubernetes RBAC on the CRs (docs: "Argo CD RBAC model, Argo CD API, notification controller, OIDC" are not available in the Core install; the CLI/UI are 'partially available').
- Prerequisites on the cluster: argocd-application-controller, argocd-repo-server, redis (Core install = manifests/core-install.yaml: no server, no dex, no RBAC). The same trick works against a full install too (flag `--core` bypasses argocd-server).
- Consequence for Oxikube: the "no server" mode is not CRD-only; achieving parity requires either (a) spawning the `argocd` binary (`argocd admin dashboard --port N` or any `--core` command; the local API is then reachable over loopback), or (b) re-implementing the same port-forward-to-Redis and port-forward-to-repo-server plumbing natively in Rust (Redis key/compression format, and repo-server gRPC with the internal repository.proto; unversioned internal APIs, TLS to repo-server by default: fragile), or (c) accepting reduced capabilities (the CRD-direct column above).

### 5. Rust client options
- crates.io/GitHub survey (verified via crates.io API + gh search): there is no maintained Argo CD API client crate.
  - `argo-crds` 0.3.1 (atcol, MIT, last update 2023-12): Rust models for Argo Events/Workflows, 4.5k downloads; stale. `argo-cd-crds` 0.1.0 (2022): stale.
  - `pipedash-plugin-argocd` 0.1.1 (GPL-3.0, tied to the Pipedash app, ~300 downloads); `lazyargo` 0.0.2 (read-only TUI, 35 downloads, 2026-08); `Saeden/argocd-client` (0 stars, 2026-07) and `WilliamAkaWill/argocd-grpc` (0 stars) are experiments. None of these should be depended on; they could serve as reading material only. Check the licenses before borrowing anything (pipedash is GPL-3.0).
- Building blocks that DO exist (versions verified on crates.io):
  - `kube` 4.2.0 (2026-07), `k8s-openapi` 0.28.0: CRD-direct mode, dynamic API (`DynamicObject`) is enough for the Argo CRs; typed structs can be generated with `kopium` 0.24.1 from the official CRD YAMLs (manifests/crds in argo-cd) and kept in sync per version.
  - `progenitor` 0.15.0 (OpenAPI 3.0.x only per its README; "may fail for some documents"). The Argo swagger is 2.0, so it needs a 2.0->3.0 conversion first, and the document has 270 definitions with a huge recursive v1alpha1 Application plus embedded k8s types. Streaming endpoints are described as unary, so they would be mis-modelled.
  - OpenAPI Generator's `rust` generator (beta) with reqwest/hyper libs: also works from Swagger 2.0 but yields a huge generated surface.
  - `tonic` 0.14.6 for gRPC. The .protos import gogoproto and k8s generated.proto (pkg/apis/application/v1alpha1/generated.proto), so you must vendor/compile those; REST covers every method, so gRPC buys nothing except lower overhead (and no bidi streams exist; the terminal is a websocket).
  - `tokio-tungstenite` 0.30.0 (terminal websocket; custom Cookie header). `reqwest-eventsource` 0.6.0 (last update 2024-03) and `eventsource-stream` 0.2.3 (2022) are stale; SSE framing here is trivial (`data: <json> \n\n`, `:\n` keepalives), so hand-parse it over `reqwest::Response::bytes_stream`. Or skip SSE entirely: omit `Accept: text/event-stream` and you get NDJSON `{"result":...}` lines.
- Shelling out to `argocd` (viable but second-best):
  - Pros: reuses auth (CLI config/SSO refresh), `--core` and `--port-forward` plumbing, `-o json|yaml` on `app get`, `app list`, `app sync -o json`, `appset`/`proj`/`repo`/`cluster` commands; includes `app diff` and `app manifests`.
  - Cons: binary distribution/version skew (CLI must match server within skew; `argocd version`), no JSON for `app diff` (text only with external-diff env), no streaming APIs, per-command process cost, interactive prompts (`--prompts-enabled`, `--assumeYes`), parsing text tables, license/redistribution (Apache-2.0, fine), logs/exec need the websocket anyway.
  - Best use: keep it as an optional "core-mode helper" (spawn `argocd admin dashboard --port N` / `--core`, then talk REST to it) rather than as the primary adapter.

### 6. How other clients integrate (verified from docs/READMEs)
- Lens (premium): Argo CD entry in the cluster navigator, shown when the CRDs are present (cluster reconnect needed). Dashboard (health strip, summary cards, needs-attention panel, event timeline), list/detail views for Applications (Name, Namespace, Sync, Health, Project, Destination, Revision, Age), ApplicationSets, Projects, Settings>Repositories, Settings>Clusters. Actions: Sync (optional prune), Refresh (normal/hard), bulk sync/refresh, "Open Argo CD UI" without manual auth, AI summarise. Connection: "reads directly from in-cluster Kubernetes data", i.e. CRD-direct, no Argo server API. Permissions are not documented.
- Freelens extension `Sebastian-Prokesch/freelens-argocd-extension` (MIT, updated 2026-09-13, validated against Argo CD 3.4/3.5): Argo hub with ArgoCD + Rollouts + early Workflows pages. Default CRD-direct; OPTIONAL named Argo CD API server connections in preferences (so lists/sync work against a remote server). Actions: refresh, hard refresh, sync, advanced sync, terminate, rollback via Sync History (blocked when auto-sync is on), sync-policy toggle (automated/prune/self-heal/allow-empty), drift hotspots, operation timeline, resource diff (CRD-only summary), Rollouts promote/promote-full/skip-current/skip-all/abort/retry. Documented limits: no API-backed line-level diff, no API-backed resource tree, no repo/cluster secret management via API, Test-connection only for its own API connection. This is the closest design precedent for "optional but integrated".
- Headlamp: `privilegedescalation/headlamp-argocd-plugin` (1 star; read-only; monitors Applications, Rollouts, health; no write ops). Another (RajPrakash681) is empty/0 stars.
- k9s: official `plugins/argocd.yaml` (derailed/k9s) uses only kubectl+jq against the Application CRD with the user's kubeconfig; no argocd CLI; mutating actions marked `dangerous` (hidden when the context is read-only). Also plugin yamls for argo-rollouts and argo-workflows.
- Argonaut (darksworm/argonaut, Go TUI): wraps the `argocd` CLI session (`argocd login` first; reads the persisted token); browse/scope by cluster/namespace/project, stream live status, sync, diff in pager, rollback.
- Pipedash (Tauri desktop app with a Rust backend, GPL-3.0): read-only ArgoCD plugin (pipelines view).
- Takeaways: every existing client is CRD-first; those that need diff/tree/manifests use the server API or CLI. Nobody offers full Argo UI parity natively. Parity is the differentiator but it requires server-API or core-mode backends.

### 7. Argo Rollouts (argoproj/argo-rollouts v1.10.0)
CRDs (manifests/crds verified): Rollout, AnalysisRun, AnalysisTemplate, ClusterAnalysisTemplate, Experiment (group argoproj.io, v1alpha1).
- Rollout spec/strategy: blueGreen (activeService, previewService, autoPromotionEnabled, scaleDownDelay..., pre/postPromotionAnalysis) and canary (steps: setWeight, pause{duration}, analysis, experiment, setCanaryScale, setHeaderRoute, setMirrorRoute; trafficRouting for Istio/NGINX/ALB/SMI/Traefik/Ambassador/Apisix/Gateway API/plugins; stable/canary services; dynamicStableScale; analysis). Also `workloadRef` (reference an existing Deployment).
- RolloutStatus (verified): abort, pauseConditions, controllerPause, abortedAt, currentPodHash, currentStepHash, currentStepIndex, replicas/updated/ready/available, canary{...}, blueGreen{...}, stableRS, restartedAt, promoteFull, phase, message, conditions, alb/albs, HPAReplicas, selector.
- kubectl-argo-rollouts commands (cmd dirs verified): abort, create, dashboard, get (live tree), lint, list (rollouts, experiments), pause, promote, restart, retry, set (image), signals, status, terminate, undo, version, completion.
- Everything is a CRD patch (verified from source):
  - promote: merge-patch `{"spec":{"paused":false}}` + `{"status":{"pauseConditions":null}}`; `--full` = `{"status":{"promoteFull":true}}` (needs the status subresource patch; falls back to a unified patch for old CRDs); `--skip-current-step` sets `status.currentStepIndex` to next step; `--skip-all-steps` to the last step (len(steps)).
  - abort: `{"status":{"abort":true}}`; retry rollout: `{"status":{"abort":false}}`; retry experiment: `{"status":null}`.
  - pause: `{"spec":{"paused":true}}`.
  - restart: `{"spec":{"restartAt":"<RFC3339>"}}`.
  - set image: patch pod template container image (`spec.template.spec.containers[].image`).
  - undo: fetch ReplicaSet with annotation `rollout.argoproj.io/revision` == N, patch Rollout template from it.
  - terminate (analysisrun/experiment): `spec.terminate: true` (not re-verified in this session; from memory).
- Dashboard (`kubectl argo rollouts dashboard`, localhost:3100) is a local process using kubeconfig, not an in-cluster server. It exposes a small API (rollout.swagger.json: list/watch rollout infos, abort, promote, restart, retry, set image, undo, version, namespace); our client doesn't need it: it is a CRD-patch wrapper.
- Argo CD built-in Lua resource actions for Rollout (verified in resource_customizations/argoproj.io/Rollout/actions): abort, pause, promote-full, restart, resume, retry, skip-current-step. Via the server these need `action/argoproj.io/Rollout/<name>` RBAC. CRD-direct equivalents are the patches above.
- Rollouts: CRD-direct is fully sufficient for list/watch/get/promote/abort/retry/pause/restart/set image/undo, status trees (Rollout -> ReplicaSets -> Pods, AnalysisRuns, Experiments through ownerReferences/labels `rollouts-pod-template-hash`), and logs of analysis metric jobs.

### 8. Recommendation (architecture for an OPTIONAL integration)
1. Detection (optional feature gating): the integration is off unless the CRD `applications.argoproj.io` exists on the current cluster, or the user adds a remote Argo CD server profile. Also detect `argocd-server` Deployment/Service (label app.kubernetes.io/name=argocd-server), `argocd-cm` `url`, version from the image tag, Rollouts via `rollouts.argoproj.io` CRD.
2. Three backends behind one `ArgoBackend` trait; the UI asks for capabilities (like Freelens' fallbacks, but explicit):
   - Backend A, CRD-direct (zero extra credentials; works on core installs): `kube` dynamic API + typed structs generated by `kopium` from the CRDs. Covers list/watch/status/sync/refresh/rollback/terminate/delete/spec edit/projects/appsets, plus live-resource views and our own logs/exec. Show "diff, target manifests, repo browsing unavailable" when running only A.
   - Backend B, Server API (REST over HTTPS; credentials: token/SSO/local login; reach via direct URL or native kube port-forward to svc/argocd-server). Hand-written reqwest client for the ~45 endpoints above, with typed models reused from A (the REST JSON embeds the same v1alpha1 Application shape as the CRD, so the same serde types serve both). SSE/NDJSON watch, log streaming, terminal websocket with cookie auth. This delivers full web-UI parity.
   - Backend C, Core helper (optional): detect the `argocd` binary; spawn `argocd admin dashboard --port <free>` (or an equivalent `--core` local server) and point Backend B at `http://127.0.0.1:<port>` with no token. Gives diff/manifests/tree/repo browsing without argocd-server or Argo credentials. A native re-implementation of the Redis+repo-server port-forwards is a later optimisation, not a first step.
3. Keep gRPC out of v1: REST covers everything and works through any proxy.
4. Don't generate the whole client from swagger.json. Hand-write the client for the subset; if desired, use swagger.json purely as a conformance check or to generate request/response structs with `typify`/openapi-generator for small leaf types. Pin per-minor-version fixtures (v3.3/v3.4/v3.5) for tests.
5. Auth: import CLI config profiles; support token entry; implement SSO PKCE loopback natively (`openidconnect`); store secrets in the OS keychain; honour `--insecure`/`--plaintext`/`--grpc-web` equivalents per profile; prompt on 401 to re-login. Use `GET /api/v1/account/can-i/...` to disable actions. Show which backend served each view.
6. Rollouts as a separate CRD-only module (same detection pattern), with the patch recipes of §7.
7. Bundle sizes/time: `kube`/`k8s-openapi` are already needed. New deps: kopium-generated code (build-time), reqwest (already), tokio-tungstenite (already needed for exec if implemented that way), `openidconnect`.

### 9. Risks / open questions
- Version skew: the REST schema evolves per minor (v3.5 alpha ApplicationSet UI, v3.6 rc features: hydrator, progressive sync annotations). Keep every model field optional and tolerate unknown fields (`#[serde(default)]`, flatten to `serde_json::Value`). Test against 3.3, 3.4, 3.5.
- Safety: CRD-direct writes skip Argo server validation and RBAC; a sync/delete from a desktop client can delete production workloads (prune, cascading finalizers). Need confirmation UIs (dry-run first; surface `Delete=confirm` and `requiresPruning`), read-only mode per cluster context (k9s precedent), and audit info (`operation.initiatedBy.username`, `info[]`).
- Redis/repo-server native access is a trap: internal formats unversioned, TLS-by-default to repo-server, auth secrets, compression modes, key sharding (`shardsCount`). Prefer API/binary.
- `resourceHealthSource: appTree` removes inline per-resource health from the CR; CRD-only views degrade in those installs.
- Applications in any namespace: CRD listing must cover all permitted namespaces (`appsInAnyNamespaceEnabled`; `appNamespace` params; `argocd.argoproj.io/managed-by` semantics) and multi-instance installs (multiple Argo CD in one cluster; `controllerNamespace`, `installationID`).
- Destination cluster vs control-plane cluster: Argo apps often deploy to other clusters; live resources/logs/exec for those clusters are not reachable with the current kube context. The server API proxies those; CRD-direct cannot. UI must say so.
- Terminal and logs through argocd-server need `exec.enabled` and RBAC; WS auth is cookie-only server-side (verify Bearer on upgrade in a spike).
- SSO: loopback redirect URI must be pre-registered at the IdP for the CLI client; some orgs forbid it; the `cliClientID`/Dex client id details were from memory. Provide a manual-token fallback.
- Stream resumption semantics (resourceVersion, `ADDED` replay on connect, 504s from proxies/ingress idle timeouts on SSE) need a spike. Argo sends keepalive comments to survive proxies.
- Argo CD Agent (argocd-agent, new topology: principal/agent; apps may be mirrored to workload clusters) will change where CRs live; out of scope but design the cluster-selection layer to allow multiple control planes.
- Licensing: Argo CD is Apache-2.0. We can embed no UI extensions. Lens's Argo support is a paid, closed feature, so there is room to compete with an open one.
- Items NOT verified that need a spike: (1) terminal websocket message operation names; (2) the unary-response `fields` filter param name; (3) whether the controller re-enforces manual sync windows for CRD-written operations; (4) token default lifetime; (5) Dex CLI client id; (6) `terminate` patch for AnalysisRun/Experiment; (7) behaviour of `Accept: text/event-stream` through k8s service proxy.

### 10. 15-line summary
1. Latest stable Argo CD is v3.5.3 (2026-09-14); v3.6.0-rc1 exists; Rollouts v1.10.0; Image Updater v1.3.0.
2. swagger.json is Swagger 2.0, 82 REST paths in 13 services; REST + gRPC + gRPC-web share one port.
3. Streams: watch/log/tree via `Accept: text/event-stream` (SSE, `data: {...} \n\n`) or NDJSON `{"result":...}`; terminal is a cookie-authed websocket at /terminal.
4. Auth: local user session JWT, account/project tokens, SSO is auth-code+PKCE on loopback :8085 (not device flow), `~/.config/argocd/config` is importable.
5. `--core` actually spawns an in-process API server using kubeconfig and port-forwards to repo-server (8081) and Redis (6379); it is not CRD-only.
6. CRD-direct can do list/watch/status, sync (via `operation`), refresh (annotation), terminate, rollback, delete (finalizers), spec edits, projects, appsets.
7. CRD-direct cannot do diff, target manifests, repo browsing, resource actions (Lua), full tree, token/account mgmt.
8. Existing clients (Lens premium, Freelens ext, k9s, Headlamp) are CRD-first; Freelens adds an optional Argo CD API connection.
9. No maintained Rust Argo CD crate exists; `kube` + `kopium` for CRDs; hand-write reqwest for ~45 endpoints; no gRPC in v1.
10. progenitor needs OpenAPI 3.0; swagger 2.0 with recursive k8s types makes full codegen a poor fit.
11. Shelling out to `argocd` works as an optional core-mode helper (`argocd admin dashboard --port N`), not the main adapter.
12. Recommendation: three backends (CRD-direct, Server API, Core helper) behind one trait, capability-gated UI.
13. Rollouts is fully CRD-direct (promote/abort/retry/pause/restart/set image/undo are merge patches, verified in source).
14. Biggest risks: unvalidated destructive writes, destination clusters not reachable, version skew, `resourceHealthSource: appTree`.
15. Unverified items listed in section 9 need a spike against a live server.
