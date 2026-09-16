# Dashboard and reviewer filtering implementation plan

Date: 2026-09-16. Status: Ready for implementation review; no implementation or deployment is recorded by this document.

## 1. Scope and implementation approach

Implement [Dashboard and reviewer filtering requirements](dashboard-reviewer-requirements-20260916.md), including RP-1, RP-2, DP-1 through DP-5, NAV-1, LAYOUT-1, PREF-1, LIVE-1, and AC-01 through AC-20.

Build on the existing Rust/Hyper application, embedded Turso storage, vanilla JavaScript UI, central personal views, and SSE collaboration. Retain the single-instance container, local persistent storage, current identity modes, job supervisor, and backup mechanism. No additional service or frontend framework is required. Entra configuration is not a prerequisite; use the working fallback sign-in and isolated test identities for validation.

Keep analysis ownership, job ownership, and reviewer assignment distinct. Preserve archive read-only behavior, equal shared-analysis access, comment ownership rules, and the separation of reviewer assignment from SME email, implementation owner, and ETA.

This is a dependency-ordered delivery plan, not a calendar estimate. Each milestone has a demonstrable exit condition; intermediate milestones do not constitute the completed enhancement.

## 2. Current code findings and implications

| Existing code | Finding | Implementation implication |
| --- | --- | --- |
| `src/storage/mod.rs`, `schema.sql` | `analyses.creator` stores the initial owner's identity; `AnalysisInfo` already exposes owner and creation date. | Reuse existing ownership; do not infer it from assignments or add a competing owner field. |
| `Store::list` | Current catalog query selects either active-only or archived-only and looks up each analysis separately. | Add an explicit include-archived query for Dashboard; use joined summaries and avoid per-analysis lookups. Preserve the old modal contract unless deliberately updated with its tests. |
| `entities` and `reviewers` tables | Mutable labels/workflow and reviewer JSON exist, but there is no dashboard-oriented incident-count/parent summary or indexed reviewer identity. | Add a small queryable summary of immutable entity metadata and an indexed assignment identity; do not load every analysis artifact to list assigned work. |
| Storage initialization | Existing startup accepts schema version 1. | Implement and test an actual versioned migration, including existing installations. Editing only the fresh schema is insufficient. |
| `collaboration.js` | Catalog, jobs, analysis opening, shared SSE, and personal-view saving are mostly inside one initializer. | Extract reusable job rendering/actions and analysis navigation so Dashboard and existing modals share behavior. |
| `results.js` | Tree and detail filtering have separate entry points; exports and pivot use their row selections. | Add a shared reviewer predicate/row-set helper and wire both paths, preserving existing selection semantics. |
| `view-state.js`, `session.rs` | Personal views are captured in JavaScript and sanitized in Rust. Binary session DTOs embed view state. | Extend browser and server state together; explicitly preserve old binary layouts before adding fields. |
| `shared.rs` and `analysis.js` | Shared changes have a global SSE stream; job progress has separate streams, and completion currently opens Results. | Refresh Dashboard from shared events, poll the user's job summaries, and prevent background completion from stealing navigation. |
| `web.rs` | Browser assets are explicitly embedded and routed. | Register every new JavaScript asset in both embedding and routing. |

## 3. Proposed implementation contracts

These are implementation choices for this enhancement, not additional stakeholder requirements. Preserve the compatibility interpretations in section 10 of the requirements.

### 3.1 Reviewer filtering

Represent the personal filter as a tagged mode: `all`, or `selected` with deduplicated user IDs and category flags for `me`, `unassigned`, and `unconfirmed`. Selecting All resets restrictions. An explicitly empty selected mode matches nothing; it must not accidentally become All. Missing state in older views defaults to All.

Resolve `me` against the currently authenticated identity. Match confirmed user IDs, never name/email text. Keep unconfirmed imported attribution separate. Build a set of permitted source-row indices from matching parent clusters; themes use their parent's assignment. Intersect this set with other filter types at both tree and detail entry points. Retain existing selection-specific detail/pivot behavior and deduplicate incident membership.

Use the same assignment resolver for tree text, reviewer selection, and filter matching. Invalidate cached row sets when the run, assignment snapshot, user identity, or filter changes. A temporarily empty filtered view must not silently clear the user's saved filter.

