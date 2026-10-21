# Cluster trends implementation status

Date: 2026-09-18. Implements the [requirements](cluster-trends-requirements-20260917.md) and [implementation plan](cluster-trends-implementation-plan-20260917.md).

## Delivered application behavior

- Series catalog with unique normalized names, search, rename, recoverable archive, and restore. Dashboard remains the landing page.
- Centrally saved analysis membership, descriptive inclusive dates, chronological periods, global membership uniqueness, overlap rejection, and explicit source replacement.
- Versioned weighted text profiles and an exact shared-feature candidate index. Matching considers all earlier periods, confidence margin, trusted references, representative evidence, and period-wide split competition. Structured service fields and incident-ID overlap are not matching prerequisites.
- Independent tracked-problem identities/names, candidate explanations and earlier-source navigation, manual linking and multi-cluster combination, protected decisions, candidate rejection/reopening, and explicit zero/revocation.
- Durable matching tasks, persisted profile checkpoints, restart recovery, progress, and retry. Heavy profile/matching work shares admission with clustering while normal review remains independent.
- Complete impact previews and transactional publication with expected revisions, source checks, command receipts, audit history, and typed SSE events. Replaced source links remain in history rather than transferring by cluster number.
- Trend chart/table with distinct missing/zero states, all-processed-record percentages, clickable contributors, and correct Results navigation. System light/dark theme, existing typography/control styles, responsive layout, accessible labels, keyboard chart-point activation, and an accessible data table.
- Schema-3 migration retaining existing delivery cursors, backup/restore coverage, old session-format compatibility, and no central series identity in `.icas` exports.
- Offline evaluation tool and synthetic dataset generator in `examples/evaluate_trends.rs`; [usage and operations](cluster-trends-operations-20260918.md) describe deployment and later validation.

## Explicitly deferred with the user

Only two test datasets are currently available. The user confirmed that representative examples and an acceptable false-link rate will be decided after additional test data is created. Production automatic linking therefore remains disabled by default. The implemented matcher can produce automatic links in evaluation and, later, under an operator-supplied accepted validation configuration. Candidate review, manual linking, initial/new identities, counts, and corrections are available now.

Synthetic tests establish mechanics, not production accuracy. This is not an assertion that the original plan's historical matching quality gate has passed. No external service or model is enabled implicitly.

## Implementation choices relative to the plan

- Persist coherent series snapshots with separately indexed catalog, membership, tasks, history, and compressed profiles. The fine-grained logical records in the plan are represented inside snapshots where atomic publication is useful.
- Require confirmation for every computed proposal, including first-period creation and simple append. This provides one consistent publication path; eligible automatic associations are grouped in the proposal rather than requiring individual confirmation.
- Rebuilds use the earliest current period as their explicit reference. Existing models never silently adopt newly configured thresholds or weights.
- The browser loads the series context and paginated trend rows before rendering, with incremental table/review rendering. No series-specific size or latency SLA was agreed; very long histories need representative workload validation.
- Task retries use current state and generate a fresh preview. Closing a preview leaves it unapplied; an unapplied preview expires after 24 hours.

## Acceptance coverage

| Requirements | Evidence |
| --- | --- |
| AC-01–04: shared series, membership, periods, descriptive dates | Domain date tests, storage membership/publication tests, HTTP historical insertion test, two-user browser flow. Counts use immutable row membership and full processed-record totals. |
| AC-05: initial identities and separate names | Matcher initial identity tests, direct rename commands, series UI; source review data is not mutated by series commands. |
| AC-06–10: strong, ambiguous, new, returning, text-only matches | Synthetic positive/negative/ambiguity fixtures and offline labeled evaluator. Representative quality validation remains deferred. |
| AC-11–13: manual protection, splits, merges | Protected decision, duplicate-membership union, period-wide split, trusted-reference drift and ambiguous-candidate tests. No API can assign a whole cluster to multiple problems. |
| AC-14–17: counts, percentages, missing/zero, navigation | Domain cell tests, zero exclusion tests, JavaScript chart-gap tests, browser counts/zero/multiple-contributor navigation with Results geometry coverage. |
| AC-18–19: replacement and historical insertion | HTTP test adds February before January, replaces a source, and verifies replacement review and unchanged archived source. Every proposal is explicit and source-checked. |
| AC-20–22: lifecycle, concurrency, audit | Normalized archived-name test, concurrent revision race, duplicate command receipt, stale preview, typed events, audit checks and two-user browser conflict. |
| Recovery and compatibility | Reopen/profile/task recovery tests, transactional failed publication, version-1 migration with an existing delivery sequence, backup/restore including trends, existing session compatibility suite. |

## Recorded measurements

Final checks passed: 52 Rust library tests (8 explicit/ignored workload and browser tests), 3 session integration tests, 16 JavaScript tests, formatting, and Clippy with the repository's existing `too_many_arguments` allowance. Both the new series browser acceptance and existing collaboration browser acceptance passed when run explicitly. The large profile workload below also passed explicitly. Windows occasionally held a just-linked test executable open (`LNK1104`); unchanged retries completed successfully.

The four-period synthetic evaluation produced 5 automatic links, 0 false automatic links, 2 pending split occurrences, and 0 erroneous new identities. Automatic coverage of recurrent occurrences was 5/7. These deliberately simple repeated-text examples are not a production precision estimate or an approved threshold setting.

The explicit local Windows **debug-build** workload used 150,000 processed records, 150 clusters, and 515,861 weighted features. Fitting took 15,692 ms; fitting plus profile extraction took 50,087 ms. Serialized uncompressed profile data occupied 19,213,530 bytes. This measures text-profile generation, not complete retrospective series matching, database publication, peak memory, or the Linux production host. No new timing guarantee is inferred.

Browser screenshots are generated under `target/ui-theme/series-*.png` for light, dark, and narrow-screen layouts. The existing collaboration browser suite also checks Dashboard rows, reviewer filters, conflict handling, personal settings, portable imports, and Results tree/details geometry.

## Release and operation

Rebuild/restart the local application to load the new embedded UI assets. Take a stopped-service backup before upgrading an existing database. No production deployment, central-data migration on the user's live directory, or automatic-link approval is performed by this task. See [operations](cluster-trends-operations-20260918.md) for schema migration, backup/rollback, retry, and validation configuration.
