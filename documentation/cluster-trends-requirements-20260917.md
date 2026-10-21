# Cluster trends across repeated analyses

## 1. Status and scope

- Product: Incident Clustering Analyzer
- Date: 2026-09-17
- Status: Requirements consolidated from stakeholder clarification; ready for review and subsequent architecture/design.
- Scope: Named analysis series, recurring-problem identification, automatic and manual cluster linking, historical trend presentation, and correction history.

This document extends the [multi-user requirements](multi-user-requirements-20260913.md) and complements the [Dashboard and reviewer requirements](dashboard-reviewer-requirements-20260916.md). It specifies observable behavior, not implementation technology. It does not authorize changing existing clustering algorithms, cluster membership, reviewer assignments, workflow states, or analysis ownership.

Requirements using “shall” express confirmed decisions or consistency rules necessary to implement those decisions. Section 10 explicitly separates remaining design choices and proposed defaults from confirmed behavior. No matching algorithm, numerical similarity threshold, accuracy target, or additional performance SLA has yet been approved.

## 2. Purpose and terminology

Repeated analyses cover separate periods. Users want to determine whether the same underlying problems are growing, shrinking, disappearing, or returning, even when each period contains entirely new incident IDs.

| Term | Meaning |
| --- | --- |
| Series | A named collection of centrally saved analyses associated with non-overlapping periods. |
| Period | An explicitly entered start and end date associated with one analysis in a series. These dates describe the dataset; they do not filter it. |
| Analysis cluster | A cluster produced within a particular saved analysis. Its number, label, and membership are local to that analysis. |
| Tracked problem | A persistent identity within a series representing a recurring underlying problem across periods. |
| Cluster occurrence/link | The association of an analysis cluster with a tracked problem for that analysis's period. |
| Text signature | A representation of cluster incident texts used to assess similarity across analyses. It is evidence for matching, not the persistent tracked-problem identity itself. |
| Automatic link | A link created by a sufficiently strong and unambiguous matching decision. |
| Protected manual decision | A user-confirmed or user-corrected link that subsequent automatic matching cannot overwrite. |
| Pending match | A cluster whose plausible tracked-problem association requires user review. |
| Confirmed zero | An explicit, attributed user confirmation that a tracked problem has zero occurrences in a period with no linked cluster. |

The scope is **clusters only**. Themes do not receive separate trend identities or counts. Tracking means recurrence of an underlying problem, not recurrence of the exact incident records or equality of generated cluster labels.

## 3. Series and periods

### SER-01: Explicit series membership

Users shall explicitly add centrally saved analyses to a named series. Temporary, unsaved results shall not participate until centrally saved. Existing historical analyses shall be eligible; users shall not need to rerun clustering to establish historical trends.

Each analysis shall belong to at most one series at a time. Each period shall contain exactly one selected analysis. A rerun for the same period shall require explicit replacement of its existing analysis rather than creating duplicate observations.

### SER-02: Period validation

Each series entry shall specify a start date and end date. Invalid date ranges and overlapping periods in the same series shall be rejected. Gaps and unequal period lengths shall be allowed.

Period order shall follow the entered dates, not analysis creation time or the order in which users add analyses. Boundary-date conventions remain to be finalized in design as recorded in section 10.

Entered dates shall be descriptive metadata. The application shall not silently exclude records whose incident dates fall outside that range. Counts shall use the actual cluster memberships in the selected analysis.

### SER-03: Management and access

Series shall support unique names, name search, renaming, recoverable archiving, and restoration. Update 2026-10-20: all signed-in users may permanently delete an active or archived series after explicit confirmation. Deletion removes its periods, matching decisions, history, and tasks/proposals, releases its name and analysis memberships, and preserves the saved analyses and their review content. Concurrent changes invalidate stale deletion confirmations; other viewers are notified.

Every authorized application user shall be able to manage series and correct cluster links, consistent with existing shared-analysis access. Analysis ownership and reviewer assignment shall not restrict these operations.

### SER-04: Archived analyses

Archived analyses shall remain eligible for historical series membership and trend display. Series links, tracked-problem names, and trend annotations shall be maintained separately from analysis review content. Changing these series records shall not unarchive an analysis or modify its read-only review content.

## 4. Matching behavior

### MATCH-01: Text-based comparison

Matching shall assess similarity of the incident texts in clusters. Initially, application, service, and other structured fields shall not be mandatory equality constraints. Consequently, similar text from different applications/services may be proposed as the same tracked problem.

Matching shall not require overlapping incident IDs. It shall not depend on local cluster numbers, record order, or equality of cluster labels. Changing a tracked-problem name shall not change its identity.

