# Decision Log

## ADR-001: Use A Local Rust Desktop Application

- Status: Accepted
- Context: The application must run locally/offline, process up to 200,000 records, and provide a desktop GUI.
- Options:
  - Rust desktop application with `eframe`/`egui`.
  - Python desktop application.
  - Web application with local backend.
- Decision: Use Rust with `eframe`/`egui` on Windows 11. Pin UI/rendering dependencies to `egui = "0.32.0"`, `wgpu = "25.0.2"`, and `eframe = { version = "0.32.0", features = ["wgpu"] }`.
- Rationale: Rust provides strong performance and memory control for the 200,000-row workload, while `egui` supports a lightweight native desktop UI without a browser or server dependency.
- Consequences: Development requires Rust GUI expertise. Some desktop widgets may need custom egui implementation. Version upgrades for `egui`, `eframe`, and `wgpu` require an explicit architecture/dependency decision.

## ADR-002: Start With Offline Lexical Similarity Instead Of Transformer Embeddings

- Status: Accepted
- Context: Text processing must be offline and complete within 15 minutes on 4 CPUs and 16 GiB RAM.
- Options:
  - TF-IDF and n-gram sparse lexical similarity.
  - Local transformer embeddings.
  - External embedding API.
- Decision: Use sparse lexical features with word and character n-grams for the first implementation.
- Rationale: This is deterministic, fast, local, easier to inspect, and better aligned with the performance target. External APIs are disallowed.
- Consequences: Semantic matches with different vocabulary may be missed. If SME acceptance is below target, evaluate local embedding models as a later optimization.

## ADR-003: Avoid User-Specified Cluster Count

- Status: Accepted
- Context: Requirements explicitly state the user should not specify the number of clusters.
- Options:
  - K-means-style clustering with user or inferred `k`.
  - Density/graph-style clustering with automatic cluster discovery.
- Decision: Use candidate similarity graph grouping with threshold-based connected components and minimum cluster size promotion.
- Rationale: This naturally supports automatic cluster count and an `Unclustered` group.
- Consequences: Threshold tuning becomes important and must be validated against representative incident data.

## ADR-004: Apply Filters To The Result View For MVP

- Status: Accepted
- Context: Requirements require filtering the cluster view, but do not state that clustering must be recalculated for filtered subsets.
- Options:
  - Apply filters to the existing clustered result view.
  - Rerun clustering after every filter change.
- Decision: Apply filters to the result view for MVP.
- Rationale: This is faster, simpler, and supports interactive exploration without invalidating the original analysis.
- Consequences: Filtered views show subsets of clusters generated from the full dataset. Rerun-on-filter can be added later if needed.

## ADR-005: Store Mapping Profiles Separately From Analysis Sessions

- Status: Accepted
- Context: Users need to save and reload previous column mappings or analysis sessions.
- Options:
  - One combined session format only.
  - Separate mapping profiles and full analysis sessions.
- Decision: Support separate mapping profiles and full analysis sessions. Mapping profiles may be stored under the user profile. Full analysis sessions must embed the original incident data and should be saved to an explicit user-selected path.
- Rationale: Mapping reuse is lightweight, while full session reload requires preserving source data, mappings, settings, cluster assignments, labels, filters, and ignored-row summary.
- Consequences: Full session files are sensitive and may be large. The UI needs clear save/load choices and should communicate that full sessions contain incident data.

## ADR-006: Exclude Ignored Rows From Export

- Status: Accepted
- Context: Rows missing incident number or short description are ignored for clustering.
- Options:
  - Include ignored rows in export with blank or reserved cluster values.
  - Exclude ignored rows from export and show them only in the application.
- Decision: Exclude ignored source rows from Excel export.
- Rationale: Export output should focus on processed clustered results while the application still reports ignored-row counts and reasons.
- Consequences: Export row count may be lower than source row count. The UI should make this visible so users understand the difference.

## ADR-007: Use Human-Readable Sentence-Style Cluster Summaries

- Status: Accepted
- Context: Cluster labels must be scannable and meaningful to problem managers.
- Options:
  - Keyword-only labels.
  - Sentence-style summaries generated from representative terms.
- Decision: Generate concise human-readable sentence-style summaries for cluster and subgroup labels.
- Rationale: Sentence-style summaries are easier for users and SMEs to review than raw keyword lists.
- Consequences: Summary generation needs additional heuristics and tests. The implementation must still provide a non-empty fallback summary.

## ADR-008: Deliver As A Single Windows Executable

- Status: Accepted
- Context: The target environment is Windows 11 on Azure VDI.
- Options:
  - Single executable.
  - Installer.
  - Portable folder.
- Decision: Deliver the application as a single executable.
- Rationale: A single executable is simplest for local/offline use and controlled VDI deployment.
- Consequences: Packaging must account for any runtime assets, default stopword resources, icons, and configuration defaults so the executable remains self-contained or creates local config files on first run.
