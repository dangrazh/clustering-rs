# Cluster trends implementation plan

Date: 2026-09-17. Status: Proposed implementation design and delivery plan. No application implementation, matching validation, or deployment is recorded by this document.

## 1. Scope and approach

Implement [Cluster trends requirements](cluster-trends-requirements-20260917.md), including acceptance scenarios AC-01 through AC-22. Build on the existing Rust/Hyper server, embedded Turso database, local immutable analysis artifacts, authenticated commands, SSE, and vanilla JavaScript panels. Retain the single-instance Linux container and existing backup mechanism. No new external service, identity provider, or frontend framework is required.

Deliver named series, chronological periods, persistent tracked problems, automatic text-based matching with review and correction, attributed zero confirmations, and a chart/table with navigation to source clusters. Trend attribution must remain separate from analysis labels, reviewer assignments, workflow, and comments. Themes do not receive independent trend identities.

Use dependency-ordered milestones rather than a calendar estimate. A manual-linking milestone provides a useful verification slice but does not satisfy the complete requirement for automatic matching. Automatic matching is subject to a historical-data quality gate; algorithm development and validation are implementation work, not assumed completed by this plan.

## 2. Existing code and integration points

| Area | Current behavior | Planned integration |
| --- | --- | --- |
| `src/model.rs`, `src/clustering.rs` | Clusters contain incident row memberships; IDs are assigned within each run. | Identify occurrences by analysis identity plus cluster ID. Never use cluster number or label as a cross-analysis identity. |
| `src/text.rs` | `feature_terms` extracts text features; `extract_features` assigns local term IDs and corpus-specific weights. | Reuse tested text normalization where suitable, but build a separately versioned cross-analysis feature representation. Local numeric term IDs are not comparable across runs. |
| `src/session.rs` | Full analysis artifacts and legacy session readers exist; artifact fingerprints identify immutable content. | Use fingerprints for cache integrity, not fuzzy matching. Keep series membership outside `.icas`; importing a session does not import central series identity. |
| `src/artifacts.rs` | Verified immutable artifacts, bounded cache, and coalesced loads. | Extract profiles in background work, one large artifact at a time; never load every series artifact in a chart request. |
| `src/storage/mod.rs`, `dashboard.rs` | Versioned commands, receipts, writer admission, scoped locks, schema migration from 1 to 2, and entity summaries. | Add a version-3 migration and dedicated trends storage module; reuse command and consistency patterns. Use cluster summaries for listings and immutable membership for authoritative counts. |
| `src/jobs.rs`, `src/web/jobs_runtime.rs` | Durable clustering queue and child-process execution. | Add distinct trend tasks with coordinated resource admission; do not disguise matching as a clustering job or change My jobs semantics. |
| Storage outbox and `src/web/shared.rs` | Durable global event sequence currently assumes an analysis ID. | Introduce typed resource events while preserving existing analysis events and replay cursors. |
| `collaboration.js`, `dashboard.js`, `results.js` | Shared analysis opening, assigned-work navigation, personal filters, navigation race protection. | Extract reusable targeted Results navigation for trend contributors. Preserve Dashboard as landing pane. |
| `src/web.rs`, browser assets | Explicitly embedded assets and existing theme/control styles. | Register new series assets; reuse current typography, colors, sizing, and system light/dark behavior. |
| `src/backup.rs` | Application backup/restore of durable state. | Include and verify all new durable data and restart behavior; do not introduce untracked model files. |

Proposed new modules: `src/trends/{mod,model,profile,matching,recalculation}.rs`, `src/storage/trends.rs`, `src/web/trends.rs`, and `src/web_assets/{series,series-chart}.js`. Split modules according to responsibility during implementation; filenames are guidance rather than an API commitment.

## 3. Proposed product and consistency defaults

These resolve design choices left open in requirements section 10. They are proposals, not additional stakeholder confirmations.