Cluster record count is the measurement to trend; growth or shrinkage alone shall not disqualify an otherwise equivalent problem. Similarity is evidence rather than a guarantee of identity, and users shall retain the ability to correct matches.

### MATCH-02: Initial identities

The first chronological period shall establish the initial tracked problems from its clusters. Their initial names shall come from the cluster labels. Subsequent analysis-label edits shall not implicitly replace independently maintained tracked-problem names.

Users shall be able to rename tracked problems without renaming analysis clusters. Problem identities shall persist independently of their names; equally worded labels alone shall not merge identities.

### MATCH-03: Match against earlier history

For a later period, matching shall consider relevant occurrences across all earlier periods in the series, rather than only the immediately preceding period. This shall allow a problem to return after one or more periods with no matched cluster.

Historical series construction shall support chronological matching even when analyses were created or added in a different order. An intervening period without a match shall not delete the tracked-problem identity.

### MATCH-04: Three matching outcomes

| Outcome | Required behavior |
| --- | --- |
| Strong match with a clear lead over competing candidates | Link automatically to the existing tracked problem, subject to manual protections and split/merge rules. |
| Plausible but ambiguous or insufficiently decisive match | Leave pending for user review; do not silently select a winner or count the cluster against several problems. |
| No plausible existing match | Create a new tracked problem, initially named from the cluster label. |

Users shall be able to distinguish automatic links, protected manual links, pending matches, and new tracked problems. The review interface shall provide the candidate associations needed to decide whether to accept a proposed link, choose another problem, or treat the cluster as a new problem.

Thresholds and the required lead over alternatives shall be calibrated against representative historical analyses during design/validation. A similarity score shall not be presented as a verified probability of correctness unless such an interpretation has actually been validated.

### MATCH-05: Manual protection

Users shall be able to confirm or correct links. Subsequent automatic matching or recalculation shall not overwrite protected manual links.

A user shall be able to resolve a pending match by selecting the appropriate tracked problem or creating a new one. Manual corrections shall change trend attribution without changing source cluster membership, labels, assignments, or workflow content.

The system shall preserve enough information to identify which decisions were automatic and which were made or confirmed by a user. Detailed algorithm provenance and explanations are design matters in section 10.

## 5. Splits, merges, and count integrity

### LINK-01: Several clusters for one problem

Users shall be able to manually associate several clusters in one period with the same tracked problem. Their incident counts shall be combined for that problem's period value.

Automatic matching shall not silently establish a split relationship or combine several cluster histories merely because individual similarities are high. Potential split cases shall be flagged for manual resolution.

### LINK-02: A cluster combining several problems

A cluster that appears to combine several previously separate problems shall be flagged for manual resolution. Its count shall not be automatically divided among those problems, and its full count shall not be duplicated across them.

For the initial scope, a cluster shall contribute its whole count to at most one tracked problem in a period. Users may resolve the association or leave it pending. Reliable allocation of portions of a merged cluster to multiple problems would require further classification and is outside this enhancement.

### LINK-03: Count each record once

Where several clusters are manually combined under one problem, the period total shall count distinct source records across those clusters. A parent cluster and its themes shall never be added together; themes are outside tracking scope.

Pending clusters shall not be attributed to a tracked problem until their links are resolved. The interface shall indicate unresolved work so users do not mistake incomplete attribution for a fully reviewed trend.

## 6. Trend view and calculations

### TREND-01: Dedicated series view

The application shall provide a dedicated series view with a line chart and table. Users shall be able to inspect tracked problems across chronological periods, including:

- Tracked-problem name.
- Period dates and associated analysis.
- Linked cluster record count, including any manually combined clusters.
- Percentage of all processed records in that period's analysis.
- Whether a value comes from a link, an explicit zero confirmation, or has no matched cluster.
- Outstanding matching decisions affecting interpretation.

The chart and table shall represent the same counts and states. Period dates shall remain visible so users can recognize gaps and unequal durations; raw counts shall not be silently converted to a daily/monthly rate.

### TREND-02: Count and percentage definition

For tracked problem P and period T:

`count(P,T) = number of distinct source records in the clusters linked to P in T`

`percentage(P,T) = 100 × count(P,T) / processedRecords(analysis(T))`

The denominator shall be the full processed-record population of the selected analysis, including processed records not attributed to a tracked problem. It shall not be limited to currently visible rows, linked clusters, or a user's personal Results filters. Ignored/unprocessed source rows are not processed records.

If the denominator is zero, percentage shall be shown as unavailable, rather than an invalid numeric result. Missing attribution shall not be converted into a numeric count or percentage. Display rounding is a design choice and shall not alter the stored/countable population.

### TREND-03: Missing values and explicit zero

