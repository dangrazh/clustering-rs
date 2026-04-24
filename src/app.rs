use crate::io::{export_analysis, import_source, import_xlsx_sheet, list_worksheets};
use crate::model::{AnalysisRun, ColumnMapping, RunSettings, SourceTable};
use crate::progress::ProgressUpdate;
use crate::schema::{suggest_mapping, validate_mapping};
use crate::session::{
    load_analysis_session, load_mapping_profile, save_analysis_session, save_mapping_profile,
};
use crate::worker::{spawn_analysis, WorkerMessage};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Import,
    Mapping,
    Run,
    Results,
}

pub struct IncidentClusteringApp {
    screen: Screen,
    source: Option<SourceTable>,
    mapping: ColumnMapping,
    settings: RunSettings,
    analysis: Option<AnalysisRun>,
    worker: Option<Receiver<WorkerMessage>>,
    current_progress: Option<ProgressUpdate>,
    progress_log: Vec<ProgressLogEntry>,
    run_started_at: Option<Instant>,
    last_run_elapsed: Option<Duration>,
    results_tree_width: f32,
    status: String,
    worksheets: Vec<String>,
    selected_worksheet: Option<String>,
}

#[derive(Debug, Clone)]
struct ProgressLogEntry {
    elapsed: Duration,
    update: ProgressUpdate,
}

impl Default for IncidentClusteringApp {
    fn default() -> Self {
        Self {
            screen: Screen::Import,
            source: None,
            mapping: ColumnMapping::default(),
            settings: RunSettings::default(),
            analysis: None,
            worker: None,
            current_progress: None,
            progress_log: Vec::new(),
            run_started_at: None,
            last_run_elapsed: None,
            results_tree_width: 1_400.0,
            status: "Select a CSV or Excel incident export.".to_owned(),
            worksheets: Vec::new(),
            selected_worksheet: None,
        }
    }
}

impl eframe::App for IncidentClusteringApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker(ctx);

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("Incident Clustering Analyzer");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(source) = &self.source {
                        ui.label(format!("{} rows", source.row_count()));
                    }
                });
            });
            ui.add_space(4.0);
            self.workflow_header(ui);
            ui.add_space(4.0);
        });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(&self.status);
                if let Some(started_at) = self.run_started_at {
                    ui.separator();
                    ui.label(format!(
                        "Elapsed: {}",
                        format_duration(started_at.elapsed())
                    ));
                } else if let Some(elapsed) = self.last_run_elapsed {
                    ui.separator();
                    ui.label(format!("Last run: {}", format_duration(elapsed)));
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.screen {
            Screen::Import => self.import_screen(ui),
            Screen::Mapping => self.mapping_screen(ui),
            Screen::Run => self.run_screen(ui),
            Screen::Results => self.results_screen(ui),
        });
    }
}

