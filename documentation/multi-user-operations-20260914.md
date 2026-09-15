# Multi-user application operations

## Deployment

Run exactly one application instance on Linux with a persistent **local filesystem** mounted at `/data`. Embedded Turso is pinned to 0.7.2. The process owns the database; clustering children never open it. The instance lock prevents a second server or stopped-service backup from opening the same data directory.

Build with `docker build -t incident-clustering:multi-user .`. The image runs as UID/GID 10001. Make the mounted directory writable by that identity. Allow up to 8 logical cores and 64 GiB RAM; the image starts one clustering child, six worker threads, four active database writers, and a 4 GiB immutable artifact cache. There is no external database or queue service.

Terminate HTTPS at company ingress and forward to port 8080. Allow uploads of at least 128 MiB, responses large enough for complete analysis JSON, and requests lasting at least 60 seconds. Disable buffering and caching for `/api/changes` and `/api/jobs/*/events`; preserve `Last-Event-ID` and permit long-lived SSE connections. The application also sends `X-Accel-Buffering: no` for shared changes. Mount storage before starting the process; do not mount network storage or run multiple replicas.

The application does not have a development sign-in bypass. Without Entra configuration the static sign-in page and health endpoint work, but protected APIs reject anonymous requests. Test identities and signing keys are compiled only into Rust tests.

## Configuration

| Variable | Meaning / default |
| --- | --- |
| `APP_DATA_DIR` | Persistent application directory; `data` locally, `/data` in the image |
| `CLUSTERING_WEB_BIND` | Listening socket; `127.0.0.1:8080` locally, `0.0.0.0:8080` in the image |
| `APP_DATABASE_WRITERS` | Active database writers, default 4; range 1–16 |
| `APP_CACHE_MIB` | Immutable artifact cache budget, default 4096; range 64–32768 |
| `APP_MAX_QUEUED_JOBS` | Maximum queued plus running clustering jobs, default 10; range 1–100 |
| `APP_JOB_RETENTION_DAYS` | Finished, failed or cancelled unsaved jobs, default 7; range 1–365 |
| `APP_WORKER_THREADS` | Rayon threads; locally CPU count minus two, minimum one; image default 6 |
| `APP_JOB_TIMEOUT_SECONDS` | Child wall-clock limit, default 3600; range 60–86400; timed-out jobs fail and require explicit resubmission |
| `APP_HEAVY_REQUESTS` | Concurrent imports, initial saves, result serialization, pivots and exports, default 2; range 1–8; excess requests receive retryable HTTP 503 |
| `CLUSTERING_WEB_CONFIG` | Existing optional label-term configuration JSON |
| `RUST_LOG` | Logging filter, default `info` |
| `ENTRA_TENANT_ID` | Company tenant UUID; required for sign-in |
| `ENTRA_CLIENT_ID` | Application registration/client ID |
| `ENTRA_CLIENT_SECRET` | Client secret injected through deployment secrets |
| `ENTRA_REDIRECT_URL` | Exact registered HTTPS URL ending in `/auth/callback` |
| `ENTRA_ALLOW_GUESTS` | Explicit `true` or `false`; agree eligibility with IT |
| `APP_SESSION_SECONDS` | Explicit session lifetime, 300–86400 seconds; agree with IT |

Register a single-tenant web application using authorization code flow. Configure `openid profile email` and the `acct` optional ID-token claim when guest access is disabled: employee accounts must supply `acct=0`. The ID token must supply a display name and a valid email or email-form preferred username. The application validates signature, issuer, audience, tenant, object ID, nonce, expiry and not-before time. Sign-in uses state and PKCE; application sessions use opaque, secure, HTTP-only cookies and CSRF tokens.

Tenant/client/redirect values were not available during implementation. Company sign-in, guest eligibility, session lifetime and access-removal policy therefore remain staging prerequisites. Disabling a user in Entra prevents subsequent sign-ins; existing application sessions expire according to `APP_SESSION_SECONDS`. Immediate application-session revocation is not tied to Entra events. Do not assume it is.

## User and storage behavior

Users first sign in to appear in the reviewer directory. All authenticated users can discover and edit saved analyses. Initial clustering/import results remain owner-only until explicitly named and centrally saved. Names are trimmed, canonically normalized and compared using locale-independent Unicode lowercase; archived names stay reserved.