A period without a linked cluster for a tracked problem shall initially display **No matched cluster**. This does not establish that the problem had no incidents; clustering thresholds or unresolved matching may have prevented a usable occurrence.

Users shall be able to explicitly confirm zero occurrences for a tracked problem and period when no cluster is linked to that cell. The application shall record who confirmed zero and when. A confirmed-zero cell shall be distinct from missing/unmatched data in both table and chart.

As a consistency rule, a zero confirmation and a linked occurrence shall not coexist in the same cell. A later proposed link conflicting with a zero confirmation shall require explicit resolution rather than silently deleting the confirmation or displaying contradictory values.

Gaps between series periods shall not be filled with invented zero-valued periods. The chart shall distinguish missing data from confirmed zero and avoid presenting unmatched periods as confirmed observations.

### TREND-04: Open underlying analysis

Clicking a populated trend value shall allow users to open the related analysis in Results and inspect the corresponding cluster. When a value combines several clusters, users shall be able to identify and open each contributing cluster; the exact navigation presentation remains a design choice.

Navigation shall expose the requested cluster even when personal filters would hide it, consistent with existing assigned-work navigation. It shall preserve archive read-only behavior. A missing value or confirmed zero shall not navigate to an invented cluster.

## 7. Corrections, replacement, and recalculation

### CHANGE-01: Replace an analysis explicitly

Replacing a period's analysis shall be an explicit action. The old analysis shall no longer supply that period's cluster occurrences or counts. The replacement shall not delete or modify the old centrally saved analysis.

Links to clusters in the replaced analysis shall require review because those clusters no longer belong to that series period. The application shall not transfer a link solely because the replacement happens to contain the same cluster number or label.

### CHANGE-02: Preview downstream impact

Replacing periods and correcting historical links can affect later automatic matches. Before applying a recalculation with downstream effects, the application shall show the affected links and require confirmation.

The preview shall distinguish proposed automatic changes, pending decisions, protected manual links, and links requiring review because their source analysis is being replaced. Protected manual links with still-valid source clusters shall remain protected.

Newly added earlier periods shall be evaluated in chronological context. Their effect on existing later matches shall follow the same preview, confirmation, and manual-protection rules rather than silently rewriting established history.

### CHANGE-03: History and shared consistency

Link corrections and confirmations shall record the responsible user and time, with the affected period, cluster, tracked problem, and previous/new association sufficient to understand the change. Zero confirmations shall also be attributed and dated.

As a consequence of allowing all users to manage series, concurrent operations shall not silently overwrite one another or leave a period containing conflicting analyses/links. If shared state changes after an impact preview, the user shall review the updated consequences before confirming a stale recalculation. The implementation mechanism is deferred to design.

Changing series attribution shall not copy comments, statuses, or assignments from one analysis cluster to another. Trend links and existing shared review content remain distinct.

## 8. Acceptance scenarios

| ID | Scenario and expected outcome |
| --- | --- |
| AC-01 | Create and name a series, add a centrally saved analysis with valid dates, then reopen it as another authorized user: membership and dates remain available. |
| AC-02 | Try to add an unsaved result, add an analysis already belonging to another series, or add a second analysis for the same period: the constraints are enforced and an explicit replacement path is available where appropriate. |
| AC-03 | Add overlapping dates or an end date before the start: reject the entry. Add a gap or a differently sized non-overlapping period: accept it. |
| AC-04 | Enter descriptive dates that differ from incident dates: source membership and counts remain unchanged; no silent date-based filtering occurs. |
| AC-05 | Build the first period: its clusters establish separate tracked identities with initial names from cluster labels. Renaming a tracked identity leaves source labels and links intact. |
| AC-06 | Analyze new incidents representing a recurring problem: a calibrated strong, unambiguous text match links automatically even with no repeated incident IDs and different cluster numbers/labels. |
| AC-07 | Supply two plausible competing matches: leave the cluster pending and let a user resolve it; neither problem receives its count automatically. |
| AC-08 | Supply a cluster with no plausible earlier match: create a new tracked problem. |
| AC-09 | A problem returns after unmatched periods: earlier history remains eligible, allowing it to link to its existing identity. |
| AC-10 | Provide similar text with different service/application values: those structured differences do not constitute mandatory rejection rules in this release. |
| AC-11 | Confirm or correct an association, then recalculate: the protected manual link remains intact unless its source analysis is explicitly replaced and the affected link goes through review. |
| AC-12 | A previous problem appears as two clusters: flag the relationship for review. After manual combination, show their distinct-record total once under the same problem. |
| AC-13 | One cluster appears to combine two tracked problems: flag it; do not divide its count automatically or count it fully for both. |
| AC-14 | Compare chart/table values with source memberships: counts agree, percentages use all processed records, and personal Results filters do not change the trend. |
| AC-15 | A period has no linked occurrence: show No matched cluster rather than zero. Confirm zero manually and verify attribution and distinct chart/table representation. |
| AC-16 | Attempt to link a cluster into a confirmed-zero cell, or confirm zero where a link exists: require explicit resolution; do not retain contradictory states. |
| AC-17 | Open a trend cell with one or several contributors: the correct analysis and contributing clusters are reachable; missing cells do not imply a cluster. Archived analyses remain read-only. |
| AC-18 | Replace an analysis or correct an earlier link: show downstream impact and require confirmation; no old cluster IDs silently become replacement cluster links. |
| AC-19 | Add historical analyses out of creation/addition order: evaluate periods chronologically, considering all earlier periods and protecting existing manual decisions. |
| AC-20 | Search, rename, archive, and restore a series: retain its historical membership, links, names, and trend decisions. Explicitly confirmed permanent deletion removes the series and its history/proposals, releases its name and analysis memberships, and preserves source analyses for reuse. |
| AC-21 | Two users change the same relationship or period while a preview is open: the stale operation cannot silently overwrite newer decisions or violate series membership constraints. |
| AC-22 | Review correction history: identify who changed which association and when, without changing source review content. |