- Treat period dates as inclusive calendar dates without time-zone conversion. Reject `end < start` and any overlap; adjacent periods start the day after the previous end. Dates describe an analysis and never filter its records.
- Follow analysis naming conventions: trim names, apply the same case normalization, and reserve archived names. Hide archived series by default; make them read-only until restored. Archiving an analysis does not prevent its use in an editable series.
- Keep Dashboard as the landing page. Add a Series navigation control and a dedicated series pane with searchable catalog, period management, matching review, trend chart/table, and history.
- Start with transient personal series display controls. Cross-device persistence of series search, chart selection, and zoom is not required for this release; do not change existing Dashboard or Results preferences.
- Use one decimal place for displayed percentages, retaining full-precision calculations. Display an unavailable percentage if the processed-record denominator is zero.
- Rejecting a candidate creates a protected exclusion for that exact source occurrence and tracked problem. Keep it until explicitly reopened, including across matcher upgrades. It does not reject that problem for future occurrences. Rejection alone does not force the cluster to become a new problem.
- Offer explicit actions to reopen a manual association for matching and to revoke a zero confirmation, with audit history and downstream preview where applicable. Ordinary recalculation never reopens either automatically.
- Do not add standalone period deletion/date editing, tracked-problem deletion/merging, or trend export in this scope. Correct period source data through the explicit replacement workflow; additional lifecycle operations require separate requirements.

## 4. Persistent domain and schema

### 4.1 Records

Add tables with foreign keys and indexes appropriate to the following logical records. Store searchable keys and states in columns; reserve JSON for versioned evidence and command details.

| Record | Required information and constraints |
| --- | --- |
| Series | Stable ID, name, normalized unique name, archive state, revision, creator/time. Creator is attribution, not a new permission role. |
| Period | Stable ID, series ID, inclusive dates, current analysis ID, source fingerprint, processed-record total. Unique active membership for an analysis across all series, including archived series. |
| Tracked problem | Stable ID, series ID, independent name, creation provenance. Names need not be unique; source label changes do not rename it. |
| Occurrence | Period/source analysis/cluster identity, authoritative count, resolution state, optional problem ID, decision provenance and revision. At most one effective problem association per source cluster. |
| Zero confirmation | Unique problem/period cell, actor/time and revision. Must not coexist with any linked occurrence in that cell. |
| Candidate/exclusion | Source occurrence, candidate problem, evidence, score/rank, matcher version, rejection actor/time where applicable. No attributed count until linked. |
| Matching model | Immutable version, feature/weighting configuration, calibration version, reference-corpus fingerprints, checksum, and creation information. |
| Cluster profile | Source fingerprint, cluster ID, model version, vector and compact representative/evidence data. Unique cache key on these inputs. |
| Trend task | Kind, series, requested revision, actor, command ID, state, progress, error, checkpoint, model version. |
| Change preview | Immutable proposed operations, base revisions, source fingerprints, model version, operation digest, expiry, initiator. |
| Trend audit | Actor/time, operation, series/period/source occurrence, before/after association or zero state, command and preview IDs. Retain replaced-source references as historical evidence. |

An occurrence can be automatically linked, manually linked/confirmed, or pending. Pending reasons include ambiguous candidates, possible split/merge, insufficient text, and replacement review. Record creation of a new tracked problem separately from its later manual confirmation.

Multiple source clusters may link to the same problem in a period only through manual resolution. Compute the union of their source row indices for the cell count; do not sum blindly or deduplicate by incident-number strings. The percentage denominator is the full `processed_incidents` count for that analysis, including records outside linked clusters and regardless of personal filters.

Keep compact aggregate counts in the committed series snapshot. Validate/cache membership counts during artifact processing so chart requests do not decode source artifacts. Prevent membership references to nonexistent clusters. Keep historical evidence distinct from current membership constraints so replacement releases the old analysis's active membership without destroying history.

### 4.2 Migration and transactions

Implement the actual 2-to-3 migration and fresh-install 1-to-2-to-3 sequence. Update the existing version guard and make earlier migrations skip already newer schemas; the current dashboard migration must not rerun against version 3. Test migration interruption and retry. Do not eagerly profile all existing analyses on startup.

Enforce uniqueness in the database. Enforce interval overlap and link/zero exclusion inside a short transaction with a series lock and expected revision. Global analysis-membership uniqueness must also hold when different series are modified concurrently. Define a consistent lock order and reuse existing maintenance and writer-admission rules.

