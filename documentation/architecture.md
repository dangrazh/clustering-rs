# Technical Specification: Incident Clustering Analyzer

## 1. Purpose

Build a local/offline desktop application that imports incident exports from Excel or CSV, clusters similar incident records by selected text fields, supports interactive cluster exploration, and exports the original data unchanged with added cluster metadata.

This specification targets a Rust implementation on Windows 11. The desktop UI uses `eframe`/`egui` with the `wgpu` renderer.

## 2. Architecture Drivers

- Local/offline processing is mandatory; no incident text may leave the workstation.
- Target workload is 200,000 records on Azure VDI `Standard_D4ds_v4` with 4 CPUs and 16 GiB RAM.
- End-to-end import, similarity analysis, and clustering must complete within 15 minutes.
- Input must support `.xlsx` and `.csv`.
- Export must preserve all original source columns unchanged and add `Cluster ID`, `Cluster Label`, and `Cluster Size`.
- The GUI must support import, mapping, clustering, exploration, filtering, session reload, and export without command-line usage.
- Clustering quality matters: at least 75% of the top 20 largest clusters should be accepted by SME review.
- Target operating system is Windows 11.
- Delivery target is a single executable.
- Open-source crates with permissive licenses are allowed.

## 3. Recommended Architecture

Use a modular Rust desktop application with an `eframe`/`egui` GUI, a local data-processing core, and file-based persistence for mappings and sessions.

Pinned UI/rendering dependencies:

```toml
egui = "0.32.0"
wgpu = "25.0.2"
eframe = { version = "0.32.0", features = ["wgpu"] }
```

Primary modules:

- `app`: GUI state machine, screen routing, progress display, and user interactions.
- `io`: CSV/XLSX import, worksheet discovery, source row preservation, export writer.
- `schema`: column mapping, optional filter field mapping, validation, saved mapping format.
- `text`: language-aware normalization, tokenization, stopword handling, n-gram generation, feature extraction.
- `clustering`: vectorization, approximate similarity grouping, cluster promotion, subgroup generation.
- `labels`: cluster and subgroup sentence-style summary generation.
- `session`: save/load of mappings and analysis session state.
- `model`: shared domain types for incident rows, processed incidents, clusters, subgroups, filters, and ignored rows.
- `worker`: background processing orchestration, cancellation, progress, and message passing to the GUI.

## 4. Component Design

### 4.1 GUI

Use `eframe`/`egui` for a native desktop GUI.

Core screens:

- Import screen: file picker, workbook worksheet selection, file preview.
- Mapping screen: mandatory incident number and short-description mapping; optional text/filter/date mappings.
- Run screen: minimum cluster size input, clustering start, progress, cancellation.
- Results screen: cluster tree ordered by size descending, subgroup/theme drill-down, incident detail view, ignored row summary.
- Export screen: target path selection and export status.
- Session screen: save/load mappings and analysis sessions.

The GUI must never run import or clustering work on the UI thread. Long-running work runs in a background worker and reports progress through channels.

The mapping UI should suggest likely defaults from common export columns:

- Incident number: `INC Number`
- Short description: `INC Short Description`
- Category: `Category`
- Priority: `Priority`
- Assignment group: `Assignment Group`

### 4.2 Import

Represent imported source data as:

- Header names in original order.
- Rows as ordered cell values matching the original source columns.
- Parsed metadata for selected mapped fields.

CSV import should stream records where possible. XLSX import should read the selected worksheet and preserve displayed cell values for export. For memory control, store source rows compactly and avoid repeated string copies during text processing.

Rows missing incident number or short description are excluded from clustering and recorded as ignored rows with:

- Source row index.
- Missing incident number flag.
- Missing short-description flag.
- Optional preview values for user display.

### 4.3 Text Processing

Build a deterministic offline text pipeline:

1. Concatenate short description plus selected optional text fields with weighting biased toward short description.
2. Normalize case, whitespace, punctuation, digits, and common ticket-system noise.
3. Treat English as the most common language while supporting German, French, Italian, and mixed-language text without requiring a single language per row.
4. Apply stopword handling for German, French, Italian, and English.
5. Generate word n-grams and character n-grams to handle short, noisy, and mixed-language descriptions.
6. Build sparse TF-IDF-style feature vectors.

Initial implementation should prefer robust lexical similarity over local transformer embeddings because the performance and offline constraints are strict and the requirements do not mandate semantic embeddings.

### 4.4 Clustering

Use an automatic clustering approach that does not require a user-provided cluster count.

Recommended first implementation:

