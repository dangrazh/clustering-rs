# Requirements

## 1. Document Control
- Project: Incident Clustering Analyzer
- Version: 1.0
- Date: 2026-04-20
- Author (Requirements Engineer): Codex
- Status: Final

## 2. Problem Statement
- Problem managers need a local/offline desktop application to identify recurring incident patterns from already-exported Excel or CSV incident data.
- The source data can contain up to 200,000 incident records. Manual review at this scale is inefficient, and similar incidents are difficult to detect reliably from short descriptions alone.
- The application must analyze incident text similarity, automatically cluster similar tickets, preserve the link to full source incident data, and allow the problem manager to explore large clusters as candidates for problem management and ticket-volume reduction.

## 3. Goals and Success Metrics
- Goal G1: Identify recurring incident groups that are meaningful for problem management.
  - Metric: SME acceptance rate for the top 20 largest clusters.
  - Target: At least 75% of the top 20 largest clusters are accepted by the SME as meaningful.
- Goal G2: Process large incident exports efficiently.
  - Metric: Processing time for 200,000 incident records.
  - Target: Complete import, text similarity analysis, and clustering in under 15 minutes on the intended local machine.
- Goal G3: Preserve incident context for analysis and follow-up.
  - Metric: Export completeness.
  - Target: Export includes all original source columns unchanged plus required cluster information.

## 4. Stakeholders
- Sponsor: Problem Manager
- Decision Makers: Problem Manager
- Users: Problem Manager
- Other Stakeholders: Subject matter experts validating cluster usefulness

## 5. Scope
### In Scope
- Desktop GUI application for local/offline use.
- Import of already-exported `.xlsx` and `.csv` incident data files.
- Manual user selection of workbook, worksheet where applicable, incident number column, short-description column, optional additional text columns, and date filter column.
- Text similarity analysis based primarily on incident short description.
- Optional inclusion of additional text fields such as description or resolution text in the similarity analysis.
- Automatic clustering of similar incident tickets.
- Automatic selection of the number of clusters.
- User-configurable minimum useful cluster size, with 50 incidents as the default.
- Placement of tickets without a meaningful cluster into an `Unclustered` group.
- Interactive exploration of clustered data.
- Tree view ordered by cluster size descending.
- Automatically generated cluster labels as human-readable sentence-style summaries using representative keywords and phrases.
- Required subgroup/theme drill-down within clusters before individual incident drill-down.
- Filtering by assignment group, service, category, date range, and configuration item.
- Export processed rows to Excel with all original columns unchanged plus cluster columns; ignored rows are excluded from export.
- Display of ignored row information after clustering where incident number or short description is missing.
- Language-specific handling for English, non-English, and mixed-language incident text.
- Saving and reloading of previous column mappings or analysis sessions, with full analysis sessions embedding original incident data.

### Out of Scope
- Direct integration with ticketing systems such as ServiceNow, Jira, Remedy, or similar systems.
- Cloud-based or externally hosted text processing.
- Sending incident content to external services.
- Manual SME validation workflow tracking inside the application.
- Similarity explanation features such as nearest-neighbor explanation, shared phrase explanation, or detailed clustering rationale.
- Deduplication or special handling of duplicate incident numbers or duplicate short descriptions.

## 6. Assumptions and Constraints
### Assumptions
- The main user is a problem manager analyzing exported incident data.
- Incident number and short description are the only mandatory source columns.
- Additional columns such as assignment group, category, service, priority, status, created date, resolved date, configuration item, and resolution notes may or may not be present or populated.
- The user can identify which source columns should be used for import, text analysis, and filtering.
- SME validation of cluster usefulness will be performed outside the application.
- English is expected to be the most common incident-description language.
- Common source columns include `INC Number`, `INC Short Description`, `Category`, `Priority`, and `Assignment Group`.