Every successful mutation commits state, audit, command receipt, revision increment, and outbox event together. Repeating a command with identical payload returns its previous result; reusing its ID with another payload fails. Compute text features and previews outside write transactions. Revalidate all prerequisites before publication.

Use versioned series snapshots or staged change sets so multi-period recalculation is published atomically. The UI continues to show the last committed snapshot while a task runs; a failed task cannot expose partially rewritten trends.

## 5. Comparable text profiles and matching

### 5.1 Initial candidate representation

Implement a local weighted lexical baseline first, behind a versioned matching interface. Use immutable `IncidentRecord.analysis_text`, not editable cluster labels, workflow fields, reviewer names, or incident-ID overlap. Structured service/application differences are not hard matching gates.

Use stable feature strings with explicit word/phrase/character namespaces, or a persisted equivalent vocabulary. Do not compare local `TermId` values. Start from existing normalization and feature extraction, adding regression coverage before any shared helper changes. Keep identifier/number masking conservative and configurable: useful error codes must not be erased indiscriminately.

Fit document-frequency weights once on the earliest source period available when a series model is initialized; freeze that reference and its provenance. Use bounded weights for previously unseen terms rather than discarding them. Historical additions do not silently refit this model. A requested rebuild fits an explicitly selected reference, profiles every affected source consistently, and follows the same preview/confirmation process as recalculation. Record reference dates to make retrospective evaluations distinguishable from chronological holdout tests.

Normalize each incident vector, aggregate a normalized cluster profile, and retain a deterministic, bounded set of diverse representative vectors. Version all feature caps, boilerplate suppression, weights, and sampling rules. Missing or unusable text produces a review state, not a confident new identity or false zero. Exact text hashes may support cache/duplicate detection but never establish recurring-problem identity by themselves.

This representation is a candidate to validate, not a claim that lexical similarity captures all paraphrases. If it fails the quality gate, compare a locally hosted semantic representation through the same interface and document its resource, licensing, model-distribution, and operational implications before adopting it. Do not silently introduce a remote service.

### 5.2 Candidate selection and decisions

Process periods chronologically. Search all eligible earlier occurrences in the same series, including older occurrences before unmatched periods. Aggregate candidates by tracked-problem ID before calculating the best-versus-runner-up margin; several historical examples of one problem are not competing identities.

Start with cosine similarity between comparable profiles, supported by representative-text coverage. Preserve the founding reference for each problem and manually confirmed exemplars. Require consistency with a trusted reference as well as the strongest historical match; a chain of increasingly different automatic matches must not redefine the problem silently. If a founding source is replaced, invalidate that reference and route re-establishment through replacement review.

Use a versioned candidate retrieval index over persisted profiles. For small fixtures, compare retrieval results with exhaustive scoring; candidate pruning must not hide the runner-up and manufacture confidence. No full artifact scans occur on interactive reads.

Calibrate three distinct decisions: minimum plausible similarity, minimum automatic-link strength, and minimum lead over another problem. A single candidate still needs the absolute-strength and reference-consistency checks. Treat scores as similarity evidence, never as a probability of correctness.

Evaluate the complete incoming period before committing automatic decisions:

1. Its first chronological period creates one initial tracked problem per cluster; do not merge similar first-period clusters automatically.
2. Later strong, unambiguous candidates may link automatically, subject to manual exclusions, zero confirmations, and split/merge checks.
3. Competing problem candidates, materially mixed representative evidence, or several incoming clusters plausibly claiming one problem enter review. Do not greedily link one contender and silently create identities for the others.
4. A cluster with usable text and no plausible historical candidate creates a new problem, initially named from its current source label.
5. Unresolved occurrences contribute to no problem total. Keep their existence visible in period completeness indicators.

Possible splits and merges are review flags, not assertions of certainty. Users may link several complete clusters to one problem, choose a single problem for a merged cluster, create a distinct problem, or leave it pending. Never distribute one cluster's count across several problems.

### 5.3 Quality gate

Build a labeled historical validation set with domain reviewers. Include recurrence with new IDs, paraphrases, common templates, unrelated problems sharing vocabulary, changing volumes, small/noisy clusters, gaps, multiple languages where present, and split/merge cases. Include negative examples with strong lexical overlap and different underlying problems.

