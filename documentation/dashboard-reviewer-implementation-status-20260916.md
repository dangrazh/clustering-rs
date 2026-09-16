# Dashboard and reviewer filtering delivery status

Date: 2026-09-16. Implementation of [the plan](dashboard-reviewer-implementation-plan-20260916.md) and [requirements](dashboard-reviewer-requirements-20260916.md). Local implementation and validation are complete; deployment to the company container is separate.

## Delivered behavior

- Dashboard is the authenticated landing pane on sign-in and reload, with My jobs, My analyses, Assigned work and All analyses. Existing Analyses and My jobs controls remain available.
- Analysis lists show owner, creation time and archive state, with independent search/archive controls. Assigned work groups inherited themes under collapsible clusters and supports status/archive filtering with parent context.
- Shared events update catalog/review data; visible job summaries refresh every three seconds. Updates preserve controls, expansion, loaded list depth, feasible scroll position and keyboard focus. Failed refreshes retain prior data with an error message.
- Dashboard preferences save per user centrally and follow that user across devices. Open tabs keep independent views. Analysis views additionally persist the multi-select reviewer filter.
- Results reviewer filtering supports All, Assigned to me, named reviewers, Unassigned and Unconfirmed imported reviewer; the same row population reaches tree/detail/pivot and their data exports. Each theme displays its inherited reviewer beside its workflow button.
- Dashboard navigation opens saved views or the selected assigned cluster/theme, clears filters hiding an assigned target and saves the resulting personal view. Archived analyses remain read-only. Job clicks handle available results, linked shared analyses, progress, and failed/cancelled details and resubmission prerequisites.
- Late analysis responses and background job completion cannot override later navigation. New UI uses the existing palette, typography, controls and system light/dark setting, including a vertically stacked narrow-screen Dashboard.

## Persistence and compatibility

Schema 2 adds per-user Dashboard preferences, immutable entity metadata, backfill completion markers, and indexed confirmed reviewer identities. Creation and assignment changes maintain the index transactionally. Existing installations migrate automatically and index their artifacts once before serving requests; completed analyses are skipped on resume.

Portable session format 4 retains full analysis content plus the personal reviewer filter. Explicit legacy wire structures continue reading formats 1, 2 and 3. Unconfirmed imported assignments remain distinct from signed-in identities. Backup/restore includes Dashboard preferences in the existing database backup.

The implemented API consolidates the proposed list routes into `/api/dashboard/data?offset=N`, with 500-record pages for catalog and assignment metadata, plus `/api/dashboard/preferences`. The browser assembles metadata pages before independent section filtering, so a page boundary cannot remove a required parent heading or produce an incomplete search. It renders lists incrementally, and theme content only when expanded. This keeps artifact loading out of list requests without adding a service or an additional mutable workflow store.

## Validation evidence

- `cargo test --lib --locked`: 41 passed; six explicitly ignored tests have separate prerequisites. Coverage includes migration from populated schema 1, indexed confirmed versus unconfirmed assignments, owner isolation, inherited statuses, release/reassignment, resumable backfill, preference isolation, backup/restore and the existing workflow/identity/concurrency suite.
- `cargo test --test sessions --locked`: three passed. Library compatibility tests also exercise legacy session formats 1–3 and format-4 reviewer-filter round trips.
- `node --test web_tests/*.mjs`: 13 passed; reviewer identity/category matching, deduplication, reassignment, view round trips and intersections with detail/drilldown filters, alongside existing filtering/workflow tests.
- Explicit Playwright `browser_collaboration_acceptance`: real fallback sign-in in light and dark browser contexts; existing collaboration regression plus Dashboard landing, reviewer display/filtering, hidden-theme navigation, automatic removal of reassigned work, preference persistence on reload and a second device, independent open views, and narrow-screen layout.
- Explicit `large_browser_reopen_acceptance`: 150,000 source rows, 30 columns; Chromium ready in **24,483 ms** over local HTTP, reported JavaScript heap approximately **588 MB**. This meets the existing 30-second reopen target in this run.
- Explicit `fifty_users_review_with_readers`: 500 directory users, 1,000 saved analyses, 50 active reviewers across five analyses, and 1,000 edits; p95 acknowledgment **152 ms**, maximum **176 ms** in the local debug run.
- `cargo clippy --all-targets --locked -- -D warnings -A clippy::too_many_arguments`: passed. The exception retains the existing progress API convention. Formatting and diff whitespace checks also pass.

Dashboard-specific synthetic storage test: **1,001 catalog entries and 2,000 assigned entities** were traversed in approximately **1.50 seconds**; **50 simultaneous first-page readers** completed in approximately **18.88 seconds total** on the single-threaded test runtime. These are Windows debug-build observations, not per-user latency guarantees or a company-container SLA. The test uses nonexistent artifact references to prove that listing performs no artifact reads.

Light/dark and responsive screenshots are generated under `target/ui-theme/`, including `dashboard-light.png`, `dashboard-dark.png`, `dashboard-populated-dark.png`, and `dashboard-narrow-dark.png`. These local artifacts are intentionally not versioned.

## Operator handoff and validation limits

Restart using the normal launcher to build/load the updated application. Take a stopped-service backup before upgrading an existing installation. First startup may take longer while indexing old analyses; the logs identify progress and any unreadable artifact. See [operations guidance](multi-user-operations-20260914.md#dashboard-upgrade-schema-2-session-format-4).

No company Linux container or production dataset was used for these measurements. Rehearse first-start indexing against a backup of the actual catalog and confirm deployment latency/resource behavior before broad rollout. A rollback to the old binary requires a compatible pre-upgrade backup; it cannot assume support for schema 2 or session format 4. No Entra configuration or extra infrastructure is needed for this enhancement.

## Follow-up UI corrections

Dashboard rows now keep their title/action and metadata on one horizontal line, with metadata to the right. Each quadrant retains its own overflow scrolling for content wider than the available space, and continues to use the existing system-aware colors and typography.

Fixed a Results grid regression: the reviewer filter was inserted as a fourth direct child of the three-column tree/splitter/details grid. That displaced the tree into the 8-pixel splitter track and moved the details pane to a second grid row. The filter now sits beside the workflow filter, outside the grid. A browser regression test reproduced the 8-pixel tree before the fix and now asserts useful tree/table widths and side-by-side positioning. Browser acceptance passed in light and dark modes, as did the 13 JavaScript tests. Screenshots include `results-layout-light.png`, `results-layout-dark.png`, and `dashboard-single-line-light.png`/`dashboard-single-line-dark.png` under `target/ui-theme/`.