Matching-related acceptance fixtures shall include realistic positive examples, unrelated clusters with common vocabulary, ambiguous candidates, changing volumes, and split/merge examples. The labeled examples and numerical quality gates must be agreed during validation; universal automatic accuracy is not assumed.

## 9. Compatibility and presentation

The enhancement shall preserve the existing authentication choices and shared-analysis access model. It shall not require incident data to leave the existing company hosting environment as an implicit consequence of enabling trends; any proposal for an external processing service belongs in architecture review.

The series view and matching/correction controls shall follow the existing application's colors, typography, font sizes, control styling, and system light/dark setting. Keyboard navigation and readable empty, pending, loading, and error states shall be included in design.

Existing analysis-scale and multi-user operating expectations remain the starting point for design. Series-specific sizes, matching duration, backfill cost, chart response time, and live-update targets have not yet been established. Historical analysis matching shall be supported, with its workload and progress/recovery behavior defined during design.

## 10. Design decisions and proposed defaults still to resolve

These items are not additional confirmed stakeholder decisions and need not reopen the agreed core workflow:

1. **Matching representation:** Evaluate consistent weighted text profiles as the initial candidate. Choose normalization, common-boilerplate handling, stable feature weighting, representative texts, and any semantic-model use during design. Independently fitted per-analysis weights/feature identifiers must not be assumed directly comparable.
2. **Quality and explainability:** Calibrate automatic-link thresholds and the lead over alternatives using historical examples. Decide how to explain candidates, represent uncertainty, preserve matching-method versions, and prevent gradual changes from incorrectly chaining unrelated problems into one identity.
3. **Manual rejection semantics:** Define the scope and lifetime of rejected candidates and the explicit way to reopen a protected decision. Confirmed/corrected links remain protected regardless of that choice.
4. **Series archive behavior and names:** Recommended defaults are the existing analysis conventions: names unique after trimming and case normalization, archived names reserved, archived series hidden by default and read-only until restored. Recoverable archiving and unique names are confirmed; these finer rules are proposed.
5. **Period boundaries:** Recommended default is inclusive calendar start/end dates, with adjacent periods starting the day after the previous period ends. Specify the final boundary convention before validation and interface implementation.
6. **Correction interface:** Choose the exact split/merge review flow, zero-confirmation reversal, replacement preview, and multi-cluster navigation. Standalone period removal/date editing and retiring or merging tracked-problem identities have not been requested; do not silently add destructive lifecycle operations.
7. **Presentation and operation:** Decide where series navigation appears, chart selection and missing-value rendering, rounding, saved personal series-view settings, refresh/recovery behavior, and workload limits. No export/reporting enhancement for trends is yet requested.

## 11. Exclusions

- Theme-level trend identities and independent theme matching.
- Mandatory application/service equality constraints in the initial matching scheme.
- Requiring repeated incident IDs or exactly equal text to recognize recurrence.
- Automatic apportionment of merged-cluster counts among several problems.
- Automatic rewriting of protected manual links or silent downstream recalculation.
- Implicit date filtering, automatic normalization to equal-length periods, or interpreting an unmatched period as zero.
- Automatic propagation of labels, comments, review statuses, reviewers, or workflow fields between analyses.
- A new access-role model or changes to analysis ownership.
- A committed choice of embeddings, an external AI service, or a specific signature/matching algorithm before design and validation.
