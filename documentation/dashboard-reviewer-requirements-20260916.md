# Dashboard and reviewer filtering requirements

## 1. Status and scope

- Product: Incident Clustering Analyzer
- Date: 2026-09-16
- Status: Requirements consolidated from stakeholder decisions; ready for review and subsequent design.
- Scope: Results reviewer filtering and display, a Dashboard landing pane, navigation, personal settings, and acceptance criteria.

This document extends [the multi-user requirements](multi-user-requirements-20260913.md). Existing collaboration, workflow, archive, identity, and persistence behavior remains applicable unless explicitly changed here. Architecture, API design, storage changes, implementation planning, and implementation are outside this document's scope.

Requirements using “shall” reflect the agreed behavior. Section 10 distinguishes compatibility interpretations and details still to settle during design from confirmed stakeholder decisions.

## 2. Purpose and terminology

Users need to find their jobs, analyses, and assigned review work immediately after signing in. They also need to identify reviewers in the Results tree and restrict their working view to particular reviewers.

| Term | Meaning |
| --- | --- |
| Analysis owner | The user who initially saves an analysis centrally. Renaming, reviewing, or reopening it does not change its owner. |
| Job user | The user who submitted a job; this determines inclusion in My jobs. |
| Cluster reviewer | The assigned reviewer for a cluster. Each theme inherits its parent cluster's reviewer. |
| Assigned work | Clusters assigned to the signed-in user and their inherited themes, across centrally saved analyses. |
| Unassigned | A cluster, or inherited theme, with no reviewer assignment. |
| Unconfirmed imported reviewer | An attribution retained from an imported file that has not been confirmed as an application-user assignment. |
| Personal analysis settings | A user's selection, filters, sorting, expansion, and pivot layout for an analysis. |
| Dashboard settings | A user's dashboard searches, filters, and expanded groups, separate from personal analysis settings. |

Ownership and reviewer assignment shall not restrict access. All authorized users retain access to every shared analysis. Cluster reviewer, SME email, implementation/action-owner email, and ETA remain separate data points. This enhancement does not introduce independent theme assignments or ownership transfer.

## 3. Results pane

### RP-1: Filter by cluster reviewer

The Results pane shall provide a reviewer filter supporting selection of multiple reviewers and the following shortcuts or categories:

- **All:** no reviewer restriction.
- **Assigned to me:** clusters assigned to the signed-in user and their themes.
- **Unassigned:** clusters with no reviewer and their themes.
- **Unconfirmed imported reviewer:** clusters with unconfirmed imported reviewer assignments and their themes.

Multiple selected reviewers/categories shall combine as alternatives: an item matching any selected reviewer/category passes the reviewer filter. This result shall combine with other filter types as an intersection, preserving the existing meanings of selection, workflow filtering, column filters, and pivot drilldown. All represents an unrestricted reviewer filter, rather than an additional restrictive selection.

Matching shall use reviewer identity, not display-name text. A theme shall match exactly the same reviewer selections as its parent cluster. An unconfirmed imported attribution shall not count as Assigned to me merely because its recorded name or email resembles the signed-in user's details.

The reviewer filter shall consistently constrain:

- The Cluster Tree.
- The incident table.
- The pivot's incident population.
- Data exports to which Results filters apply; see the session-export interpretation in section 10.

Matching parent context shall remain available where needed to display themes. A contextual heading shall not by itself include incidents excluded by active filters. Each incident shall contribute only once to a filtered incident population even when it belongs to both a displayed cluster and one of its themes.

The reviewer selection shall save automatically in the user's personal settings for that analysis and restore when those settings are loaded. Another user's filter changes shall not change the current user's view.

If assignments change while a filter is active, the visible results shall update to reflect the new assignment. A filter matching no work shall produce a clear empty state and allow the user to change or clear it.

### RP-2: Display reviewer beside workflow status

Each cluster and theme row shall display its reviewer's display name immediately to the right of its workflow-status button. Themes shall show the inherited reviewer name, without requiring users to inspect the parent row.