- Generate sparse text vectors from normalized tokens and character n-grams.
- Use locality-sensitive hashing or nearest-neighbor candidate generation to avoid all-pairs comparison.
- Build a similarity graph only for candidate pairs above a configured similarity threshold.
- Extract connected components as candidate clusters.
- Promote components with size greater than or equal to the configured minimum useful cluster size.
- Place all remaining processed incidents into `Unclustered`.

Subgroups/themes:

- For each promoted primary cluster, rerun local feature extraction within the cluster.
- Generate 2-10 subgroups using tighter similarity thresholds or lightweight divisive grouping.
- If a cluster is too small or too homogeneous, generate one subgroup named from its representative keywords.

Automatic threshold defaults must be validated against representative incident exports. Keep thresholds configurable internally for tuning, but do not expose cluster count to the user.

### 4.5 Cluster Summaries

Generate human-readable sentence-style summaries for clusters and subgroups:

- Compute terms with high within-cluster frequency and high contrast against the full corpus.
- Prefer short phrases or bigrams when available.
- Remove terms that are globally common or operationally unhelpful.
- Compose a concise sentence-style summary from representative keywords and phrases, for example `Password reset failures for SAP users`.
- Ensure every cluster and subgroup has a non-empty fallback summary such as `Similar incidents in cluster <id>`.

The exported `Cluster Label` field should contain the same human-readable summary shown in the UI.

### 4.6 Filtering

Filters are applied to the result view, not to the already-computed clustering, unless the user explicitly reruns analysis on a filtered dataset in a later release.

Supported filters:

- Assignment group.
- Service.
- Category.
- Configuration item.
- Date range from selected date column.

Unavailable filter mappings should hide or disable the corresponding controls.

### 4.7 Export

Export to `.xlsx`.

For each processed source row:

- Preserve all original source columns unchanged.
- Add `Cluster ID`, `Cluster Label`, and `Cluster Size`.
- Mark unclustered processed incidents with `Unclustered`.

Rows ignored because incident number or short description is missing are excluded from export. They remain visible in the application ignored-row summary.

Use stable exported field names exactly as specified in `requirements.md`.

### 4.8 Session Persistence

Support two persistence levels:

- Mapping profile: file-independent column mapping preferences and optional field roles.
- Analysis session: embedded original incident data, source file reference, mapping, run parameters, cluster results, labels, filters, and ignored row summary.

Use a versioned local JSON or binary session format. Full analysis sessions contain incident data and must be treated as sensitive local files. Mapping profiles may be stored under the user profile; exports and full session files should be saved to explicit user-selected paths.

## 5. Data Model

Core domain types:

- `ColumnRole`: incident number, short description, additional text, assignment group, service, category, configuration item, date.
- `ColumnMapping`: selected source column indices by role.
- `SourceTable`: headers plus original rows.
- `IncidentRecord`: processed row index, incident number, selected text, filter values, parsed date.
- `IgnoredRow`: row index and missing-field reason.
- `TextFeatures`: sparse feature representation.
- `Cluster`: id, summary label, size, incident row indices, subgroups.
- `Subgroup`: id, summary label, incident row indices.
- `AnalysisRun`: mapping, settings, generated clusters, unclustered row indices, ignored rows, timing metrics.

## 6. Performance Strategy

- Use streaming or chunked import where practical.
- Avoid O(n^2) pairwise similarity over 200,000 records.
- Use sparse vectors and candidate generation before similarity scoring.
- Parallelize CPU-heavy text processing and scoring with `rayon`.
- Cache normalized text and feature vectors during a run.
- Measure import, vectorization, candidate generation, clustering, labeling, and export separately.
- Use the available synthetic test data for early tuning and 200,000-row performance validation, then supplement it with representative anonymized exports if they become available.

## 7. Security And Privacy

- No network calls for text processing or clustering.
- Do not add telemetry.
- Treat source files, exports, and full session files as sensitive local data because saved analysis sessions embed original incident data.
- Avoid logging raw incident descriptions by default.
- Provide clear user control over where exported results and saved sessions are written.

## 8. Testing Strategy

Test levels:

- Unit tests for text normalization, language stopwords, feature extraction, summary generation, mapping validation, and ignored-row detection.
- Integration tests for CSV import/export and XLSX worksheet import/export.
- Clustering tests using the available synthetic datasets with known groups.
- GUI smoke tests for app state transitions where feasible.
- Performance test for 200,000 records on target VDI hardware.
- Offline/privacy test confirming the core workflow does not require network access.

## 9. MVP Boundary

MVP includes all `Must` requirements, but keeps the clustering implementation lexical and deterministic.

Later-phase candidates:

- More advanced local embeddings if lexical clustering and sentence-style summary generation miss the 75% SME acceptance target.
- Rerun clustering on filtered subsets.
- Cluster explanation features, currently out of scope.
- Richer SME validation workflow, currently out of scope.