Once centrally saved, source data, mappings, clustering settings and membership remain fixed. Review changes, history, receipts and change notifications commit together. Independent fields use separate versions. Workflow state, SME email, action-owner email, ETA and inheritance share one version; cluster reviewer is separate. Unacknowledged browser edits remain in the open page. Closing a browser does not promise recovery of unsent edits.

Review assignments do not lock editing. Presence expires after 60 seconds without a heartbeat. Personal views are saved separately, and only the latest accepted view is used on reopening. Open tabs do not receive another tab's view settings.

The queue executes one child process at a time. Queued jobs survive a restart; a running job interrupted by restart is marked failed and must be explicitly resubmitted. Browser closure does not cancel jobs. Completed results are available through My jobs for the retention interval. A centrally saved result remains available through its analysis regardless of that interval.

Expired unsaved jobs are excluded from access and My jobs immediately according to their stored expiry. Physical cleanup runs at application startup: expired job/session records are removed, and unreferenced artifact/input/staging files older than one day are reclaimed. Schedule normal maintenance restarts if continuous uptime would otherwise leave old files on disk. Command receipts and delivery history currently remain retained; monitor disk consumption.

Temporary review drafts remain in server memory before first central save. They are not promised to survive an application restart. Eviction drops only reloadable immutable result data, retaining those in-memory review drafts.

On session expiry, the page retains pending requests and opens a sign-in renewal dialog. Sign in through its new-tab link; the original page resumes only after the same internal user identity is established. It reuses the pending command ID and expected versions. Signing in as another person does not submit the original person's drafts.

Clustering children receive only OS path/temp variables, worker-thread configuration and the log filter. They do not inherit Entra credentials or server session configuration. Thread count and timeout are application limits; enforce memory and CPU limits through the container runtime.

New `.icas` downloads use schema 3. Schema 1 and 2 imports remain supported. Imports create new central identities, preserve recorded comment attribution as read-only, mark imported reviewers unconfirmed, and initialize only the importing user's view. New comments retain normal ownership. `.icar` review replacement is no longer served.

## Backup, restore and upgrades

Use the application backup command from the hosting scheduler. **Stop the application first.** Live backups have not been adopted for this release. Back up the complete database and its sidecars together with referenced artifacts and inputs; do not copy only `analysis.db`.

```sh
APP_DATA_DIR=/srv/incident-clustering/data incident-clustering-analyzer backup /srv/backups/incident-clustering-20260914
incident-clustering-analyzer restore /srv/backups/incident-clustering-20260914 /srv/incident-clustering/restored-data
```

The destination must be a new directory outside the source. A backup is complete only when `manifest.json` exists and the command succeeds. The manifest records file sizes, SHA-256 hashes, format version and pinned engine version. Restore verifies the complete manifest before creating the new data directory, then verifies copied files again. Restore never overwrites the current data directory.

For a container, stop the server container and run its image with the same data mount plus a separate backup mount, passing `backup /backups/<new-name>` after the image name. The backup container needs the same filesystem permissions. Restart the server only after backup completes. The scheduler, backup retention policy, encryption/storage controls and failure alerting belong to the hosting environment.

Before an upgrade, take a successful stopped backup. Validate the new image against a restored copy, then start it on the production mount with only one instance. On rollback, use the earlier image with a separately restored compatible data directory. Do not assume Turso and SQLite database files or future Turso releases are interchangeable.

`GET /healthz` checks database access. The image health check uses the executable's `healthcheck` command. SIGTERM and Ctrl+C stop the supervisor; interrupted work is reconciled at the next startup. Monitor health, queue failures, publication-retry logs, memory and free disk space. Backup RPO/RTO targets remain for agreement with IT.

Runtime metrics are logged every 60 seconds: queued/running jobs, unpublished changes, estimated cached bytes, active presence views, free disk bytes and available admission permits. `RUST_LOG=info,incident_clustering_analyzer::storage=debug` adds per-command admission/total latency, retries and outcome without incident text. Child start/completion logs include PID and elapsed time; monitor process CPU/RSS through the hosting environment. Backup and restore log successful completion only after verification and durable publication. Publication backlog counts are a diagnostic, not an end-to-end SSE latency measurement.

## Validation commands and release prerequisites