### 3.2 Dashboard read model

Add compact immutable entity summaries keyed by analysis and entity, containing parent cluster key, cluster/theme identifiers, generated label, and incident count. Read mutable label overrides and effective workflow from existing authoritative records rather than maintaining a second mutable workflow copy. Reuse existing workflow inheritance semantics, including parent status fallback for themes.

Maintain a nullable indexed confirmed-reviewer user ID alongside the existing reviewer record. Populate it only from confirmed assignments. Update it in the same transaction as assignment changes, command receipt, history, and outbox publication. Keep the existing reviewer JSON for attribution and imports; test that the two representations cannot diverge through create, import, assign, claim, release, or retry paths.

Create summaries during initial central saving/import publication from the already loaded immutable run. For existing analyses, backfill summaries once from artifacts before Dashboard is enabled. Decode artifacts one at a time, outside write transactions, and commit bounded batches. Record per-analysis completion so an interrupted backfill can resume without duplicates. Do not silently omit analyses with unreadable artifacts: report the analysis and expose a clear readiness/error state.

Dashboard requests must not deserialize source rows, full analysis files, comments, or audit history. Join analysis ownership and mutable entity records in bounded queries. Candidate indexes include analysis owner/creation ordering, confirmed reviewer identity, and summary parent keys; verify them against the actual Turso runtime and workload.

### 3.3 Routes and settings

Introduce focused authenticated endpoints, with final names allowed to follow repository conventions:

| Proposed route | Purpose |
| --- | --- |
| `GET /api/dashboard/analyses` | Analysis summaries; `scope=mine|all`, name search, explicit `includeArchived`, bounded pagination. |
| `GET /api/dashboard/assigned-work` | Current user's assigned clusters/themes grouped by analysis; status and include-archived filters, bounded group pagination. |
| `GET /api/dashboard/preferences` | Current user's dashboard settings or defaults. |
| `POST /api/dashboard/preferences` | Persist validated dashboard settings using existing authentication and CSRF protection. |
| Existing `GET /api/jobs` | Reuse current user's job list, metadata, and retention semantics. |

Use a dedicated per-user settings record, separate from analysis views. Store a settings schema version, each section's independent search/filter state, and expansion keys based on stable analysis/entity IDs. Bound request size, search length, and expansion collections; sanitize stale or malformed data. Do not accept a client-supplied owner as the authority for current-user endpoints.

Paginate analyses in deterministic creation-date/ID order. Page assigned work at cluster-group boundaries with analysis headings repeated when necessary; preserve matching parent context. Load theme children in bounded batches if a single cluster is large. Never silently truncate at a server limit. Name search will use the existing trimmed, case-insensitive normalized substring convention. Use numeric cluster and theme ordering with stable IDs as tie-breakers.

Personal dashboard settings use debounced, serialized saves and retry while the page remains open. Display saving/failure state without claiming success before acknowledgment. Open tabs remain independent; the most recently accepted complete settings record restores on next reopen. Hydrate settings before starting autosave so defaults cannot overwrite stored preferences. Shared events never overwrite local personal controls.

### 3.4 Updates and navigation

Reuse the existing shared SSE connection for catalog, assignment, label, and workflow invalidation. Coalesce refreshes per affected dashboard section, ignore obsolete responses using request generations, and refresh snapshots on reconnect and return to Dashboard. A refresh failure preserves the last successful data with an error indicator.

For jobs, start with one owner-scoped polling loop while Dashboard or My jobs is visible; reuse the existing progress SSE for a job explicitly opened by the user. A provisional three-second polling interval is an implementation default to measure, not a newly agreed dashboard SLA. Pause hidden-pane polling, refresh on return, and back off after failure. Do not open a progress stream for every listed job or broadcast private job metadata on the shared-analysis stream.

Introduce one navigation coordinator for permanent analysis IDs, temporary job IDs, and optional cluster/theme targets. Resolve the latest job state at click time. Load shared state and the personal view before applying an assigned-item navigation override. Suspend intermediate view autosaves, expose the target, then persist the final view through the existing save path.

