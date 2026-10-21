# Cluster trends: usage, validation, and operation

Date: 2026-09-18.

## Using series

Dashboard remains the landing page. The Series pane includes **How to create a series**, with separate CSV/XLSX and `.icas` instructions, and a **Prepare or import an analysis** shortcut to Source. After creating a series, the saved-analysis chooser opens automatically. Use **Prepare comparison**, then **Review proposal → Save to series** to confirm the addition. Open **Series**, create a unique name, then choose **Add saved analysis**. Choose a centrally saved analysis and inclusive start/end dates. Dates are descriptive; all processed records remain in the denominator. Periods cannot overlap and an analysis cannot belong to two series, including archived series.

Matching runs in the background. **Review proposal** shows proposed associations and their consequences; **Save to series** publishes the whole proposal atomically. This implementation asks for confirmation of every computed proposal, including the first period and simple appends. Within an accepted proposal, eligible automatic links do not require separate per-cluster confirmation when the matcher has been validated.

Keep successive periods in the **same series**: for example, create “Monthly incidents” once, add January, save its proposal, then add February to that same series. The proposal screen accepts the complete change or cancels it; individual associations are edited after saving through **Cluster associations → Review link**.

If an analysis is already assigned, the chooser lists its existing series with an **Open series** button instead of offering it for addition. Archived series still retain membership; open and restore the series to continue editing. Existing proposals that conflict with another series offer **Open [series name]** or **Choose another analysis**. Membership is checked again at save time to handle concurrent users. Preparation and unsuccessful saves do not move an analysis or overwrite existing trends.

Use **Cluster associations → Review link** to choose an existing tracked problem, create a new problem, or explicitly leave a cluster pending. Select multiple clusters to combine a split manually. A whole cluster can contribute to only one tracked problem; counts are unions of source row identities. Candidate evidence includes similarity scores, shared terms, and links to earlier contributing clusters. Scores are not probabilities.

The chart and table share the same counts and percentages. Choose count or percentage for the chart, and select up to 12 lines. Missing matches remain gaps; confirmed zero is an explicit attributed decision. Click a plotted point or table count to inspect the source. A cell with several contributing clusters offers a chooser. Results navigation clears filters that would hide the selected cluster and retains the existing personal-view saving behavior.

**Confirm zero** is available only when the cell has no linked cluster. A subsequent manual link must explicitly revoke that confirmation. **Reopen matching** releases a protected manual association; **Reconsider** removes an explicit candidate rejection. Every association change, confirmation, and zero decision is retained in series history. Review comments, assignments, labels, and workflow are not copied between analyses.

**Periods → Replace** replaces the analysis for the same dates. Old source links are removed from effective totals but retained in history. Replacement clusters require review; cluster numbers do not transfer identity. Historical insertion and all corrections recompute in chronological order, considering all earlier periods. A stale preview cannot overwrite changes by another user. **Retry with current state** recomputes a failed or stale proposal and requires a new review.

Archiving a series makes it read-only until restored; its name remains reserved. Archived analyses can still contribute to an editable series. Source navigation respects analysis archive restrictions. Renaming a tracked problem never changes source labels. Standalone period deletion/date editing, merging/deleting tracked identities, and trend export are outside this release.

**Delete series** permanently removes an active or archived series after an explicit confirmation. It removes its periods, tracked problems, matching decisions, history, and tasks/proposals. Saved analyses, review content, and reusable source-profile caches remain intact. Analysis memberships and the series name are released, allowing test analyses to be used in another series. All signed-in users can delete series. A stale confirmation is rejected if the series changed; other viewers are notified when deletion succeeds. There is no in-app undo; existing backups remain subject to the hosting environment's retention policy.

## Matching validation is deliberately deferred

The user confirmed on 2026-09-17 that only two test datasets currently exist and that additional representative examples and quality targets will be decided later. Therefore production automatic linking is disabled by default. Initial/new identities, candidate review, manual links, counts, corrections, and history are usable without enabling it.

The initial matcher uses normalized weighted lexical profiles with a frozen reference vocabulary weighting, bounded unseen-term weights, cluster aggregates, and diverse representative vectors. It uses text only. Persisted profiles are keyed by immutable source fingerprint and complete model configuration. Candidate retrieval retains every problem sharing any profile feature, so pruning cannot hide a nonzero runner-up. Automatic decisions require absolute strength, a margin over a competing identity, consistency with founding/manual reference evidence, and absence of split/merge or zero conflicts.

The built-in provisional thresholds are for developing candidate review and synthetic mechanics tests. They are not an approved accuracy policy. Repeated phrases in synthetic fixtures are easier than real incident paraphrases, changing templates, mixed clusters, and genuinely ambiguous problems.

### Generate and evaluate mechanics fixtures

From the repository, choose a new output directory:

```sh
cargo run --example evaluate_trends -- --generate-synthetic target/trends-fixtures
cargo run --example evaluate_trends -- target/trends-fixtures/manifest.json > target/trends-fixtures/report.json
```

The generator creates four `.icas` files and a labeled manifest covering changed incident IDs, reordered cluster numbers, recurrence after a gap, a new problem, and a possible split. It never writes application storage. The report always marks `qualityApproved: false`.

