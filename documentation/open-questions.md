# Clarifications And Decisions

The original open questions have been answered inline by the Product Owner. This file is retained as the clarification record.

## Product Questions

1. Should ignored rows appear in the Excel export, and if yes, what cluster values should they receive?
   - Proposed default: preserve ignored source rows in export with blank cluster fields or `Ignored` in `Cluster ID`.
   - => ignored source rows can be excluded from export.
   - Decision: Ignored rows are excluded from export and remain visible only in the application ignored-row summary.
   
2. Should filters only narrow the displayed clustered results, or should users be able to rerun clustering on a filtered subset?
   - Proposed MVP default: filters narrow the displayed results only.
   - => Proposed default is fine.
   - Decision: MVP filters narrow the displayed clustered results only.
   
3. What does “analysis session or equivalent analysis state” need to include?
   - Proposed default: source file reference, mappings, settings, cluster assignments, labels, ignored-row summary, and enough display state to reload results.
   - => Proposed default is fine.
   - Decision: Full sessions include source file reference, mappings, settings, cluster assignments, labels, ignored-row summary, and enough display state to reload results.
   
4. Should saved analysis sessions embed the original incident data?
   - Proposed default: mapping profiles do not embed data; full sessions may embed data only when the user explicitly saves a session.
   - => saving analysis session should embed the original incident data.
   - Decision: Full analysis sessions embed the original incident data.
   
5. Are cluster labels allowed to be keyword phrases only, or do they need human-readable sentence-style summaries?
   - Proposed default: keyword phrases are sufficient for version 1.0.
   - => please make it human-readable sentenece-style summaries.
   - Decision: Cluster and subgroup labels are human-readable sentence-style summaries.
   
6. Should date filtering support multiple date formats and locales automatically, or should the user choose/confirm a date format?
   - Proposed default: infer common formats and show unparsed-date counts.
   - => Proposed default is fine.
   - Decision: Infer common date formats and show unparsed-date counts.

## Data And Quality Questions

1. Is there a representative anonymized export available for tuning and performance validation?
   - Owner: Problem Manager
   - Needed by: Phase 2
   - => yes, syntetic test data is available
   - Decision: Use available synthetic test data for early tuning and performance validation.
   
2. What are the most common source systems and column names in the exports?
   - Owner: Problem Manager
   - Needed by: Phase 1
   - => "INC Number";"INC Short Description";"Category";"Priority";"Assignment Group"
   - Decision: Use these as known common columns and suggest them automatically during mapping.
   
3. What languages are most common in practice, and are English descriptions common enough to include explicitly in acceptance testing?
   - Owner: Problem Manager
   - Needed by: Phase 2
   - => yes, English is by far the most common language
   - Decision: Optimize language handling for English-first data while retaining German, French, Italian, and mixed-language support.
   
4. How should duplicate incident numbers be displayed and exported?
   - Proposed default: process and export duplicates exactly as provided.
   - => Proposed default is fine.
   - Decision: Process and export duplicate incident numbers exactly as provided.

## Technical Questions

1. Is Rust the confirmed implementation language?
   - Proposed default: yes, based on repository naming and local performance needs.

   - => Yes, Rust is confirmed, however make sure to use the following explicit versions of egui / wgpu and eframe:
   
     egui = "0.32.0"
   
     wgpu = "25.0.2"
   
     eframe = {version = "0.32.0", features = ["wgpu"] }
   - Decision: Rust is confirmed with pinned `egui`, `wgpu`, and `eframe` versions.
   
2. Which operating system image is used on the Azure VDI?
   - Owner: Problem Manager or IT
   - Needed by: Phase 0
   - => Windows 11
   - Decision: Target OS is Windows 11.
   
3. Are there corporate restrictions on open-source crate licenses?
   - Owner: Problem Manager or IT
   - Needed by: Phase 0
   - => open-source crates with permissive licenses are fine.
   - Decision: Open-source crates with permissive licenses are allowed.
   
4. Can the application store local config/session files under the user profile, or must all outputs be explicitly user-selected?
   - Proposed default: mapping profiles under user profile; exports and full sessions user-selected.
   - => Proposed default is fine.
   - Decision: Store mapping profiles under the user profile; exports and full sessions use explicit user-selected paths.
   
5. Should the application be delivered as a single executable, installer, or portable folder?
   - Owner: Problem Manager or IT
   - Needed by: Phase 6
   - => single executable
   - Decision: Deliver as a single Windows executable.