For target navigation, remove filter restrictions that prevent the target from being visible, evaluating the combined result as well as individual restrictions. If necessary, clear Results filters and drilldown as a safe visibility fallback while retaining sort and pivot layout. Do not save a half-restored view if loading fails. Guard all asynchronous opens with a navigation generation so late responses cannot replace a newer selection.

## 4. Delivery milestones

| Milestone | Outcome | Depends on |
| --- | --- | --- |
| M0 | Baseline, fixtures, and finalized contracts | None |
| M1 | Migrated storage, summaries, dashboard APIs, and preferences | M0 |
| M2 | Reviewer display/filter with durable personal state and compatible sessions | M0; M1 for integrated fixtures |
| M3 | Dashboard landing pane and reusable lists | M1 |
| M4 | Complete item navigation and automatic updates | M2, M3 |
| M5 | Integrated acceptance, workload validation, and rollout documentation | M1–M4 |

### M0 — Baseline and compatibility fixtures

1. Record current working-tree changes and baseline test results without overwriting unrelated work. Ownership changes already present are an input to this plan.
2. Capture representative shared analyses: two owners, several reviewers, unassigned and unconfirmed imported assignments, parent/theme workflow differences, active and archived analyses, and every retained job state.
3. Freeze independently generated old `.icas` fixtures for all currently supported formats before changing `ViewState`. Include files with labels, reviewers, personal views, and imported attribution.
4. Finalize DTO limits, grouped pagination, stable sort rules, empty reviewer-filter behavior, and settings versioning described in section 3. Document exact fields alongside the implementation.

**Exit:** Baseline failures are distinguished from new failures; fixtures exist; contracts are sufficiently precise for backend and browser work.

### M1 — Durable summaries and dashboard APIs

1. Add a tested schema upgrade from version 1 for dashboard preferences, entity summaries/backfill readiness, and confirmed reviewer lookup. Retain existing records and immutable ownership.
2. Make fresh initialization and upgraded databases converge to the same schema. Preserve startup rejection of unknown future versions. Validate transactional DDL behavior with the actual embedded Turso dependency.
3. Integrate summary/index writes into central publication and reviewer commands under existing lock and transaction ordering. Keep artifact decoding outside transactions and database admission permits.
4. Implement resumable existing-analysis backfill with progress/error reporting. Use the single-instance maintenance/startup boundary; measure the interruption for existing catalogs and document it before rollout.
5. Add joined owner-aware catalog queries with include-archived semantics, name search, deterministic pagination, and assigned-work queries that retain parent context for matching themes.
6. Add preference read/write operations and protected HTTP routes. Register routes with the existing authorization checks; validate query/body data and avoid internal artifact paths in dashboard DTOs.
7. Verify backup and restore include the added tables, preferences, and backfill state through the existing mechanism.

**Exit:** API tests demonstrate correct ownership, assignments, status inheritance, archive inclusion, pagination, and per-user settings across restart. Dashboard queries operate without decoding artifacts. Interrupted migration/backfill can resume safely.

### M2 — Reviewer filtering and display end to end

1. Add a focused reviewer helper module for identity resolution, display names, category matching, and permitted-row sets. Use it for both permanent analyses and imported/temporary results where attribution is available.
2. Add an accessible multi-select control with All, Assigned to me, Unassigned, and Unconfirmed imported reviewer. Populate confirmed reviewers from the signed-in directory, retaining resolvable existing assignments even if a user is no longer an active choice for reassignment.
3. Extend `state.js`, `view-state.js`, Rust personal-view DTOs, and sanitization. Missing legacy filters become All; preserve valid empty-selected state. Prevent refreshes from applying another personal view over an open tab.
4. Wire reviewer filtering into `treeVisibleRows` and detail row selection. Check pivot requests, incident export, cluster-view export, and pivot export; they must receive the intended filtered population, including empty populations rather than accidentally exporting everything.
5. Add reviewer text beside workflow-status buttons for clusters and themes, using existing theme variables and typography. Preserve status-button behavior and workflow-modal assignment controls.
6. Preserve old binary view/session wire types explicitly. Introduce a new portable session version if needed for the extended view; update header dispatch, codecs, limits, and conversions together. Serde defaults alone are not a binary Postcard compatibility strategy. Continue importing existing formats and export the full analysis plus personal filter state.
7. Invalidate filters and update tree labels on shared assignment changes without losing review-editor drafts. Handle filters selecting users with no remaining assignments with an empty state.

