# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Incident Clustering Analyzer: a local/offline Windows desktop app that imports incident exports (CSV/XLSX), clusters similar records by text similarity, and exports the original data with added cluster metadata. Built with Rust, GPUI (from the Zed editor project). No data leaves the workstation.

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

The crate is both a library (`lib.rs` re-exports all modules) and a binary (`main.rs` launches the GPUI app).

### Module responsibilities

- **`model`** — All shared domain types: `SourceTable`, `IncidentRecord`, `Cluster`, `Subgroup`, `ColumnMapping`, `RunSettings`, `AnalysisRun`, `TextFeatures`, etc. Central to every other module.
- **`app`** — GPUI-based GUI state machine. Screens: Import → Mapping → Run → Results. Owns all UI state and delegates to other modules. Never runs heavy work on the UI thread. Polls the worker channel each frame via `request_animation_frame`. The entire implementation lives inside a private `gpui_app` module and re-exports `IncidentClusteringApp`.
- **`worker`** — Spawns analysis on a background `std::thread`, sends `WorkerMessage` (Started/Progress/Finished) over `mpsc` channels back to the GUI. `run_analysis` is the synchronous entry point for tests.
- **`progress`** — `ProgressReporter` and `ParallelProgressTracker` types that bridge the background pipeline to the UI. Tracks per-worker completion via atomics and emits throttled snapshots (one per percentage-point bucket).
- **`io`** — CSV/XLSX import (`calamine`), worksheet listing, and XLSX export (`rust_xlsxwriter`). Preserves all original source columns and appends 6 metadata columns on export: `Cluster ID`, `Cluster Label`, `Cluster Size`, `Theme ID`, `Theme Label`, `Theme Size`.
- **`schema`** — Column mapping logic: `suggest_mapping` auto-detects common ServiceNow column names, `validate_mapping` checks mandatory fields, `build_records` converts source rows into `IncidentRecord`s and `IgnoredRow`s (parallelized with rayon).
- **`text`** — Text normalization pipeline (lowercase, strip punctuation, collapse whitespace), multilingual stopwords (EN/DE/FR/IT), feature term generation (unigrams + bigrams + character 4-grams prefixed `char:`), and sparse TF-IDF feature extraction with parallel document-frequency counting.
- **`clustering`** — Jaccard similarity via inverted-index shared-count accumulation (avoids O(n²) pair enumeration), connected-component clustering with a `DisjointSet` (union-find), subgroup generation with a tighter threshold. Parallelized with rayon.
- **`labels`** — Generates sentence-style cluster/subgroup summaries from high-frequency terms. Uses TF-based keyword ranking (top-4 terms → templated sentence).
- **`session`** — JSON-based save/load for mapping profiles (`MappingProfile`) and full analysis sessions (`AnalysisSession`), versioned with a `version: u16` field.

### Data flow

1. `io::import_source` / `io::import_xlsx_sheet` → `SourceTable`
2. `schema::suggest_mapping` → `ColumnMapping` (user adjusts in UI)
3. `schema::build_records` → `Vec<IncidentRecord>` + `Vec<IgnoredRow>`
4. `text::extract_features` → `Vec<TextFeatures>` (sparse TF-IDF vectors)
5. `clustering::cluster_incidents` → `Vec<Cluster>` (with subgroups and labels)
6. `io::export_analysis` → XLSX with original columns + 6 cluster/theme metadata columns

### Key constants and thresholds

- `ClusterId(0)` is reserved for `UNCLUSTERED` — real cluster IDs start at 1.
- Default similarity threshold: 42%, subgroup threshold: 58%, minimum cluster size: 50.
- `MAX_TERM_DOC_FREQUENCY = 2_000` — terms appearing in more documents are excluded from candidate pair generation.
- `MAX_SUBGROUP_RECLUSTER_SIZE = 5_000` — clusters larger than this skip recursive re-clustering for subgroups.
- `EXCEL_CELL_CHAR_LIMIT = 32_767` — values exceeding this are truncated with `...` on export.
- Rayon thread pool is configured globally in `main.rs` based on `std::thread::available_parallelism`.

### UI framework notes

The GUI uses **GPUI** (`gpui = "0.2.2"`, `gpui_component = "0.5.1"`), not egui/eframe. Key patterns:
- Layout is built with `div()` builder chains (`.flex()`, `.flex_col()`, `.gap_3()`, etc.) — similar to Tailwind CSS.
- State changes call `cx.notify()` to trigger re-render.
- Event handlers use `cx.listener(...)` or `cx.weak_entity()` + `app.update(cx, ...)` for deferred updates from dropdown menus.
- Worker polling: `window.request_animation_frame()` keeps the render loop alive while analysis is running.
- Tables use `gpui_component::table::{Table, TableDelegate, TableState}` with a custom `GridTableDelegate`.
- Tree views use `gpui_component::tree::{tree, TreeItem, TreeState}`.
- The UI refers to subgroups as **"Themes"** throughout (tree items, detail titles).

### Terminology mapping

| Domain model | UI label | Export column |
|---|---|---|
| `Cluster` | Cluster | `Cluster ID`, `Cluster Label`, `Cluster Size` |
| `Subgroup` | Theme | `Theme ID`, `Theme Label`, `Theme Size` |

## Design Documents

Located in `documentation/`:
- `architecture.md` — Full technical specification
- `requirements.md` — Detailed requirements with MoSCoW priorities
- `decision-log.md` — Architectural decisions and rationale
- `delivery-plan.md` — Phased delivery plan
- `open-questions.md` — Unresolved design questions