### Constraints
- All processing must happen locally/offline.
- The application must support up to 200,000 records.
- The application must complete processing for 200,000 records in under 15 minutes on a Microsoft Azure VDI using the `Standard_D4ds_v4` profile with 4 CPUs and 16 GiB memory.
- The application must support `.xlsx` and `.csv` input files.
- Export must preserve all original source columns unchanged.
- Rows missing incident number or short description must be ignored rather than processed.
- Ignored rows are excluded from Excel export.

## 7. Functional Requirements
- FR-001:
  - Description: The application shall provide a desktop GUI for importing, processing, exploring, and exporting clustered incident data.
  - Rationale: The primary user prefers an interactive desktop application for local problem-management analysis.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The user can complete import, clustering, exploration, and export without using a command line.

- FR-002:
  - Description: The application shall allow the user to select a `.xlsx` or `.csv` source file.
  - Rationale: Incident data is provided as already-exported files.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The user can select a `.xlsx` file as input.
    - AC-002: The user can select a `.csv` file as input.
    - AC-003: Unsupported file types are rejected with a clear message.

- FR-003:
  - Description: For `.xlsx` files, the application shall allow the user to select the worksheet containing source data.
  - Rationale: Excel workbooks may contain multiple worksheets.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The application lists available worksheets for a selected `.xlsx` workbook.
    - AC-002: The user can choose which worksheet to process.

- FR-004:
  - Description: The application shall allow the user to map the incident number column and short-description column before processing.
  - Rationale: Source files may not use fixed column names.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The user can select the incident number column.
    - AC-002: The user can select the short-description column.
    - AC-003: Processing cannot start until both mandatory columns are selected.

- FR-005:
  - Description: The application shall allow the user to optionally select additional text columns for similarity analysis.
  - Rationale: Some datasets contain useful text fields such as description or resolution text.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The user can process data using only the short-description column.
    - AC-002: The user can include one or more additional text columns in the similarity analysis.

- FR-006:
  - Description: The application shall ignore rows where the incident number or short description is missing.
  - Rationale: These rows do not contain the minimum data required for traceable clustering.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: Rows with missing incident number are excluded from clustering.
    - AC-002: Rows with missing short description are excluded from clustering.
    - AC-003: Rows with duplicate incident numbers or duplicate short descriptions are processed as provided.

- FR-007:
  - Description: The application shall report ignored rows caused by missing incident number or missing short description.
  - Rationale: The user needs visibility into source data excluded from analysis.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The application shows the number of ignored rows after clustering completes.
    - AC-002: The application displays ignored row information in the application.
    - AC-003: The displayed information identifies whether each ignored row was missing incident number, short description, or both.

- FR-008:
  - Description: The application shall perform text similarity analysis on the selected text fields.
  - Rationale: Similarity analysis is required to identify recurring incident patterns.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: Each processed incident receives similarity-analysis treatment based on the selected text fields.
    - AC-002: The analysis supports datasets up to 200,000 records.

- FR-009:
  - Description: The application shall automatically cluster similar incidents without requiring the user to specify the number of clusters.
  - Rationale: The user expects automatic discovery of recurring incident groups.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The user can start clustering without entering a target cluster count.
    - AC-002: The application assigns each processed incident either to a generated cluster or to the `Unclustered` group.

- FR-010:
  - Description: The application shall allow the user to configure the minimum useful cluster size, with 50 incidents as the default value.
  - Rationale: The problem manager considers 50 incidents the default minimum useful size, but different datasets may require adjustment.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The default minimum useful cluster size is 50 incidents.
    - AC-002: The user can change the minimum useful cluster size before clustering.
    - AC-003: Clusters below the configured minimum useful size are not presented as primary clusters.
    - AC-004: Incidents that do not belong to a meaningful cluster are placed in `Unclustered`.