**Exit:** RP-1 and RP-2 work through UI, saved views, exports, and old/new session round trips. Tests prove identity-based matching, inherited assignment, filter intersections, and no duplicate incident counts.

### M3 — Dashboard landing pane and lists

1. Extract reusable job descriptions, state-specific actions, and catalog-open behavior from `collaboration.js`. Keep the existing Analyses and My jobs controls working.
2. Add `dashboard.js` and focused helpers as needed; register embedded asset routes in `web.rs`. Add the Dashboard navigation entry and screen to `index.html`, with Dashboard initially active. Initialize after authentication and settings hydration without flashing Results or overwriting personal views.
3. Render the four sections in the specified order. Reuse app typography, palette, form styling, and buttons; implement the two-by-two grid and small-screen stack with independently scrollable lists.
4. Implement separate search/archive state for My analyses and All analyses, plus Assigned work's status/archive filters. Provide clear loading, empty, no-match, stale-data, and failure states per section.
5. Render assigned analysis groups and collapsible clusters with themes, context-only headings, IDs, labels, statuses, and incident counts. Keep count labels clear: parent and child counts are not additive.
6. Restore and autosave preferences across devices. Keep scroll restoration during refresh in memory, independent from durable expansion/filter settings.
7. Use stable keyed rows or reconciled sections so refreshes preserve expansion, focus, and scroll anchors. Use safely escaped text/DOM text nodes for names and labels, and keyboard-accessible activation without nested interactive controls.

**Exit:** All four lists show correct data and independent settings; Dashboard is the landing pane on sign-in/reload; existing modals still work; light/dark and narrow-screen checks pass.

### M4 — Navigation, job behavior, and live updates

1. Route dashboard and modal open actions through the shared navigation coordinator. Completed linked jobs open their shared analysis; temporary results use the existing job result flow. Analysis entries restore personal views.
2. Implement assigned cluster/theme targeting after view restoration, including parent expansion, filter visibility resolution, and one final personal-view save. Preserve archive read-only behavior and never claim on opening.
3. Let queued/running jobs open progress. Failed/cancelled rows open details and resubmit actions; missing retained input produces an explanatory unavailable action. Handle expiry and click-time state changes explicitly.
4. Adjust `listenForProgress` completion behavior so only a still-active, explicitly opened progress view may transition to its result. Background job completion updates Dashboard without moving users away from their chosen pane.
5. Connect shared SSE invalidations and job polling as described above. On reconnect, resnapshot rather than relying solely on future events. Prevent duplicate subscriptions/timers after repeated navigation or sign-in renewal.
6. Handle catalog changes, assignment removals/additions, label/status changes, and archival without resetting list controls. Prune empty visible groups; preserve unrelated sections and editor drafts.
7. Exercise same-user multiple tabs, two different users, interrupted requests, stale API responses, and sign-out/session renewal. Discard pending responses/settings writes belonging to the previous authenticated identity.

**Exit:** DP-4 and LIVE-1 pass with two simultaneous users and background jobs. No stale response overrides a newer navigation, failed load overwrites a saved view, or background completion steals focus/navigation.

### M5 — Integrated validation and release preparation

1. Complete the acceptance matrix below and run the existing workflow, job, authentication, session, and collaboration regression suites.
2. Exercise a representative 1,000-analysis catalog, approximately 50 active users, 10 simultaneous reviewers on one analysis, and both typical 20,000-row and large 150,000-row/30-column analyses. Choose and record a representative high cluster/theme count; source-row count alone does not bound dashboard list size.
3. Measure dashboard query latency, response size, browser render/refresh time, memory, polling/SSE load, and backfill duration. Confirm dashboard listing does not load large artifacts or materially disrupt existing save/load and shared-edit timing targets. Record measured results rather than claiming an unagreed dashboard SLA.
4. Rehearse upgrade on a copy of existing data, restart during backfill, and backup/restore of the upgraded schema. Document any required maintenance window and artifact problems before rollout.
5. Update `documentation/web-architecture.md` and operations guidance with routes, schema version, backfill procedure, refresh lifecycle, personal-state behavior, and session-format compatibility. Record final validation evidence in a delivery-status document.

