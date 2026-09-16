# Current Rust/web architecture

Updated 2026-09-14. This describes the implemented multi-user application. The older desktop specification is historical. Release acceptance is recorded in [implementation status](multi-user-implementation-status-20260914.md); deployment instructions are in the [operator guide](multi-user-operations-20260914.md).

## Runtime and ownership

One Linux container runs one Hyper/Tokio server with embedded browser assets. `src/main.rs` supports server, supervised worker, healthcheck, stopped backup and restore modes. Embedded Turso 0.7.2 and immutable artifacts live on a persistent local filesystem. An exclusive instance lock prevents two servers from sharing the directory. No external database, queue or message broker is required.

`auth.rs` implements company Entra authorization-code sign-in with state, PKCE, nonce and signed ID-token validation. Stable tenant/object identity maps to an internal user UUID. Opaque server sessions and CSRF tokens protect APIs; actors come from the authenticated session. The directory contains users who have previously signed in. All authenticated users can edit all centrally saved analyses. Temporary source/job/result access is owner-only. Real Entra configuration remains a staging prerequisite; test fixtures do not provide a production bypass.

`jobs.rs` persists queued work and retained temporary results. `web/jobs_runtime.rs` supervises one child process at a time. The child receives input/output file paths and a restricted environment, emits progress, and never opens the shared database. Queue acceptance is serialized, with monotonic ordering when submissions share a millisecond. Cancel, timeout and nonzero exit produce explicit terminal outcomes. Queued jobs survive restart; interrupted running jobs require explicit resubmission. Completed unsaved results expire after seven days by default. My jobs displays their expiry.

## Immutable results and mutable review

`artifacts.rs` stages, verifies, flushes and publishes checksum-named `.icas` artifacts before catalog records reference them. `AnalysisRun` contains immutable source, mapping, settings and membership. The artifact cache coalesces loads and applies an estimated byte budget; active requests pin temporary results against eviction. Expensive HTTP operations have separate bounded admission. Large parsing, compression and workbook operations use blocking tasks outside database transactions.

Initial central saving freezes a consistent temporary review state, assigns a new permanent analysis UUID and requires a globally unique name. The authenticated user performing that first save is the analysis owner, persisted by the existing `analyses.creator` foreign key. Analysis and catalog metadata expose `owner` with the user ID, display name and email. Later edits, renaming, archiving and restoration preserve the owner. Existing analyses use their recorded creator; importing and centrally saving a separate analysis establishes the importing user as its owner. Names are trimmed, NFC-normalized and compared through locale-independent Unicode lowercase, including archived names. First-save command receipts make uncertain retries safe. Further clustering creates a separate job and analysis.

`storage/schema.sql` defines schema version 1: users, sessions, analyses, review entities, reviewers, comments, workflow history, audit history, personal views, command receipts, outbox/delivery records and jobs. Startup rejects unsupported schema versions. The process configures MVCC, foreign keys and FULL synchronization. Default active write admission is four.

Shared commands use a UUID, target, action, expected record version and optional parent-workflow version. Labels, workflow, reviewer assignment and individual comments have separate versions. Workflow status, SME email, action-owner email, ETA and inheritance form one coherent record. Cluster reviewer is a distinct record; themes inherit its display. Assignment does not prevent other users from editing.

`workflow.rs` retains transition, required-field, recorded-path undo and theme inheritance rules. Shared mutations operate on affected entities, not a replacement of the complete review document. Comment authors alone can edit/delete their new comments. Imported comments are read-only regardless of matching names or emails. Label, workflow and reviewer history preserve recorded actor and before/after values.

Mutation, history, receipt and outbox entry commit atomically. Receipts retain a payload digest and are not currently pruned. Reusing a command ID with different content is rejected. Transport retry reuses the command; reconsidering a conflict creates a new command. Coordination order is maintenance boundary, catalog guard when needed, analysis lifecycle guard, parent-workflow guard, then writer admission. Archive/restore exclude shared edits; parent workflow changes exclude dependent theme commands. Bounded database retries stay inside those guards. Mutation tasks survive HTTP disconnects so the receipt resolves an uncertain outcome.