Separate calibration examples from held-out chronological series. Measure automatic-link precision and coverage, new-problem errors, ambiguity workload, split/merge flags, recurrence retrieval, and drift over multiple periods. Record thresholds, dataset provenance, model version, and resource costs in a validation report. Obtain agreement on numerical release gates before claiming the matcher validated; the requirements specify no accuracy target. Favor pending review over unsupported links, but measure whether coverage still makes the feature useful.

## 6. Durable processing and recalculation

Use one bounded trend worker initially, with durable tasks separate from clustering jobs. Coordinate heavy profile generation with clustering resource admission: start conservatively by allowing only one heavy clustering/profile task at a time, while lightweight database reads and review edits remain available. Release admission between artifacts, expose queued progress, and prevent either queue from starving the other. Revisit this limit using measurements on the available 8-core/64-GiB host.

Task stages: validate sources, load/verify artifact, generate or reuse profiles, retrieve/score candidates, construct proposed changes, and publish a snapshot or a reviewable preview. Checkpoint completed immutable profiles and work units. After restart, resume/requeue interrupted computation; never replay an unconfirmed preview as an authorized commit. Task retries must not create duplicate identities, links, or audit entries.

New series and simple append operations may publish eligible automatic results as part of the user's add-period command. If the operation affects existing later associations, require a preview before publication. Historical insertion, replacement, manual corrections with downstream effects, reopening a protected decision, and model rebuild all use this path:

1. Capture the series revision, current source fingerprints, model version, requested correction, and any new membership reservations needed for computation.
2. Compute proposed chronological results without mutating effective links. Use deterministic proposed IDs tied to the operation; avoid committing orphan identities during preview generation.
3. Show additions, removals, old/new associations, unresolved candidates, zero conflicts, protected manual decisions, and invalidated source links. Include affected counts and later periods.
4. Require explicit confirmation of the complete proposal. Cancel leaves effective state unchanged.
5. Revalidate revision, source membership, archive state, model version, and global analysis uniqueness in the apply transaction. Reject a stale preview and generate a refreshed one; never silently apply a new proposal under old confirmation.
6. Publish atomically, audit the change, and emit invalidations. Discard stale task output if the series changed while it was running.

Valid manual links remain protected. Replacement retires old occurrences from effective totals and retains their decisions in history; replacement clusters require review and cannot inherit manual links by matching IDs or labels. Retain stable tracked-problem IDs referenced by history or later manual links, even if no current occurrence remains. Show empty identities rather than deleting them silently.

A zero confirmation is a protected cell decision. Linking into that cell requires explicit revocation in the same confirmed operation; confirming zero with existing links requires their explicit removal. Audit both sides. Auto matching encountering this conflict creates a review item.

## 7. API, events, and interface

### 7.1 HTTP contracts

Add authenticated endpoints under `/api/series`, with CSRF protection on mutations and the same company-wide permissions as shared analyses. Proposed resource operations:

| Endpoint family | Purpose |
| --- | --- |
| `GET /api/series` | Paginated name search and include-archived catalog. |
| `POST /api/series` | Create with command ID; return stable series ID and revision. |
| `GET /api/series/{id}` | Coherent series summary, periods, readiness, revision. |
| `GET /api/series/{id}/trends` | Paginated problem rows and period cells from one revision, including cell state and contributor IDs. |
| `GET /api/series/{id}/review` and `/history` | Paginated unresolved decisions/evidence and audit history. |
| `POST /api/series/{id}/commands` | Rename, archive/restore, add/replace period, rename problem, link/confirm/reject/reopen, confirm/revoke zero, request recalculation. |
| `GET /api/series/{id}/tasks/{task}` | Progress, failure, and resulting snapshot or preview reference. |
| `GET /api/series/{id}/previews/{preview}` | Immutable consequences and expected revision. |
| `POST /api/series/{id}/previews/{preview}/apply` | Idempotent explicit confirmation; reject expired or stale proposals. |

Return structured validation/conflict errors and recoverable task failures. Enforce body/query limits and page bounds. Query pages must carry a snapshot revision; restart pagination if it changes rather than combining inconsistent totals. Escape all source labels, names, and evidence in browser rendering.

### 7.2 Live events

