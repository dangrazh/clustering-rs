# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Incident Clustering Analyzer — a local/offline Rust desktop application that imports incident exports (CSV/XLSX), clusters similar records by text similarity, and exports results with cluster metadata. Targets Windows 11 on Azure VDI (4 CPUs, 16 GiB RAM) processing up to 200,000 records in under 15 minutes. Delivered as a single executable.

## Build & Test Commands

```
cargo build                          # debug build
cargo build --release                # release build (single exe)
cargo test                           # all tests
cargo test <test_name>               # single test
cargo clippy -- -D warnings          # lint
cargo check                          # type-check only
```

The release binary hides the console window on Windows (`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`).

## Architecture

### UI Framework

Uses **GPUI** (`gpui` + `gpui-component` crates) — *not* egui/eframe despite what the design docs say. The architecture docs reference egui but the actual implementation switched to GPUI. The app runs a synchronous GPUI render loop; long-running work is offloaded to a background thread via `std::sync::mpsc` channels.

### Module Map

- **`app`** — GPUI application state machine. Contains `IncidentClusteringApp` (the `Render` impl), all screen rendering (Import → Mapping → Run → Results), light/dark `Palette`, result tree, detail table, filter bar. One large file with a private `gpui_app` module re-exported publicly.
- **`worker`** — Spawns the analysis pipeline on a background thread. Sends `WorkerMessage` (Started/Progress/Finished) over `mpsc::channel`. The GUI polls via `poll_worker` called each frame during `render()`.
- **`schema`** — Column mapping logic: `suggest_mapping` auto-detects common ServiceNow column names, `validate_mapping` checks required fields, `build_records` converts `SourceTable` rows into `IncidentRecord`s and `IgnoredRow`s. Parallel via rayon.
- **`text`** — Text normalization, tokenization, stopword filtering (EN/DE/FR/IT), word bigrams, character 4-grams, sparse TF-IDF feature extraction. All deterministic and offline.
- **`clustering`** — Inverted-index candidate generation → Jaccard similarity → union-find connected components → minimum-size promotion → subgroup re-clustering within primary clusters. Parallelized with rayon. Key constants: `MAX_TERM_DOC_FREQUENCY` (2000), `MAX_SUBGROUP_RECLUSTER_SIZE` (5000).
- **`labels`** — Generates sentence-style cluster summaries from top representative keywords.
- **`model`** — All shared domain types: `SourceTable`, `ColumnMapping`, `IncidentRecord`, `Cluster`, `Subgroup`, `AnalysisRun`, `RunSettings`, `TextFeatures`, etc.
- **`io`** — CSV import (via `csv`), XLSX import (via `calamine`), Excel export (via `rust_xlsxwriter`). Export adds Cluster ID/Label/Size and Theme ID/Label/Size columns.
- **`session`** — JSON-based save/load for mapping profiles and full analysis sessions (which embed all source data).
- **`progress`** — `ProgressReporter` and `ParallelProgressTracker` for reporting multi-step, multi-worker progress from rayon threads back to the GUI.

### Data Flow

```
File → import (io) → SourceTable → build_records (schema) → IncidentRecord[]
  → extract_features (text) → TextFeatures[] → cluster_incidents (clustering)
  → Cluster[] → summarize (labels) → AnalysisRun → GUI / export (io)
```

### Key Design Decisions

- **Similarity**: Jaccard on sparse TF-IDF vectors with inverted-index candidate generation (avoids O(n²)). Thresholds are user-configurable in the Run screen (`similarity_threshold_percent` default 42, `subgroup_similarity_threshold_percent` default 58).
- **Clustering**: Union-find on similarity graph edges, then connected components promoted above minimum cluster size (default 50). No user-specified cluster count.
- **Filters**: Applied post-hoc to the result view, not to clustering itself. Stored as `BTreeMap<column_index, BTreeSet<selected_values>>`.
- **Privacy**: No network calls, no telemetry, no logging of raw incident text by default.

## Coding Conventions

- Idiomatic Rust: iterators/combinators over index loops, `Result`/`Option` over sentinel values, `thiserror` for custom errors.
- New crate dependencies require explicit user approval before adding to `Cargo.toml`.
- The `app.rs` file wraps all GUI code in a private `mod gpui_app` and re-exports `IncidentClusteringApp`.
- Rayon is used for CPU-heavy parallel work; the global thread pool is configured to use all logical cores.
- All export column names are stable: `Cluster ID`, `Cluster Label`, `Cluster Size`, `Theme ID`, `Theme Label`, `Theme Size`.