## Collaboration and personal state

A single background publisher assigns durable delivery sequences to committed outbox rows. Transaction allocation order is not used as a delivery cursor. SSE replays deliveries after a cursor in bounded batches; initial connections establish a current cursor and the browser refreshes its snapshot. Invalid/future cursors reset. Delivery records are currently retained indefinitely. Streams recheck authentication. Shared snapshots include a monotonic per-analysis revision so delayed responses cannot replace newer content or another analysis's state.

Presence is transient, keyed by user and browser-view UUID. Heartbeats update the viewed analysis/cluster without claiming work. Entries expire after 60 seconds. Personal views are independent user/analysis records: selection, filters, sorting and pivot configuration. Each tab serializes/coalesces its view saves; the latest accepted write is restored on the next open. Shared notifications never apply another tab's personal view.

The browser modules are `collaboration.js` (catalog, jobs, presence, central save, views and SSE), `api.js` (CSRF, commands, retry and same-account sign-in renewal), `workflow.js` (forms, conflicts, ownership and history), `results.js` (result/detail/pivot rendering), and `view-state.js` (view capture/sanitization). Dirty forms retain drafts across shared refreshes. Archive notifications disable edits while leaving draft text visible. Sign-in renewal occurs in a new tab and does not retry a prior user's command under a different identity.

## Concrete HTTP operations

| Contract | Purpose |
| --- | --- |
| GET `/auth/login`, GET `/auth/callback`, POST `/auth/logout` | Entra sign-in and protected logout |
| GET `/api/me`, GET `/api/users` | Current session/CSRF and known reviewer directory |
| POST `/api/import`, POST `/api/sources/{id}/worksheet` | Owner source upload and worksheet selection |
| POST `/api/analyze`, GET `/api/jobs` | Enqueue clustering and list the caller's jobs |
| POST `/api/jobs/{id}/cancel` or `/resubmit` | Owner job management |
| GET `/api/jobs/{id}/events` or `/result` | Owner progress/result access |
| POST `/api/jobs/{id}/save-central` | Unique named publication with command ID and initial personal view |
| GET `/api/analyses?search=...&archived=...` | Active or archived catalog |
| GET `/api/analyses/{id}/result` or `/review` | Immutable analysis or current shared review plus caller's view |
| POST `/api/analyses/{id}/commands` | Versioned shared mutations, rename/archive/restore |
| POST `/api/analyses/{id}/view` or `/presence` | Personal view save or transient heartbeat |
| GET `/api/changes` | Authenticated delivery-cursor SSE |
| POST `/api/sessions` | Legacy/new `.icas` import to an owner-only temporary result |
| POST `/api/analyses/{id}/session/save` | Schema-3 portable download with requested view |
| POST `/api/analyses/{id}/incidents/export`, `/cluster-view/export`, `/pivot/export` | Consistent Excel exports |
| GET `/healthz` | Database readiness check |

Temporary jobs retain applicable review, pivot and export routes, protected by ownership. Shared workflows cannot use temporary mutation routes after central saving. `.icar` replacement/download routes are no longer served. Errors distinguish unauthenticated/forbidden requests, stale conflicts, archived state, missing resources and retryable capacity failures.

## Session and operational compatibility

The binary header remains `ICASESS\0`, schema at bytes 8–9, kind at 10, Postcard/Zstandard codec at 11, decoded length at 12–19, and payload SHA-256 at 20–51. New full-session exports use schema 3, including reviewer attribution and audit records; frozen schema 1 and 2 imports remain supported. Generated labels stay immutable; edited labels belong to review data. Import creates a new central identity, keeps recorded comments read-only, and shows imported reviewers as unconfirmed until selected or claimed. Exports exclude authentication, presence and other users' personal views.