The display shall use:

- The assigned user's display name for confirmed assignments.
- **Unassigned** for items with no reviewer.
- **Name (unconfirmed)** for an imported, unconfirmed attribution.

The reviewer text is informational. Assignment, reassignment, claiming, and release remain in the existing workflow modal. The status button retains its existing action. Reviewer labels shall refresh when shared assignments change.

## 4. Dashboard entry and layout

### NAV-1: Landing pane and navigation

Dashboard shall be the landing pane after every successful sign-in and every application page reload, including reloads that reuse an existing authenticated session. Saved Results settings shall remain available for subsequent opening of an analysis; they shall not replace Dashboard as the landing pane.

Dashboard shall remain reachable through application navigation. Existing Analyses and My jobs controls shall remain available. Navigating to Dashboard shall not cancel a running or queued job.

### LAYOUT-1: Four sections

On sufficiently wide screens, Dashboard shall use a two-by-two layout:

| Position | Section |
| --- | --- |
| Top left | My jobs |
| Top right | My analyses |
| Bottom left | Assigned work |
| Bottom right | All analyses |

On smaller screens, sections shall stack vertically in this order: My jobs, My analyses, Assigned work, All analyses. Each list shall support scrolling and an appropriate empty-state message, including when filters produce no matches.

The new pane, controls, and reviewer displays shall use the application's existing theme, fonts, font sizes, spacing conventions, and control styling. They shall respect the operating system's light/dark preference. Long names and labels shall remain usable without obscuring status controls or preventing navigation. Interactive rows and controls shall be accessible by keyboard.

## 5. Dashboard sections

### DP-1: My jobs

My jobs shall list the signed-in user's jobs, newest first, and retain the current My jobs metadata and applicable actions. In particular, it shall retain source filename, source/processed row information appropriate to the job state, column count, job status, progress, and available-result/expiry information where applicable.

Job visibility, retention, and temporary-result availability shall follow existing job behavior. Showing a job on Dashboard shall not extend its retention or make a temporary result centrally saved. Jobs shall update automatically as their state changes.

### DP-2: My analyses

My analyses shall list centrally saved analyses owned by the signed-in user, newest first by creation date. It shall show:

- Analysis name.
- Owner.
- Creation date.
- Active or archived state.

The section shall support name search and an independent Show archived control. Archived analyses shall be excluded by default. Show archived shall include archived entries alongside active entries.

Review assignment does not confer analysis ownership. A user may own an analysis while another user reviews all its clusters.

### DP-3: Assigned work

Assigned work shall show the signed-in user's assigned clusters and inherited themes, grouped by analysis. Within each analysis, clusters shall be collapsible with themes beneath them.

The list shall show analysis name, cluster/theme ID and label, workflow status, and incident count. Analyses shall be ordered alphabetically by name, clusters by ID, and themes beneath their parent cluster. Cluster and theme incident counts represent their respective items; their overlap shall not be presented as an additive total.

All workflow statuses shall be included by default. This section shall provide its own workflow-status filter and Show archived control. Archived analyses shall be excluded by default; enabling Show archived shall include them alongside active analyses.

Workflow-status filtering shall evaluate the status of each cluster and theme. An analysis or cluster heading shall remain visible when a matching theme needs that context, even when the parent cluster's own status does not match. Contextual headings shall not imply that the parent itself matched the status filter. Matching cluster rows need not cause nonmatching themes to pass the status filter.

Assignment changes shall update membership automatically. Releasing or reassigning a cluster away from the signed-in user shall remove it and its themes from Assigned work. Newly assigned clusters shall appear if they meet the section's other filters. Empty analysis groups shall disappear.

### DP-5: All analyses

All analyses occupies the fourth quarter and shall list all centrally saved analyses available to the signed-in user, including analyses they neither own nor review. It shall not exclude owned analyses simply because they also appear in My analyses.

It shall show name, owner, creation date, and active/archived state; order entries newest first; and provide name search and its own Show archived control. Archived analyses shall be excluded by default. Show archived shall include them alongside active entries.