impl IncidentClusteringApp {
    fn workflow_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            self.workflow_button(ui, Screen::Import, "1", "Source");
            self.workflow_separator(ui);
            self.workflow_button(ui, Screen::Mapping, "2", "Mapping");
            self.workflow_separator(ui);
            self.workflow_button(ui, Screen::Run, "3", "Analysis");
            self.workflow_separator(ui);
            self.workflow_button(ui, Screen::Results, "4", "Results");
        });
    }

    fn workflow_separator(&self, ui: &mut egui::Ui) {
        ui.label(">");
    }

    fn workflow_button(&mut self, ui: &mut egui::Ui, screen: Screen, number: &str, label: &str) {
        let enabled = match screen {
            Screen::Import => true,
            Screen::Mapping | Screen::Run => self.source.is_some(),
            Screen::Results => self.analysis.is_some(),
        };

        let state = self.step_state(screen);
        let text = format!("{number}  {label}  {state}");
        if ui
            .add_enabled(
                enabled,
                egui::Button::new(text)
                    .selected(self.screen == screen)
                    .min_size(egui::vec2(150.0, 32.0)),
            )
            .clicked()
        {
            self.screen = screen;
        }
    }

    fn step_state(&self, screen: Screen) -> &'static str {
        match screen {
            Screen::Import if self.source.is_some() => "done",
            Screen::Mapping if self.mapping_ready() => "done",
            Screen::Run if self.worker.is_some() => "running",
            Screen::Run if self.analysis.is_some() => "done",
            Screen::Results if self.analysis.is_some() => "ready",
            _ => "pending",
        }
    }

    fn import_screen(&mut self, ui: &mut egui::Ui) {
        screen_heading(ui, "1. Source file");

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add_sized([160.0, 32.0], egui::Button::new("Open CSV/XLSX"))
                    .clicked()
                {
                    self.open_source_file();
                }
                if ui
                    .add_sized([140.0, 32.0], egui::Button::new("Load Session"))
                    .clicked()
                {
                    self.load_session();
                }
            });
        });

        if let Some(source) = &self.source {
            ui.add_space(8.0);
            summary_row(
                ui,
                &[
                    ("Rows", source.row_count().to_string()),
                    ("Columns", source.headers.len().to_string()),
                    (
                        "File",
                        source
                            .source_path
                            .as_ref()
                            .and_then(|path| path.file_name())
                            .and_then(|name| name.to_str())
                            .unwrap_or("Loaded session")
                            .to_owned(),
                    ),
                ],
            );
        }

        if !self.worksheets.is_empty() {
            ui.add_space(8.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Worksheet");
                    let current = self
                        .selected_worksheet
                        .clone()
                        .unwrap_or_else(|| "Select worksheet".to_owned());
                    egui::ComboBox::from_id_salt("worksheet_select")
                        .width(280.0)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for sheet in self.worksheets.clone() {
                                if ui
                                    .selectable_label(
                                        self.selected_worksheet.as_ref() == Some(&sheet),
                                        &sheet,
                                    )
                                    .clicked()
                                {
                                    self.selected_worksheet = Some(sheet.clone());
                                    self.open_selected_worksheet();
                                }
                            }
                        });
                });
            });
        }

        ui.add_space(8.0);
        self.source_preview(ui);

        if self.source.is_some() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_sized([180.0, 34.0], egui::Button::new("Confirm Source"))
                    .clicked()
                {
                    self.screen = Screen::Mapping;
                }
            });
        }
    }

    fn mapping_screen(&mut self, ui: &mut egui::Ui) {
        screen_heading(ui, "2. Field mapping");
        let Some(source) = &self.source else {
            ui.label("No source file loaded.");
            return;
        };
        let headers = source.headers.clone();
        let source_for_validation = source.clone();

        summary_row(
            ui,
            &[
                ("Required", self.required_mapping_status()),
                (
                    "Additional text",
                    self.mapping.additional_text.len().to_string(),
                ),
                ("Filters", self.filter_mapping_count().to_string()),
            ],
        );

        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong("Required fields");
            ui.add_space(4.0);
            egui::Grid::new("required_mapping_grid")
                .num_columns(2)
                .spacing([24.0, 8.0])
                .show(ui, |ui| {
                    column_combo_row(
                        ui,
                        "Incident number",
                        &headers,
                        &mut self.mapping.incident_number,
                    );
                    column_combo_row(
                        ui,
                        "Short description",
                        &headers,
                        &mut self.mapping.short_description,
                    );
                });
        });

        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong("Filter and context fields");
            ui.add_space(4.0);
            egui::Grid::new("filter_mapping_grid")
                .num_columns(2)
                .spacing([24.0, 8.0])
                .show(ui, |ui| {
                    column_combo_row(
                        ui,
                        "Assignment group",
                        &headers,
                        &mut self.mapping.assignment_group,
                    );
                    column_combo_row(ui, "Service", &headers, &mut self.mapping.service);
                    column_combo_row(ui, "Category", &headers, &mut self.mapping.category);
                    column_combo_row(
                        ui,
                        "Configuration item",
                        &headers,
                        &mut self.mapping.configuration_item,
                    );
                    column_combo_row(ui, "Date", &headers, &mut self.mapping.date);
                });
        });

        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong("Additional text for similarity");
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    for (index, header) in headers.iter().enumerate() {
                        let mut selected = self.mapping.additional_text.contains(&index);
                        if ui.checkbox(&mut selected, header).changed() {
                            if selected {
                                self.mapping.additional_text.push(index);
                                self.mapping.additional_text.sort_unstable();
                                self.mapping.additional_text.dedup();
                            } else {
                                self.mapping.additional_text.retain(|value| *value != index);
                            }
                        }
                    }
                });
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Save Mapping").clicked() {
                self.save_mapping();
            }
            if ui.button("Load Mapping").clicked() {
                self.load_mapping();
            }
            ui.separator();
            if ui
                .add_enabled(
                    self.mapping_ready(),
                    egui::Button::new("Confirm Mapping").min_size(egui::vec2(170.0, 34.0)),
                )
                .clicked()
            {
                match validate_mapping(&self.mapping, &source_for_validation) {
                    Ok(()) => self.screen = Screen::Run,
                    Err(err) => self.status = err.to_string(),
                }
            }
        });
    }

    fn run_screen(&mut self, ui: &mut egui::Ui) {
        screen_heading(ui, "3. Run analysis");
        if self.worker.is_some() {
            self.progress_view(ui);
            return;
        }

        if let Some(source) = &self.source {
            summary_row(
                ui,
                &[
                    ("Source rows", source.row_count().to_string()),
                    ("Mapped fields", self.mapped_field_count().to_string()),
                    (
                        "Minimum cluster size",
                        self.settings.minimum_cluster_size.to_string(),
                    ),
                ],
            );
        }

        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong("Analysis settings");
            ui.add_space(4.0);
            ui.add(
                egui::Slider::new(&mut self.settings.minimum_cluster_size, 2..=1000)
                    .text("Minimum useful cluster size"),
            );
            ui.add(
                egui::Slider::new(&mut self.settings.similarity_threshold_percent, 10..=90)
                    .text("Cluster similarity threshold"),
            );
            ui.add(
                egui::Slider::new(
                    &mut self.settings.subgroup_similarity_threshold_percent,
                    10..=95,
                )
                .text("Subgroup similarity threshold"),
            );
        });

        ui.add_space(8.0);
        if ui
            .add_sized(
                [180.0, 36.0],
                egui::Button::new("Start Analysis").selected(false),
            )
            .clicked()
        {
            self.start_analysis();
        }
    }

    fn results_screen(&mut self, ui: &mut egui::Ui) {
        screen_heading(ui, "4. Explore results");
        let Some(analysis) = &self.analysis else {
            ui.label("No analysis results available.");
            return;
        };
        let processed_count = analysis.processed_incidents.len();
        let ignored_count = analysis.ignored_rows.len();
        let cluster_count = analysis.clusters.len();
        let unclustered_count = analysis.unclustered_row_indices.len();

        summary_row(
            ui,
            &[
                ("Processed", processed_count.to_string()),
                ("Ignored", ignored_count.to_string()),
                ("Clusters", cluster_count.to_string()),
                ("Unclustered", unclustered_count.to_string()),
            ],
        );

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_sized([130.0, 32.0], egui::Button::new("Export Excel"))
                .clicked()
            {
                self.export_results();
            }
            if ui
                .add_sized([130.0, 32.0], egui::Button::new("Save Session"))
                .clicked()
            {
                self.save_session();
            }
        });

        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Tree width");
                ui.add(
                    egui::Slider::new(&mut self.results_tree_width, 800.0..=4_000.0)
                        .suffix(" px")
                        .clamping(egui::SliderClamping::Always),
                );
                if ui.button("Reset").clicked() {
                    self.results_tree_width = 1_400.0;
                }
            });
        });

        ui.add_space(8.0);
        let Some(analysis) = &self.analysis else {
            return;
        };
        let tree_width = ui.available_width().max(self.results_tree_width);
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(tree_width);
                for cluster in &analysis.clusters {
                    egui::CollapsingHeader::new(format!(
                        "{} - {} ({})",
                        cluster.id,
                        cluster.label,
                        cluster.size()
                    ))
                    .default_open(false)
                    .show(ui, |ui| {
                        for subgroup in &cluster.subgroups {
                            egui::CollapsingHeader::new(format!(
                                "Theme {} - {} ({})",
                                subgroup.id,
                                subgroup.label,
                                subgroup.size()
                            ))
                            .show(ui, |ui| {
                                for row_index in subgroup.incident_row_indices.iter().take(250) {
                                    if let Some(record) = analysis
                                        .processed_incidents
                                        .iter()
                                        .find(|record| record.source_row_index == *row_index)
                                    {
                                        ui.label(format!(
                                            "{}: {}",
                                            record.incident_number, record.analysis_text
                                        ));
                                    }
                                }
                            });
                        }
                    });
                }
            });
    }

    fn source_preview(&self, ui: &mut egui::Ui) {
        let Some(source) = &self.source else {
            return;
        };

        ui.separator();
        ui.label(format!(
            "{} columns, {} rows",
            source.headers.len(),
            source.row_count()
        ));
        egui::ScrollArea::both().max_height(360.0).show(ui, |ui| {
            egui::Grid::new("source_preview_grid")
                .striped(true)
                .show(ui, |ui| {
                    for header in &source.headers {
                        ui.strong(header);
                    }
                    ui.end_row();

                    for row in source.rows.iter().take(25) {
                        for column in 0..source.headers.len() {
                            ui.label(row.get(column).map(String::as_str).unwrap_or_default());
                        }
                        ui.end_row();
                    }
                });
        });
    }

    fn open_source_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Incident exports", &["csv", "xlsx", "xlsm", "xls"])
            .pick_file()
        else {
            return;
        };

        self.worksheets.clear();
        self.selected_worksheet = None;

        if is_excel(&path) {
            match list_worksheets(&path) {
                Ok(worksheets) => {
                    self.worksheets = worksheets;
                    self.selected_worksheet = self.worksheets.first().cloned();
                    if let Some(sheet) = self.selected_worksheet.clone() {
                        match import_xlsx_sheet(&path, &sheet) {
                            Ok(source) => self.accept_source(source),
                            Err(err) => self.status = err.to_string(),
                        }
                    }
                }
                Err(err) => self.status = err.to_string(),
            }
        } else {
            match import_source(&path) {
                Ok(source) => self.accept_source(source),
                Err(err) => self.status = err.to_string(),
            }
        }
    }

    fn open_selected_worksheet(&mut self) {
        let Some(source_path) = self
            .source
            .as_ref()
            .and_then(|source| source.source_path.clone())
            .or_else(|| self.last_excel_path())
        else {
            return;
        };
        let Some(sheet) = self.selected_worksheet.clone() else {
            return;
        };

        match import_xlsx_sheet(&source_path, &sheet) {
            Ok(source) => self.accept_source(source),
            Err(err) => self.status = err.to_string(),
        }
    }

    fn last_excel_path(&self) -> Option<PathBuf> {
        self.source
            .as_ref()
            .and_then(|source| source.source_path.clone())
    }

    fn accept_source(&mut self, source: SourceTable) {
        self.mapping = suggest_mapping(&source.headers);
        self.status = format!(
            "Loaded {} rows from {}.",
            source.row_count(),
            source
                .source_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "source".to_owned())
        );
        self.source = Some(source);
        self.screen = Screen::Mapping;
    }

    fn start_analysis(&mut self) {
        let Some(source) = self.source.clone() else {
            self.status = "Load a source file first.".to_owned();
            return;
        };
        if let Err(err) = validate_mapping(&self.mapping, &source) {
            self.status = err.to_string();
            return;
        }
        self.worker = Some(spawn_analysis(
            source,
            self.mapping.clone(),
            self.settings.clone(),
        ));
        self.current_progress = None;
        self.progress_log.clear();
        self.run_started_at = Some(Instant::now());
        self.last_run_elapsed = None;
        self.status = "Started clustering analysis.".to_owned();
    }

    fn poll_worker(&mut self, ctx: &egui::Context) {
        let Some(worker) = self.worker.take() else {
            return;
        };

        let mut keep_worker = true;
        while let Ok(message) = worker.try_recv() {
            match message {
                WorkerMessage::Started => {
                    self.status = "Analysis worker started.".to_owned();
                }
                WorkerMessage::Progress(progress) => {
                    self.status = format!("{}: {}", progress.stage, progress.detail);
                    self.current_progress = Some(progress.clone());
                    self.progress_log.push(ProgressLogEntry {
                        elapsed: self
                            .run_started_at
                            .map(|started_at| started_at.elapsed())
                            .unwrap_or_default(),
                        update: progress,
                    });
                    if self.progress_log.len() > 40 {
                        self.progress_log.remove(0);
                    }
                }
                WorkerMessage::Finished(Ok(run)) => {
                    let elapsed = self.finish_run_timer();
                    self.status = format!(
                        "Analysis complete in {}: {} clusters, {} ignored rows.",
                        format_duration(elapsed),
                        run.clusters.len(),
                        run.ignored_rows.len()
                    );
                    self.analysis = Some(*run);
                    self.current_progress = None;
                    self.screen = Screen::Results;
                    keep_worker = false;
                }
                WorkerMessage::Finished(Err(err)) => {
                    let elapsed = self.finish_run_timer();
                    self.status = format!(
                        "Analysis failed after {}: {}",
                        format_duration(elapsed),
                        err
                    );
                    self.current_progress = None;
                    keep_worker = false;
                }
            }
        }

        if keep_worker {
            self.worker = Some(worker);
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn export_results(&mut self) {
        let Some(analysis) = &self.analysis else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Excel workbook", &["xlsx"])
            .set_file_name("clustered_incidents.xlsx")
            .save_file()
        else {
            return;
        };

        match export_analysis(analysis, &path) {
            Ok(()) => self.status = format!("Exported {}", path.display()),
            Err(err) => self.status = err.to_string(),
        }
    }

    fn save_mapping(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Mapping profile", &["json"])
            .set_file_name("incident_mapping.json")
            .save_file()
        else {
            return;
        };
        match save_mapping_profile(&path, &self.mapping) {
            Ok(()) => self.status = format!("Saved mapping {}", path.display()),
            Err(err) => self.status = err.to_string(),
        }
    }

    fn load_mapping(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Mapping profile", &["json"])
            .pick_file()
        else {
            return;
        };
        match load_mapping_profile(&path) {
            Ok(mapping) => {
                self.mapping = mapping;
                self.status = format!("Loaded mapping {}", path.display());
            }
            Err(err) => self.status = err.to_string(),
        }
    }

    fn save_session(&mut self) {
        let Some(analysis) = &self.analysis else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Analysis session", &["json"])
            .set_file_name("incident_analysis_session.json")
            .save_file()
        else {
            return;
        };
        match save_analysis_session(&path, analysis) {
            Ok(()) => self.status = format!("Saved session {}", path.display()),
            Err(err) => self.status = err.to_string(),
        }
    }

    fn load_session(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Analysis session", &["json"])
            .pick_file()
        else {
            return;
        };
        match load_analysis_session(&path) {
            Ok(run) => {
                self.source = Some(run.source.clone());
                self.mapping = run.mapping.clone();
                self.settings = run.settings.clone();
                self.analysis = Some(run);
                self.screen = Screen::Results;
                self.status = format!("Loaded session {}", path.display());
            }
            Err(err) => self.status = err.to_string(),
        }
    }

    fn progress_view(&self, ui: &mut egui::Ui) {
        let Some(progress) = &self.current_progress else {
            ui.label("Starting analysis worker.");
            return;
        };

        let elapsed = self
            .run_started_at
            .map(|started_at| started_at.elapsed())
            .unwrap_or_default();

        summary_row(
            ui,
            &[
                ("Current stage", progress.stage.clone()),
                (
                    "Main step",
                    format!("{} of {}", progress.step, progress.total_steps),
                ),
                (
                    "Sub-step",
                    progress
                        .substep
                        .as_ref()
                        .map(|substep| format!("{} of {}", substep.current, substep.total))
                        .unwrap_or_else(|| "-".to_owned()),
                ),
                ("Elapsed", format_duration(elapsed)),
            ],
        );

        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.heading(&progress.stage);
            });
            if let Some(substep) = &progress.substep {
                ui.label(format!(
                    "Sub-step {} of {}: {}",
                    substep.current, substep.total, substep.label
                ));
            }
            ui.label(&progress.detail);
            ui.add_space(4.0);
            ui.add(
                egui::ProgressBar::new(progress.fraction())
                    .desired_width(ui.available_width())
                    .text(format!("{}%", (progress.fraction() * 100.0).round() as u8)),
            );
        });

        if !progress.workers.is_empty() {
            ui.add_space(8.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.strong("Parallel workers");
                ui.add_space(4.0);
                egui::Grid::new("parallel_worker_grid")
                    .num_columns(4)
                    .striped(true)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        ui.strong("Worker");
                        ui.strong("Processed");
                        ui.strong("Share");
                        ui.strong("Bar");
                        ui.end_row();
                        for worker in &progress.workers {
                            ui.label(format!("#{}", worker.worker));
                            ui.label(format!("{}/{}", worker.completed, worker.total));
                            ui.label(format!("{:.0}%", worker.fraction() * 100.0));
                            ui.add(
                                egui::ProgressBar::new(worker.fraction())
                                    .desired_width(180.0)
                                    .show_percentage(),
                            );
                            ui.end_row();
                        }
                    });
            });
        }

        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong("Pipeline activity");
            ui.add_space(4.0);
            egui::Grid::new("pipeline_activity_grid")
                .num_columns(5)
                .striped(true)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.strong("Time");
                    ui.strong("Main");
                    ui.strong("Sub");
                    ui.strong("Stage");
                    ui.strong("Detail");
                    ui.end_row();
                    for entry in self.progress_log.iter().rev() {
                        ui.label(format_duration(entry.elapsed));
                        ui.label(format!(
                            "{}/{}",
                            entry.update.step, entry.update.total_steps
                        ));
                        ui.label(
                            entry.update
                                .substep
                                .as_ref()
                                .map(|substep| format!("{}/{}", substep.current, substep.total))
                                .unwrap_or_else(|| "-".to_owned()),
                        );
                        ui.label(&entry.update.stage);
                        ui.label(&entry.update.detail);
                        ui.end_row();
                    }
                });
        });
    }

    fn finish_run_timer(&mut self) -> Duration {
        let elapsed = self
            .run_started_at
            .take()
            .map(|started_at| started_at.elapsed())
            .or(self.last_run_elapsed)
            .unwrap_or_default();
        self.last_run_elapsed = Some(elapsed);
        elapsed
    }

    fn mapping_ready(&self) -> bool {
        self.mapping.incident_number.is_some() && self.mapping.short_description.is_some()
    }

    fn required_mapping_status(&self) -> String {
        if self.mapping_ready() {
            "2 of 2".to_owned()
        } else {
            let mapped = [self.mapping.incident_number, self.mapping.short_description]
                .into_iter()
                .flatten()
                .count();
            format!("{mapped} of 2")
        }
    }

    fn filter_mapping_count(&self) -> usize {
        [
            self.mapping.assignment_group,
            self.mapping.service,
            self.mapping.category,
            self.mapping.configuration_item,
            self.mapping.date,
        ]
        .into_iter()
        .flatten()
        .count()
    }

    fn mapped_field_count(&self) -> usize {
        self.filter_mapping_count()
            + self.mapping.additional_text.len()
            + [self.mapping.incident_number, self.mapping.short_description]
                .into_iter()
                .flatten()
                .count()
    }
}

fn column_combo_row(
    ui: &mut egui::Ui,
    label: &str,
    headers: &[String],
    selected: &mut Option<usize>,
) {
    ui.label(label);
    let selected_text = selected
        .and_then(|index| headers.get(index))
        .map(String::as_str)
        .unwrap_or("Not mapped");
    egui::ComboBox::from_id_salt(label)
        .width(320.0)
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            ui.selectable_value(selected, None, "Not mapped");
            for (index, header) in headers.iter().enumerate() {
                ui.selectable_value(selected, Some(index), header);
            }
        });
    ui.end_row();
}

fn screen_heading(ui: &mut egui::Ui, title: &str) {
    ui.heading(title);
    ui.add_space(8.0);
}

fn summary_row(ui: &mut egui::Ui, items: &[(&str, String)]) {
    ui.horizontal_wrapped(|ui| {
        for (label, value) in items {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(130.0);
                ui.label(
                    egui::RichText::new(*label)
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                ui.strong(value);
            });
        }
    });
}

fn is_excel(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("xlsx" | "xlsm" | "xls")
    )
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let millis = duration.subsec_millis();

    if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else if seconds > 0 {
        format!("{seconds}.{millis:03}s")
    } else {
        format!("{millis}ms")
    }
}