```sh
cargo test --locked
node --test web_tests/*.mjs
cargo clippy --all-targets -- -D warnings -A clippy::too_many_arguments
cargo build --locked
cargo test --lib supervised_child_publishes_reopenable_result -- --ignored --nocapture
npm ci
npx playwright install chromium
cargo test --lib browser_collaboration_acceptance -- --ignored --nocapture
cargo test --release --lib acceptance_tests -- --ignored --nocapture
```

The Clippy exception covers the pre-existing `ProgressReporter::substep` API. The browser test launches a test-only HTTP fixture and uses opaque test sessions; no company credentials are needed. Set `PLAYWRIGHT_BROWSERS_PATH` if browsers were installed outside Playwright's default cache. Do not run every ignored test indiscriminately: the internal crash-test child waits to be killed by its parent test.

Before production rollout, complete actual Entra sign-in/logout/expiry testing, HTTPS/SSE ingress testing, a restore rehearsal on the mounted company filesystem, and mixed-workload timing with real representative XLSX files and browser clients. Confirm the selected container image builds and runs in the hosting environment. Local Linux/Windows results support these checks but do not replace them.

## Email allowlist sign-in (available 2026-09-15)

The email-only fallback is now explicitly approved and implemented. Set `APP_AUTH_MODE=allowlist`; this mode does not need any Entra variables. Users enter only their email at `/auth/login` (also `/login`). The server obtains their display name from the configured file. No password or email verification is requested.

Create an operator-maintained JSON file using this format:

```json
[
  { "email": "alice@company.example", "name": "Alice Reviewer" },
  { "email": "bob@company.example", "name": "Bob Analyst" }
]
```

Use actual work emails and names. `configuration/allowed-users.sample.json` is a template, not an enabled access list. Email comparison trims whitespace and ignores capitalization. Entries require valid emails and nonempty names of at most 200 bytes; duplicate normalized emails, malformed JSON, unknown entry fields and files over 1 MiB are rejected. An empty list permits nobody.

For local Windows testing, save the file as `data/allowed-users.json`, stop the old server, and run from PowerShell:

```powershell
.\start-local.ps1
```

Alternatively pass `-Allowlist C:\path\to\allowed-users.json`. The helper starts the application with allowlist authentication, loopback binding on port 8080 and HTTP-compatible cookies. Open `http://localhost:8080` and sign in with an email from the file. The helper builds the current executable and does not create or overwrite your allowlist.

For the company Linux container, mount the operator's allowlist file and set:

```sh
APP_AUTH_MODE=allowlist
APP_EMAIL_ALLOWLIST=/config/allowed-users.json
APP_SESSION_SECONDS=28800
APP_COOKIE_SECURE=true
```

`APP_SESSION_SECONDS` defaults to eight hours in allowlist mode (range 300–86400). `APP_COOKIE_SECURE` defaults to true; use false only for local HTTP testing. It applies to fallback session creation and deletion; Entra always uses secure cookies. Mount the allowlist read-only into the application, with operator write access outside the container. The runtime reads it during sign-in and authenticated requests, so edits do not require a restart. Replace the complete file atomically when editing. Missing or invalid files deny access; startup validates the configured file before serving requests.

Removing an address blocks subsequent authenticated requests and stops its SSE stream at the next authentication check. Restoring the address can permit its still-unexpired session again. Session expiry and explicit logout invalidate sessions normally. A display-name change is used for subsequent authenticated edits and `/api/me`; the persisted directory record is refreshed on the person's next sign-in. Previously recorded history is not renamed. Only users who have signed in appear in the reviewer directory; adding an allowlist entry alone does not register them.

The normalized email is the fallback identity key, so separate browsers and computers reopen the same user's personal settings. Changing an email creates a new fallback identity; keep emails stable for this rollout. Entra identities remain separate. A later switch to Entra must include deliberate identity mapping if personal settings, comment ownership and reviewer assignments are to follow users; email matching does not silently merge identities.

Shared mutations continue to require CSRF tokens and obtain authors from the established session. The login form uses a same-origin JSON request with a required custom header. Existing session-renewal behavior works with the fallback form, including refusing to submit pending drafts under a different identity.

The earlier statements that fallback was not activated describe the 2026-09-14 delivery and are superseded by this section. Entra-specific release checks may be deferred while operating this explicitly selected fallback; container, backup and collaboration acceptance still apply.
