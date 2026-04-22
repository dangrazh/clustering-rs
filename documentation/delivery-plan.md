# Implementation Plan

## Phase 0: Project Foundation

Deliverables:

- Rust workspace and crate structure.
- `eframe`/`egui` application shell pinned to `egui = "0.32.0"`, `wgpu = "25.0.2"`, and `eframe = { version = "0.32.0", features = ["wgpu"] }`.
- Domain model modules.
- Basic logging without raw incident text.
- CI or local validation commands.
- Permissive-license dependency review.

Acceptance:

- The app opens to an import screen.
- The app runs on Windows 11.
- `cargo fmt`, `cargo clippy`, and `cargo test` pass for the initial workspace.

## Phase 1: Import And Mapping

Deliverables:

- CSV file selection and preview.
- XLSX file selection, worksheet listing, and selected worksheet preview.
- Column mapping UI for mandatory and optional fields.
- Suggested mappings for common source columns: `INC Number`, `INC Short Description`, `Category`, `Priority`, and `Assignment Group`.
- Validation that incident number and short description are selected before processing.
- Ignored-row detection for missing mandatory fields.
- Save/load mapping profiles.

Acceptance:

- The user can load CSV and XLSX inputs.
- The user can map mandatory columns and optional text/filter/date columns.
- Rows missing incident number or short description are counted and displayed.

## Phase 2: Text Pipeline And Baseline Clustering

Deliverables:

- Text normalization optimized for English-first data while supporting German, French, Italian, and mixed text.
- Stopword handling and n-gram feature extraction.
- Sparse vectorization.
- Candidate similarity generation without all-pairs comparison.
- Automatic cluster generation.
- Minimum useful cluster size setting with default 50.
- `Unclustered` assignment.

Acceptance:

- The user can start clustering without specifying cluster count.
- Every processed incident is assigned to a cluster or `Unclustered`.
- The available synthetic dataset produces expected recurring groups.

## Phase 3: Cluster Labels, Subgroups, And Exploration

Deliverables:

- Human-readable sentence-style summaries for clusters.
- Human-readable sentence-style summaries for subgroups/themes.
- Subgroup/theme generation inside primary clusters.
- Results tree ordered by cluster size descending.
- Incident detail drill-down.
- Ignored-row display after clustering.

Acceptance:

- Each cluster has a non-empty sentence-style summary.
- Expanding a cluster shows subgroup/theme entries before incident rows.
- The user can drill down to original incident details.

## Phase 4: Filtering And Export

Deliverables:

- Filter controls for mapped assignment group, service, category, configuration item, and date range.
- Result-view filtering.
- Excel export for processed rows, preserving original columns and adding `Cluster ID`, `Cluster Label`, and `Cluster Size`.
- Export handling for `Unclustered`.
- Exclusion of ignored rows from export.

Acceptance:

- Unavailable filters are disabled or hidden.
- Exported workbook includes all original source columns unchanged for processed rows.
- Exported cluster columns have stable required names.
- Ignored rows are not exported and the application reports the ignored-row count/reasons.

## Phase 5: Session Persistence

Deliverables:

- Save/load analysis session format.
- Versioned session schema.
- Embedded original incident data in full analysis sessions.
- Session reload into results view.
- Sensitive-data warning where sessions include source data.

Acceptance:

- The user can reload a prior analysis session or equivalent analysis state.
- Loaded sessions preserve cluster results and mappings.
- Loaded sessions do not require the original source file to still be available.

## Phase 6: Performance And Quality Validation

Deliverables:

- 200,000-record performance test harness.
- Timing breakdown for import, vectorization, clustering, labeling, and export.
- Memory-use observations on target Azure VDI.
- SME evaluation package for top 20 largest clusters.
- Threshold tuning based on representative data.
- Single-executable Windows packaging validation.

Acceptance:

- Import, similarity analysis, and clustering complete under 15 minutes on `Standard_D4ds_v4`.
- At least 75% of the top 20 largest clusters are accepted as meaningful by SME review.
- The application can be delivered and launched as a single Windows executable.

## Suggested Crates

- GUI: `eframe = { version = "0.32.0", features = ["wgpu"] }`, `egui = "0.32.0"`, `egui_extras`
- Rendering: `wgpu = "25.0.2"`
- CSV: `csv`
- Excel read: `calamine`
- Excel write: `rust_xlsxwriter`
- Parallelism: `rayon`
- Serialization: `serde`, `serde_json`
- Errors: `thiserror`, `anyhow`
- File dialogs: `rfd`
- Date parsing: `chrono`
- Logging: `tracing`, `tracing-subscriber`

Final crate choices should be confirmed during project setup against maintenance status, permissive license compatibility, API fit, and Windows 11 platform support.

## Key Risks And Mitigations

- Clustering quality may miss the SME acceptance target. Mitigate with early representative-data testing and threshold tuning.
- XLSX files may be large or contain inconsistent cell types. Mitigate with worksheet preview, robust cell display conversion, and early large-file tests.
- Memory pressure may appear at 200,000 records. Mitigate with compact row storage, sparse vectors, and measured allocations.
- Subgroups may be noisy. Mitigate with fallback single-theme subgroup and SME review.
- Full session files contain sensitive source data. Mitigate with clear user-controlled save paths, explicit warnings, and no implicit cloud sync.