Migrate outbox/delivery records to a typed scope with resource kind and ID and an optional legacy analysis ID. Preserve existing delivery sequence values and analysis event payloads; use explicit column lists throughout the migrated writers. Series events carry `seriesId`, revision, and change kind, without a fabricated analysis ID.

Update the browser event dispatcher before adding series consumers. Analysis review handlers must ignore series events safely, including while no analysis is open. Coalesce series invalidations and refetch the affected visible snapshot; on SSE reconnect, refresh authoritative state. Preserve scroll, search, selection, and expanded review groups. A remote revision change marks an open preview stale immediately. Background completion does not steal navigation.

### 7.3 Series pane

Provide a series catalog, period list with add/replace controls and descriptive-date explanation, matching progress, review queue, chart/table, and audit access. Show archive state and disable series mutations when archived while keeping source navigation available. Keep tracked-problem names separate from source cluster labels in dialogs.

Implement a theme-aware SVG line chart backed by the same typed cells as the table, avoiding an additional chart dependency initially. Use actual period dates on the horizontal axis, display full ranges in tooltips, and allow count/percentage display. Do not connect across missing cells or uncovered calendar gaps in a way that suggests continuous observations; show confirmed zero at zero and missing values as gaps. Use a legend, non-color cues, keyboard-accessible values, and the table as the complete accessible alternative. Bound plotted series with explicit selection and render table rows incrementally.

Single-contributor cells open the correct cluster. Multiple-contributor cells open a chooser listing every contributing cluster and count, with a link for each. Extract the existing assigned-work navigation helper to preserve race protection, clear hiding filters, select the requested cluster, and autosave the resulting personal Results view. Do not invent a selected cluster for missing/zero cells. Preserve archived analysis read-only behavior.

Candidate review shows source period/cluster, suggested problem, relevant earlier examples, evidence and reason for review, with actions to confirm, choose another problem, create a new problem, reject a candidate, or defer. Provide an explicit multi-select split-combination action. A merge warning explains that the whole cluster can only contribute to one problem.

Reuse existing panel/dialog/button/input styles, CSS variables, fonts and font sizes. Test system light/dark modes, responsive layout, focus management, error/empty/loading states, and horizontal table scrolling. Keep new controls outside the existing Results tree/details grid so the prior layout regression cannot recur.

## 8. Delivery milestones

| Milestone | Work and dependencies | Exit condition |
| --- | --- | --- |
| M1: Contracts and durable series | Domain types, migration, naming/date/membership guards, command/audit patterns, typed events. | Existing database upgrades; two users can manage a series without violating membership or archive rules. AC-01–04, AC-20 foundations. |
| M2: Manual trend slice | Period artifact validation, initial problem creation, manual links, count aggregation, zero decisions, basic chart/table and source navigation. Depends on M1. | Accurate manual trends across two saved analyses, including multiple contributors and archived sources; AC-05, AC-12–17, AC-22. Not feature-complete. |
| M3: Background profiles and evaluation | Durable worker, versioned lexical profiles, candidate index, chronological matcher and offline validation harness. Depends on M1; can proceed after manual contracts stabilize. | Reproducible candidates and measured held-out results; no unsupported accuracy claim. AC-06–10 fixtures established. |
| M4: Safe automatic publication | Calibrated decisions, period-wide collision checks, manual protections, staged snapshots, downstream previews and atomic confirmation. Depends on M2/M3. | Append, historical insertion, replacement, correction, zero conflict, stale preview, and restart tests pass. AC-06–13, AC-16, AC-18–19, AC-21. |
| M5: Complete shared UI | Catalog, period controls, review/correction flows, progress/retry, history, live refresh, accessible chart/table, theme and responsive polish. Depends on M2/M4. | Two-user browser scenarios pass without changing Dashboard landing, Results filters, or review behavior. |
| M6: Release validation and operations | Quality-gate agreement/report, workload testing, backup/restore, migration rehearsal, operator instructions and final traceability review. | All AC-01–22 verified; matching quality accepted; resource limits documented; rollback/restore rehearsed. |

Within each milestone, implement one end-to-end path with tests before broadening the UI. Do not enable automatic linking for production data until the matching validation gate is met. Any temporary feature switch must clearly distinguish an incomplete validation build from the completed release.