The identifier DP-5 is used because DP-4 was originally assigned to navigation behavior.

## 6. DP-4: Opening dashboard entries

| Entry clicked | Required behavior |
| --- | --- |
| Completed job with available results | Open its result in Results and restore the user's saved personal view where one exists. If the job is linked to a centrally saved analysis, open that shared analysis. |
| My analyses entry | Open the analysis in Results and restore the user's personal analysis settings. |
| All analyses entry | Open the analysis in Results and restore the user's personal analysis settings. |
| Assigned cluster | Open its analysis in Results, select the cluster, and clear filters that would hide the target. |
| Assigned theme | Open its analysis in Results, select the theme, expand its parent as necessary, and clear filters that would hide the target. |
| Queued or running job | Open the job's progress view; do not attempt to display results that do not exist. |
| Failed or cancelled job | Show its error or cancellation details and a resubmit action. |

For assigned-work navigation, the target's visibility shall take precedence over a previously saved view. Filters that do not prevent its visibility may remain. Once navigation has applied the selection, necessary expansion, and filter changes, the resulting view shall become the user's new automatically saved personal analysis view.

Opening an archived analysis or one of its assigned items shall preserve archive read-only behavior. Opening any item shall not change its owner, reviewer, or workflow status and shall not claim the cluster.

An unavailable or expired temporary job result shall produce an explicit unavailable state rather than open unrelated results. If an item changes or becomes unavailable between display and click, navigation shall handle the latest state and explain why the requested target cannot be opened. Resubmission shall follow the existing job prerequisites and shall not silently succeed when required input is no longer available.

## 7. Dashboard personal settings

### PREF-1: Persistence and isolation

Dashboard searches, filters, and expanded groups shall save automatically per user and follow that user across browsers and computers. They shall be separate from per-analysis personal settings and from shared analysis content.

Each dashboard section shall have independent filters. For example, enabling Show archived in My analyses shall not enable it in All analyses or Assigned work. Name searches in the two analysis lists shall be independent. Workflow-status filtering shall apply to Assigned work.

Saved dashboard settings shall restore when Dashboard is reopened. Persisting scroll position across reloads or devices is not required; preserving it during live refresh is required by LIVE-1.

## 8. Automatic updates and operating context

### LIVE-1: Keep lists current without disrupting work

Dashboard shall refresh automatically as relevant jobs, analyses, ownership information, assignments, labels, and workflow statuses change, including analysis creation, rename, archive, and restoration. Users shall not need to reload the application to discover those changes.

Refreshes shall preserve the user's current searches, filters, expanded groups, and scroll position as far as the updated content permits. Removed groups cannot retain a visible expansion or scroll target, but unrelated sections shall not reset. A refresh shall not navigate away from the user's current pane.

Loading, unavailable data, and empty results shall be distinguishable. A failed refresh shall not falsely indicate that the user's jobs or assignments have been deleted.

Existing operating expectations remain applicable: up to 500 users, approximately 50 active users overall, up to 10 concurrent users per analysis, and approximately 1,000 saved analyses. Typical input is around 20,000 rows; the largest expected operational input is 150,000 rows and approximately 30 columns. This enhancement does not reduce the application's existing functional capacity requirements.

The existing multi-user timing requirements remain in force for their existing scope. No separate numerical dashboard loading or refresh target has been agreed. Dashboard performance and representative assigned-work volumes shall be considered during design and validation.

## 9. Acceptance scenarios