**Exit:** All applicable acceptance criteria pass; old files remain importable; migration/restore evidence is recorded; known limitations are explicit. Any deployment is a separate operational step.

## 5. Test and acceptance traceability

| Coverage | Required evidence | Requirement scenarios |
| --- | --- | --- |
| Storage and HTTP | Correct immutable owner; current-user jobs/preferences; confirmed versus imported reviewer matching; status inheritance; joined summaries; archive inclusion and pagination | AC-03, AC-05–08, AC-17–18 |
| Filter logic | Multi-select union, intersection with other filters, All versus empty selection, inherited themes, deduplication, cache invalidation, empty export handling | AC-12–14, AC-19 |
| Session compatibility | Frozen old fixtures decode; new portable export/import preserves full analysis and reviewer-filter state; old JSON views default correctly | AC-19 and requirements section 10 |
| Browser navigation | Landing behavior; modal parity; completed/progress/failed jobs; archived and expired targets; saved-view restoration; hidden-target opening; late-response races | AC-01, AC-04, AC-08–11 |
| Browser collaboration | Assignment/label/status/catalog changes; preserved filters/expansion/scroll; reconnect refresh; cross-device settings; independent tabs; background completion | AC-16–20 |
| Visual and accessible UI | Wide/narrow layouts, light/dark screenshots, long labels, keyboard access, informational reviewer placement, clear empty/error states | AC-02, AC-06–07, AC-15, AC-20 |
| Operational | Version-1 upgrade, backfill restart, fresh/upgraded schema equivalence, backup/restore, representative catalog/concurrency measurements | LIVE-1 and section 8 operating context |

Use existing suites as the starting point: `src/storage/tests.rs`, `src/web/shared_tests.rs`, `tests/sessions.rs`, `web_tests/results_filters.mjs`, `web_tests/workflow.mjs`, and `web_tests/browser-acceptance.js`. Add focused dashboard tests for behavior rather than duplicating implementation details.

Run formatting checks, relevant Rust tests with the locked dependency set, JavaScript tests, and the existing browser collaboration harness. Run ignored browser/workload tests explicitly with their documented prerequisites; record skipped environment-dependent checks honestly. Baseline failures must remain distinguishable from regressions.

## 6. Primary risks and release gates

| Risk | Required mitigation / gate |
| --- | --- |
| Extending `ViewState` breaks old binary sessions or persisted artifacts | Freeze old wire layouts and independently generated fixtures; require old-format decode and new-format round-trip tests before M2 exits. |
| Summary/index data becomes stale after edits/imports | Store immutable metadata once, read mutable authoritative records, and atomically maintain confirmed reviewer identity; test every mutation path. |
| Migration/backfill stalls startup or omits existing work | Resumable bounded backfill, explicit readiness/errors, measured upgrade rehearsal, and verified backup before upgrade. |
| Reviewer filter differs across tree/table/pivot/export | Central row-set helper, coverage of every export route, and explicit empty-set tests. |
| Assigned-item navigation overwrites saved views prematurely | Hydrate first, suppress intermediate autosave, apply target override, then save; test interrupted and competing opens. |
| Frequent refreshes degrade server responsiveness or reset UI | Joined summary queries, pagination, coalescing, one visible jobs poller, keyed updates, and mixed-workload validation. |
| Cross-user state or jobs leak through reuse of shared infrastructure | Resolve identity server-side; keep job refresh owner-scoped and preferences separate; test authentication renewal and user changes. |

Before deployment, take an application-consistent backup. If rollback to the previous binary is required after a schema/session upgrade, restore a compatible pre-upgrade backup rather than assume the old binary understands new records. Document the resulting loss of changes made after that backup and retain a separate copy of upgraded data for recovery. Do not introduce an untested destructive downgrade migration.

## 7. Definition of done

The enhancement is complete when all required behavior and AC-01–AC-20 are verified; Dashboard works with the existing fallback identity setup; reviewer filters persist and apply consistently; navigation preserves the intended target and view; existing analyses and supported session files remain usable; background updates preserve working context; and upgrade/restore and workload evidence is documented. Owner transfer, independent theme assignment, new roles, `.icar` replacement, and unrelated workflow changes remain outside scope.