Backups require a stopped server and acquire its instance lock. The command copies database sidecars, artifacts and inputs, verifies checksums and publishes a manifest only after durable copying. Restore verifies a manifest before creating a new destination and never overwrites existing data. Startup alone reclaims expired temporary records and old unreferenced files, avoiding live publication/cleanup races. Runtime logs expose queue, cache, presence, admission and disk metrics; the hosting environment supplies scheduling, memory/CPU limits and alerts.

Current review responses and result loads contain full snapshots; history/detail paging is not implemented. Local synthetic measurements are recorded separately. Large-browser readiness, sustained history growth and mixed load must determine whether paging is necessary before release; storage timing alone does not establish the 30-second browser target.

## 2026-09-15 authentication update

The user has activated email-only fallback authentication while Entra setup proceeds. `APP_AUTH_MODE=allowlist` selects the external email/display-name file through `APP_EMAIL_ALLOWLIST`. `/auth/login` and `/login` show the email form; POST `/auth/login` establishes an opaque session using the mapped name. The allowlist is validated at startup and reread for sign-in and authenticated requests. Email normalization provides identity continuity across devices. Session removal/expiry and CSRF protections remain in effect. The Entra implementation remains available through `APP_AUTH_MODE=entra`; no automatic identity merge occurs between providers. See the operator guide for local startup and deployment configuration.

## Dashboard and reviewer filtering (2026-09-16)

The default authenticated screen is Dashboard. `dashboard.js` owns its independent per-user searches, archive/status filters and expanded cluster groups. `reviewers.js` resolves confirmed and imported reviewer identities and supplies the shared Results row predicate and informational tree labels. `jobs-view.js` supplies job metadata text shared with the My jobs modal. All controls use the existing application CSS variables and system light/dark media query.

The server migrates schema 1 to schema 2 transactionally. `storage/dashboard.rs` adds `dashboard_preferences`, immutable `dashboard_entities` summaries, `dashboard_ready` backfill markers, and an indexed confirmed `reviewers.user_id`. Confirmed reviewer identity changes commit with the existing review command. Initial central saves insert summaries with the analysis; mutable labels and effective workflows are read from the authoritative entities, not copied into summaries. Before binding the HTTP listener, startup backfills missing summaries one artifact at a time and records completion per analysis. Artifact failures identify the analysis and stop startup instead of silently excluding work.

Authenticated `GET /api/dashboard/data?offset=N` returns at most 500 catalog summaries and 500 assigned entity summaries, plus `nextOffset`. The browser assembles pages before applying independent section searches/filters and preserving parent context; no incident data or artifacts are loaded by these queries. This consolidates the plan's proposed catalog/assignment routes into one paginated metadata route. Browser rendering starts with 100 analyses or assigned cluster groups, exposes Show more, and renders theme children on expansion in batches of 100. Dashboard settings use authenticated `GET/POST /api/dashboard/preferences`, with CSRF protection on writes, validation and a 64 KiB serialized limit.

Shared-analysis SSE events invalidate dashboard summaries. Reconnection, returning to Dashboard and page visibility restoration also refresh them. A single three-second visible-dashboard poll refreshes owned jobs; unchanged catalog data is not fetched on every job poll. Requests coalesce; errors retain last successful lists. Personal settings are debounced/serialized with retries while open, and are not received through shared SSE. Open tabs retain independent controls.

Reviewer filters extend personal view version 2. The row set intersects existing table, tree, workflow and drilldown filters; incident, cluster-view and pivot data exports retain those constraints. Session version 4 contains the full analysis and filter settings. Explicit frozen legacy view DTOs retain readers for session versions 1–3. Imported assignments remain unconfirmed and do not match Assigned to me by name/email.

Analysis loading uses request generations and navigation epochs to ignore obsolete responses. Opening an assigned entity applies its selection and necessary filter clearing after personal-view restoration, allowing the existing autosave to persist only the resulting view. Background job completion does not open Results after the user has left its progress pane.