For representative evaluation, export centrally saved analyses and create a separate manifest. Paths are relative to the manifest. `truth` maps every cluster number to a reviewer-assigned recurring-problem identity; use the same truth string for the same underlying problem across periods. Use independently held-out periods after calibrating thresholds.

```json
{
  "kind": "held-out",
  "description": "Describe who labeled this dataset and how it was held out",
  "plausible": 0.30,
  "strong": 0.82,
  "margin": 0.12,
  "periods": [
    {
      "file": "january.icas",
      "start": "2026-01-01",
      "end": "2026-01-31",
      "truth": {"1": "card-payment-rejection", "2": "printer-paper-jam"}
    }
  ]
}
```

Numbers above illustrate the manifest format, not accepted thresholds. The report records source checksums, model version, decisions, candidate evidence, automatic precision/coverage, pending decisions, false automatic links, erroneous new identities, and duration. Review representative split/merge flags and drift cases separately; no aggregate metric proves all of them correct. Agree a tolerable false-link rate, useful coverage, and a review-workload target before enabling automatic links.

### Enabling a validated model later

After acceptance, maintain an operator-owned JSON file containing `report`, `approvedBy`, `plausible`, `strong`, and `margin`. The first two must identify the actual validation report and approver. Numeric thresholds must be finite, with `0 < plausible < strong <= 1` and `0 < margin <= 1`. Keep the report with deployment records.

Set `APP_TRENDS_VALIDATION` to that file's path in the application environment and restart. This enables the recorded calibrated configuration for newly initialized series models. Existing models do not change silently: open a series and choose **Rebuild matching model**, then review and confirm its impact. Rebuilding fits weights from the earliest period currently in the series and applies the current validation configuration. Manual links remain protected. Removing the environment variable affects new/rebuilt models, not previously accepted models; rebuild affected series explicitly to return them to manual review.

## Storage, tasks, and upgrades

Schema version 3 adds series snapshots, globally unique analysis membership, compressed profile cache, durable tasks/previews, and audit history. Series records and matching data stay in the same embedded Turso database; no additional service or model download is required. Series membership is not exported in `.icas` files. Importing an old session creates an independent analysis that can subsequently be added to a series.

The migration runs transactionally from schema 2, including a fresh 1-to-2-to-3 installation. Existing SSE delivery sequence numbers and analysis events are retained; series events use an explicit resource kind and series ID. Prior migrations skip newer schema versions. Profile backfill is lazy rather than a startup-wide scan of every analysis.

A single durable trend worker shares a fair heavy-computation lock with clustering. Normal review and catalog requests do not acquire that lock. Heavy work runs outside write transactions; source artifact decoding/profile extraction is performed one artifact at a time. Reusable profiles survive retries and application restarts. Intermediate calculations do not appear as saved trends. The worker requeues interrupted work, but never automatically applies an unconfirmed preview.

Inspect task progress in the series view. Preview lifetime is 24 hours. Failed/stale tasks can be retried against the current revision. An expired preview needs a new recalculation. If a source is missing or fails validation, restore the source artifact from backup before retrying. Do not repair series records by manually changing database JSON. Matching model changes go through **Rebuild matching model** and an impact preview.

The application stores an atomic JSON snapshot of each series, with separately indexed catalog/membership/task/history records and compressed profile blobs. This trades fine-grained SQL queries for coherent publication and straightforward restore. Source row memberships are retained for distinct-record counts but removed from browser snapshots; vocabulary weights are also omitted from browser responses. Catalog/trend pages are bounded to 100 entries, history to 50 per request, table/review rendering expands in batches of 100, and charts are limited to 12 selected lines. The browser currently loads series context and trend pages before rendering; very long histories require further workload measurements before committing a series-size or latency SLA.

## Backup and rollback

Use the existing stopped-service application backup command described in [multi-user operations](multi-user-operations-20260914.md). All series data, model provenance, compressed profiles, task state, and audit history are inside the database and included by that command. The referenced `.icas` artifacts remain part of the normal backup. No new external profile directory is required.

Before upgrade, take a verified backup and rehearse the migration on a copy. Restore testing must include reopening a series, source navigation, zero/manual decisions, and interrupted tasks. A pre-upgrade binary must not open schema 3. Rollback requires the compatible binary and a pre-upgrade backup, with explicit acceptance that later changes would be lost; retain the upgraded data separately. No deployment or production restore is performed by this implementation task.

## Verification commands

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings -A clippy::too_many_arguments
cargo test
node --test web_tests/*.mjs
cargo test --lib browser_series_acceptance -- --ignored --nocapture
cargo test --lib browser_collaboration_acceptance -- --ignored --nocapture
cargo test --lib trend_profiles_expected_large_analysis -- --ignored --nocapture
```

Browser tests require the existing Playwright dependencies and Chromium installation. On this Windows workspace, set `PLAYWRIGHT_BROWSERS_PATH=C:/codex/clustering-web/target/playwright-browsers`. The 150,000-row workload is an explicit measurement, not part of the default unit-test suite or a production performance guarantee.