## 9. Verification plan

### 9.1 Automated coverage

| Coverage | Scenarios |
| --- | --- |
| Domain and date tests | Inclusive boundaries, leap day/year boundary, overlap in either direction, invalid ranges, unequal lengths, gaps, descriptive dates; AC-02–04. |
| Matching tests | Stable features across run order, repeatability, frozen model weights, all-prior-period retrieval, recurrence, runner-up grouping, exclusions, low-text pending, volume changes, trusted-reference drift guard; AC-05–11, AC-19. |
| Attribution tests | One problem per occurrence, manual many-to-one only, deduplicated row membership, denominator independence, zero/no-match separation and zero conflict; AC-12–16. |
| Storage/concurrency tests | Two-series membership race, simultaneous overlapping periods, stale revisions/previews, archive during task, duplicate command, atomic rollback, protected links, replacement history, durable events; AC-01–03, AC-11, AC-18–22. |
| Recovery tests | Worker failure/restart at each stage, stale output, corrupt/missing artifact, incomplete profile, model checksum mismatch, expired preview, backup/restore and migration retry. |
| HTTP tests | Identity/CSRF enforcement, validation errors, paging/snapshot consistency, archived source versus archived series, progress/retry and safe event replay. |
| Browser tests | Create/add/review/correct/replace/zero flows; second-user updates; stale-preview warning; one/multiple contributor navigation; source archive behavior; system light/dark; keyboard operation; AC-14–22. |

Run existing Rust tests, session compatibility tests, Node filtering tests, and browser collaboration acceptance alongside new tests. Retain explicit checks for Results tree/details geometry, single-line Dashboard lists, reviewer filtering, personal-view saving, and job navigation. `.icas` exports/imports remain analysis-only and must continue to round-trip without carrying central series associations.

### 9.2 Performance and quality evidence

Use the current baseline of 500 users, approximately 50 active overall, 10 on one analysis, 1,000 saved analyses, typical 20,000 rows and maximum expected 150,000 rows/approximately 30 columns. Do not infer that all 1,000 analyses belong to one series. Define and record additional stress fixtures across period count, clusters per period, text lengths, vocabulary size, and concurrent series viewers before implementation benchmarking.

Measure profile generation, incremental append, retrospective rebuild, candidate retrieval recall, preview size/time, snapshot publication, chart query/first render, peak memory, storage growth, and latency of ordinary review commands while matching runs. Include queued clustering requests and avoid keeping all source artifacts resident. Establish series-specific budgets from these results; none have been agreed yet. Existing shared-edit/presence responsiveness remains a regression constraint.

Publish a matching validation report with held-out labels and accepted numeric gates, plus a workload report identifying build mode, hardware, fixture dimensions, and limits. Synthetic fixtures establish correctness and capacity but do not substitute for domain-reviewed matching examples.

## 10. Operations, compatibility, and completion

Store model/profile/task/preview state in the central database initially, using bounded compressed payloads where appropriate and measuring growth. If measured size requires file-backed profiles, use immutable checksummed files in an explicitly backed-up directory and publish references only after durable file creation. Neither deployment caches nor temporary task directories may be the only copy of required model provenance.

Extend the scheduled application backup path and restore tests to include series, links, zeros, history, model provenance, and profiles. On restore, recover interrupted tasks safely and verify referenced source artifacts. Document operator inspection of stalled tasks, retry behavior, profile rebuilding, disk limits, and model-version changes. Upgrading a matcher must never automatically rewrite accepted history.

Before deployment, take a verified backup, rehearse migration on a copy, and document a restore-based rollback: an older executable must not open an unsupported new schema. Explain that restoring a pre-upgrade backup loses subsequent changes and requires an explicit operator decision. No downtime or disaster-recovery target beyond existing requirements is introduced here.

Completion requires all acceptance scenarios verified, accepted historical matching results, successful existing-feature regression checks, theme/accessibility validation, restart/backup/restore validation, and updated operator documentation. Record remaining limitations explicitly. Do not mark the enhancement complete while automatic matching is disabled, numerical quality acceptance is unresolved, or retrospective correction/replacement lacks safe preview and publication.