- FR-011:
  - Description: The application shall generate a human-readable sentence-style summary label for each cluster.
  - Rationale: The cluster list must be scannable by the problem manager.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: Each generated cluster has a non-empty human-readable summary label.
    - AC-002: Cluster labels are derived automatically from representative keywords and phrases without requiring manual naming.

- FR-012:
  - Description: The application shall provide an interactive tree view of clustered incidents.
  - Rationale: The user needs to explore clusters and drill down into ticket details.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The top-level tree lists clusters ordered by cluster size descending.
    - AC-002: Each top-level cluster displays cluster label and cluster size.
    - AC-003: Expanding a cluster shows subgroups or themes before individual incidents.
    - AC-004: The user can drill down to individual incident records.

- FR-013:
  - Description: The application shall generate subgroups or themes within each primary cluster.
  - Rationale: Large clusters need additional structure before the user reviews individual incidents.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: Each primary cluster can be expanded to show subgroup or theme entries.
    - AC-002: Subgroup or theme entries can be expanded to show related incident records.

- FR-014:
  - Description: The application shall allow filtering by assignment group, service, category, date range, and configuration item where those columns are available.
  - Rationale: The problem manager needs to narrow cluster analysis by operational dimensions.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The user can select source columns corresponding to assignment group, service, category, and configuration item where available.
    - AC-002: The user can select a date column for date range filtering.
    - AC-003: The user can filter the cluster view by selected filter values.
    - AC-004: Filters that depend on unavailable columns are not required to be active.

- FR-015:
  - Description: The application shall export clustered results to Excel.
  - Rationale: The user needs to continue analysis and reporting outside the application.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The export includes all original source columns unchanged for processed rows.
    - AC-002: The export adds `Cluster ID`, `Cluster Label`, and `Cluster Size` columns.
    - AC-003: Incidents in the `Unclustered` group are identifiable in the export.
    - AC-004: Rows ignored because incident number or short description is missing are excluded from export.

- FR-016:
  - Description: The application shall support language-specific handling for English, German, French, Italian, and mixed-language incident descriptions.
  - Rationale: Incident descriptions are expected to be primarily English but may contain German, French, Italian, or mixed-language text, and clustering quality depends on appropriate language handling.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The application can process English incident descriptions.
    - AC-002: The application can process German incident descriptions.
    - AC-003: The application can process French incident descriptions.
    - AC-004: The application can process Italian incident descriptions.
    - AC-005: The application can process mixed-language incident descriptions containing German, French, Italian, and English text.
    - AC-006: Language handling does not require sending incident text to external services.

- FR-017:
  - Description: The application shall save and reload previous column mappings or analysis sessions.
  - Rationale: The user may repeatedly analyze exports with similar structures and needs to avoid remapping fields each time.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The user can save column mappings for later reuse.
    - AC-002: The user can reload saved column mappings.
    - AC-003: The user can save and reload a previous analysis session or equivalent analysis state.
    - AC-004: Saved analysis sessions embed the original incident data so they can be reloaded without requiring the original source file.

## 8. Non-Functional Requirements
- NFR-001:
  - Category: Performance
  - Requirement: The application shall process up to 200,000 records in under 15 minutes on a Microsoft Azure VDI using the `Standard_D4ds_v4` profile with 4 CPUs and 16 GiB memory.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: A test dataset with 200,000 records completes import, similarity analysis, and clustering in less than 15 minutes on a Microsoft Azure VDI using the `Standard_D4ds_v4` profile with 4 CPUs and 16 GiB memory.

- NFR-002:
  - Category: Privacy
  - Requirement: The application shall perform all processing locally/offline.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: Incident text is not sent to external services.
    - AC-002: The application can complete import, processing, exploration, and export without an internet connection.

- NFR-003:
  - Category: Usability
  - Requirement: The application shall support a problem manager in completing the core workflow through GUI interactions.
  - Priority: Must
  - Acceptance Criteria:
    - AC-001: The user can select input data, map columns, run clustering, inspect cluster results, and export results through visible GUI controls.

