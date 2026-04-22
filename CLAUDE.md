# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Incident Clustering Analyzer: a local/offline Windows desktop app that imports incident exports (CSV/XLSX), clusters similar records by text similarity, and exports the original data with added cluster metadata (`Cluster ID`, `Cluster Label`, `Cluster Size`). Built with Rust, eframe/egui (wgpu renderer). No data leaves the workstation.

Target workload: 200k records on 4-CPU / 16 GiB VDI, end-to-end under 15 minutes.

## Build & Test Commands

```bash
cargo build                          # debug build
cargo build --release                # release build (single executable)
cargo test                           # run all unit tests
cargo test text::tests               # run tests in a specific module
cargo test --lib test_name           # run a single test by name
cargo clippy                         # lint
RUST_LOG=debug cargo run             # run with debug logging (default is info)
```

Logging is controlled via `RUST_LOG` env var (tracing-subscriber with env-filter).

## Architecture

The crate is both a library (`lib.rs` re-exports all modules) and a binary (`main.rs` launches the eframe app).

### Module responsibilities

- **`model`** - All shared domain types: `SourceTable`, `IncidentRecord`, `Cluster`, `Subgroup`, `ColumnMapping`, `RunSettings`, `AnalysisRun`, etc. Central to every other module.
- **`app`** - eframe/egui GUI state machine. Screens: Import -> Mapping -> Run -> Results. Owns all UI state and delegates to other modules. Never runs heavy work on the UI thread.
- **`worker`** - Spawns analysis on a background thread, sends `WorkerMessage` (Started/Progress/Finished) over `mpsc` channels back to the GUI. `run_analysis` is the synchronous entry point for tests.
- **`io`** - CSV/XLSX import (`calamine`), worksheet listing, and XLSX export (`rust_xlsxwriter`). Preserves all original source columns on export.
- **`schema`** - Column mapping logic: `suggest_mapping` auto-detects common ServiceNow column names, `validate_mapping` checks mandatory fields, `build_records` converts source rows into `IncidentRecord`s and `IgnoredRow`s.
- **`text`** - Text normalization pipeline (case, whitespace, punctuation, digits, ticket noise), multilingual stopwords (EN/DE/FR/IT), n-gram generation, sparse TF-IDF feature extraction.
- **`clustering`** - Sparse vector similarity, inverted-index candidate generation (avoids O(n^2)), connected-component clustering, subgroup generation with tighter thresholds. Parallelized with rayon.
- **`labels`** - Generates sentence-style cluster/subgroup summaries from high-contrast TF-IDF terms.
- **`session`** - JSON-based save/load for mapping profiles and full analysis sessions.

### Data flow

1. `io::import_source` / `io::import_xlsx_sheet` -> `SourceTable`
2. `schema::suggest_mapping` -> `ColumnMapping` (user adjusts in UI)
3. `schema::build_records` -> `Vec<IncidentRecord>` + `Vec<IgnoredRow>`
4. `text::extract_features` -> `Vec<TextFeatures>` (sparse TF-IDF vectors)
5. `clustering::cluster_incidents` -> `Vec<Cluster>` (with subgroups and labels)
6. `io::export_analysis` -> XLSX with original columns + cluster metadata

### Key constraints

- `ClusterId(0)` is reserved for `UNCLUSTERED` - real cluster IDs start at 1.
- Default similarity threshold is 42%, subgroup threshold 58%, minimum cluster size 50.
- GUI rendering is pinned: egui 0.32.0, wgpu 25.0.2, eframe 0.32.0 with wgpu feature.
- Rayon thread pool is configured globally in `main.rs` based on available parallelism.
- Filters (assignment group, service, category, config item, date range) apply to the result view, not to the clustering computation.

## Design Documents

- `architecture.md` - Full technical specification
- `requirements.md` - Detailed requirements with MoSCoW priorities
- `decision-log.md` - Architectural decisions and rationale
- `delivery-plan.md` - Phased delivery plan
- `open-questions.md` - Unresolved design questions
