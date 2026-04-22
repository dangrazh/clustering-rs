use crate::io::{export_analysis, import_source, import_xlsx_sheet, list_worksheets};
use crate::model::{AnalysisRun, ColumnMapping, RunSettings, SourceTable};
use crate::schema::{suggest_mapping, validate_mapping};
use crate::session::{
    load_analysis_session, load_mapping_profile, save_analysis_session, save_mapping_profile,
};
use crate::worker::{spawn_analysis, ProgressUpdate, WorkerMessage};
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
    progress_log: Vec<ProgressUpdate>,
    run_started_at: Option<Instant>,
    last_run_elapsed: Option<Duration>,
    results_tree_width: f32,
    status: String,
    worksheets: Vec<String>,
    selected_worksheet: Option<String>,
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
            ui.horizontal(|ui| {
                ui.heading("Incident Clustering Analyzer");
                ui.separator();
                self.nav_button(ui, Screen::Import, "Import");
                self.nav_button(ui, Screen::Mapping, "Mapping");
                self.nav_button(ui, Screen::Run, "Run");
                self.nav_button(ui, Screen::Results, "Results");
            });
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
    fn nav_button(&mut self, ui: &mut egui::Ui, screen: Screen, label: &str) {
        let enabled = match screen {
            Screen::Import => true,
            Screen::Mapping | Screen::Run => self.source.is_some(),
            Screen::Results => self.analysis.is_some(),
        };
        if ui
            .add_enabled(
                enabled,
                egui::Button::new(label).selected(self.screen == screen),
            )
            .clicked()
        {
            self.screen = screen;
        }
    }

    fn import_screen(&mut self, ui: &mut egui::Ui) {
        ui.heading("Import");
        ui.horizontal(|ui| {
            if ui.button("Open CSV/XLSX").clicked() {
                self.open_source_file();
            }
            if ui.button("Load Session").clicked() {
                self.load_session();
            }
        });

        if !self.worksheets.is_empty() {
            ui.separator();
            ui.label("Workbook worksheet");
            let current = self
                .selected_worksheet
                .clone()
                .unwrap_or_else(|| "Select worksheet".to_owned());
            egui::ComboBox::from_id_salt("worksheet_select")
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
        }

        self.source_preview(ui);
    }

    fn mapping_screen(&mut self, ui: &mut egui::Ui) {
        ui.heading("Column Mapping");
        let Some(source) = &self.source else {
            ui.label("No source file loaded.");
            return;
        };
        let headers = source.headers.clone();
        let source_for_validation = source.clone();

        column_combo(
            ui,
            "Incident number",
            &headers,
            &mut self.mapping.incident_number,
        );
        column_combo(
            ui,
            "Short description",
            &headers,
            &mut self.mapping.short_description,
        );
        column_combo(
            ui,
            "Assignment group",
            &headers,
            &mut self.mapping.assignment_group,
        );
        column_combo(ui, "Service", &headers, &mut self.mapping.service);
        column_combo(ui, "Category", &headers, &mut self.mapping.category);
        column_combo(
            ui,
            "Configuration item",
            &headers,
            &mut self.mapping.configuration_item,
        );
        column_combo(ui, "Date", &headers, &mut self.mapping.date);

        ui.separator();
        ui.label("Additional text columns");
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

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Save Mapping").clicked() {
                self.save_mapping();
            }
            if ui.button("Load Mapping").clicked() {
                self.load_mapping();
            }
            if ui.button("Continue").clicked() {
                match validate_mapping(&self.mapping, &source_for_validation) {
                    Ok(()) => self.screen = Screen::Run,
                    Err(err) => self.status = err.to_string(),
                }
            }
        });
    }

    fn run_screen(&mut self, ui: &mut egui::Ui) {
        ui.heading("Run Analysis");
        if self.worker.is_some() {
            ui.spinner();
            self.progress_view(ui);
            return;
        }

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

        if ui.button("Start Clustering").clicked() {
            self.start_analysis();
        }
    }

    fn results_screen(&mut self, ui: &mut egui::Ui) {
        ui.heading("Results");
        let Some(analysis) = &self.analysis else {
            ui.label("No analysis results available.");
            return;
        };
        let processed_count = analysis.processed_incidents.len();
        let ignored_count = analysis.ignored_rows.len();
        let cluster_count = analysis.clusters.len();
        let unclustered_count = analysis.unclustered_row_indices.len();

        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{processed_count} processed incidents"));
            ui.label(format!("{ignored_count} ignored rows"));
            ui.label(format!("{cluster_count} clusters"));
            ui.label(format!("{unclustered_count} unclustered incidents"));
        });

        ui.horizontal(|ui| {
            if ui.button("Export Excel").clicked() {
                self.export_results();
            }
            if ui.button("Save Session").clicked() {
                self.save_session();
            }
        });

        ui.separator();
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

        ui.separator();
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
                    self.progress_log.push(progress);
                    if self.progress_log.len() > 12 {
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

        ui.separator();
        ui.heading(&progress.stage);
        ui.label(&progress.detail);
        ui.add(egui::ProgressBar::new(progress.fraction()).text(format!(
            "Step {} of {}",
            progress.step, progress.total_steps
        )));

        ui.separator();
        ui.label("Pipeline activity");
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                for entry in self.progress_log.iter().rev() {
                    ui.label(format!(
                        "{}/{} - {}: {}",
                        entry.step, entry.total_steps, entry.stage, entry.detail
                    ));
                }
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
}

fn column_combo(ui: &mut egui::Ui, label: &str, headers: &[String], selected: &mut Option<usize>) {
    ui.horizontal(|ui| {
        ui.label(label);
        let selected_text = selected
            .and_then(|index| headers.get(index))
            .map(String::as_str)
            .unwrap_or("Not mapped");
        egui::ComboBox::from_id_salt(label)
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                ui.selectable_value(selected, None, "Not mapped");
                for (index, header) in headers.iter().enumerate() {
                    ui.selectable_value(selected, Some(index), header);
                }
            });
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