- NFR-004:
  - Category: Maintainability
  - Requirement: Requirement identifiers and exported cluster fields shall remain stable across releases unless explicitly changed.
  - Priority: Should
  - Acceptance Criteria:
    - AC-001: Exported cluster columns retain the names `Cluster ID`, `Cluster Label`, and `Cluster Size`.

## 9. Data Requirements
- DR-001:
  - Description: Source incident records must include an incident number and short description to be processed.
  - Source/Owner: User-provided `.xlsx` or `.csv` incident export
  - Sensitivity: Potentially sensitive operational incident data
  - Retention: No retention requirement specified

- DR-002:
  - Description: Optional source fields may include assignment group, category, service, priority, status, created date, resolved date, configuration item, description, and resolution notes.
  - Source/Owner: User-provided `.xlsx` or `.csv` incident export
  - Sensitivity: Potentially sensitive operational incident data
  - Retention: No retention requirement specified

- DR-003:
  - Description: Exported clustered results must include all original source columns unchanged for processed rows plus `Cluster ID`, `Cluster Label`, and `Cluster Size`; ignored rows are excluded.
  - Source/Owner: Application-generated export
  - Sensitivity: Same as source data
  - Retention: User-managed outside the application

- DR-004:
  - Description: Ignored row display must identify rows skipped due to missing incident number or missing short description.
  - Source/Owner: Application-generated display
  - Sensitivity: Same as source data
  - Retention: No export or retention requirement specified

- DR-005:
  - Description: Saved mappings or analysis sessions must preserve enough configuration for the user to reload previous column selections or analysis state. Full analysis sessions must embed original incident data.
  - Source/Owner: Application-generated saved configuration or session data
  - Sensitivity: Mapping profiles may be low sensitivity; full analysis sessions are sensitive because they include incident data.
  - Retention: User-managed inside or outside the application

## 10. Dependencies
- DEP-001:
  - Description: Availability and quality of user-provided `.xlsx` or `.csv` incident exports.
  - Owner: Problem Manager
  - Impact: Missing or inconsistent data can reduce clustering quality or filter availability.

- DEP-002:
  - Description: SME validation of cluster usefulness.
  - Owner: Subject matter expert
  - Impact: Required to measure whether the top 20 largest clusters meet the 75% meaningfulness target.

- DEP-003:
  - Description: Microsoft Azure VDI using the `Standard_D4ds_v4` profile with 4 CPUs and 16 GiB memory.
  - Owner: Problem Manager
  - Impact: Required to verify the 15-minute performance target for 200,000 records.

## 11. Risks
- RISK-001:
  - Description: Short descriptions may be too brief, inconsistent, or noisy to produce meaningful clusters.
  - Likelihood: Medium
  - Impact: High
  - Mitigation: Allow optional inclusion of additional text columns such as description or resolution text.

- RISK-002:
  - Description: Local/offline processing may constrain available text similarity and clustering approaches.
  - Likelihood: Medium
  - Impact: Medium
  - Mitigation: Treat local/offline processing as a mandatory architectural constraint during solution design.

- RISK-003:
  - Description: Processing 200,000 records in under 15 minutes may depend heavily on local machine resources and source file complexity.
  - Likelihood: Medium
  - Impact: High
  - Mitigation: Define a representative performance test dataset and validate early.

- RISK-004:
  - Description: Required subgroup/theme generation may be difficult to make consistently meaningful across all datasets.
  - Likelihood: Medium
  - Impact: Medium
  - Mitigation: Validate subgroup/theme usefulness with SME review as part of acceptance testing.

## 12. Open Questions
- None.

## 13. Sign-off Criteria
- The requirements are accepted when the problem manager confirms that the documented scope, functional requirements, non-functional requirements, data requirements, dependencies, and risks accurately describe the requested first version.
- The requirements are ready for architecture handoff because all `Must` requirements are understood and no open questions remain.