| ID | Scenario and expected outcome |
| --- | --- |
| AC-01 | Sign in, then reload while authenticated: Dashboard is the initial pane both times, and application navigation can return to it from Results. |
| AC-02 | Inspect a wide and narrow viewport: the four sections use the agreed grid or stacking order, scroll appropriately, and follow system light/dark styling. |
| AC-03 | Sign in as two different users: each sees only their own jobs in My jobs and only initially owned analyses in My analyses; All analyses remains available to both. |
| AC-04 | Inspect My jobs: existing metadata and actions remain available, including filename, row information, and column count; newest jobs appear first. |
| AC-05 | Search My analyses and All analyses independently: each search affects only its section, with required columns and newest-first ordering retained. |
| AC-06 | Assign a cluster with themes to the current user: Assigned work shows the analysis, cluster, and nested themes with labels, IDs, statuses, and incident counts in the agreed order. |
| AC-07 | Filter Assigned work to a status matching a theme but not its cluster: the theme remains reachable under contextual analysis/cluster headings. |
| AC-08 | Enable Show archived in one section: that section includes active and archived entries, while other sections retain their own settings. Opening archived work remains read-only. |
| AC-09 | Open a completed job, owned analysis, or All analyses entry: Results opens the correct result and restores the personal view when available. A saved job opens its linked shared analysis. |
| AC-10 | Open assigned work hidden by saved filters: Results selects and exposes the requested item, expands the theme's parent when necessary, and automatically saves the resulting personal view. |
| AC-11 | Open queued/running and failed/cancelled jobs: progress or failure/cancellation details appear respectively, with the applicable resubmit action. An expired result is explained. |
| AC-12 | Select two reviewers: the filter includes either reviewer's items; unrelated reviewers are excluded. Other Results filters continue to constrain that population. |
| AC-13 | Use All, Assigned to me, Unassigned, and Unconfirmed imported reviewer: each produces the defined population. An unconfirmed matching name/email does not establish a confirmed assignment. |
| AC-14 | Apply a reviewer filter: tree, incident table, pivot, and applicable data exports use the same permitted incident population without counting cluster/theme membership twice. |
| AC-15 | Inspect cluster and theme tree rows: reviewer text appears immediately after the status button, themes show inherited attribution, and assignment editing remains in the workflow modal. |
| AC-16 | Another user reassigns, relabels, changes status, renames, archives, or restores relevant work: Dashboard and affected Results reviewer displays update automatically without resetting unrelated view state. |
| AC-17 | Reassign a cluster away from the current user: it and its themes disappear from Assigned work, including any now-empty analysis heading. |
| AC-18 | Change dashboard searches, filters, and expansions, then reopen on another device as the same user: settings restore. A different user's settings remain independent. |
| AC-19 | Change the reviewer filter for an analysis, then reopen it: the filter restores alongside existing personal analysis settings. Dashboard entry does not overwrite those settings by itself. |
| AC-20 | Refresh while scrolled into a populated list: searches, filters, expansions, and feasible scroll position survive. Empty and failed-loading states remain distinguishable. |

## 10. Compatibility interpretations and design follow-ups

These details make the specification reviewable without presenting unconfirmed extensions as stakeholder decisions:

1. **Session exports:** The agreed filtered-export behavior is interpreted as applying to data exports, including incident/cluster/pivot exports where supported. A portable `.icas` session is assumed to continue containing the full analysis plus the exporting user's personal settings, including the reviewer filter, rather than permanently deleting filtered-out data. A partial-session export would require a separate explicit decision.
2. **Multiple open views:** Dashboard personal settings are expected to follow the existing convention: open tabs/devices retain independent working views, with the most recently saved settings used on the next reopen. This is an inherited-behavior assumption, not a request for live synchronization of personal controls.
3. **UI details:** Exact reviewer-selector presentation, theme ordering within a cluster, search matching rules, pagination/virtualization, responsive breakpoints, and truncation/accessibility details remain design choices within the behavior specified above.
4. **Performance validation:** Agree any additional dashboard response-time targets and choose representative numbers of assigned clusters/themes during design. Existing analysis save/load and collaboration targets are not replaced.

## 11. Exclusions

This enhancement does not request new access roles, owner transfer, independent theme-reviewer assignment, automatic claiming on navigation, changes to workflow transitions, a new retention policy, or permanent deletion. It does not add `.icar` review replacement. Dashboard editing actions beyond the existing job actions and specified navigation are not required; existing analysis-management and workflow controls remain available.
