// mod gpui_app {
//     use crate::io::{export_analysis, import_source, import_xlsx_sheet, list_worksheets};
//     use crate::model::{AnalysisRun, ColumnMapping, RunSettings, SourceTable};
//     use crate::progress::ProgressUpdate;
//     use crate::schema::{suggest_mapping, validate_mapping};
//     use crate::session::{
//         load_analysis_session, load_mapping_profile, save_analysis_session, save_mapping_profile,
//     };
//     use crate::worker::{spawn_analysis, WorkerMessage};
//     use gpui::{
//         div, px, rgb, App, AppContext, Context, Div, Entity, FontWeight, InteractiveElement,
//         IntoElement, ParentElement, Pixels, Render, SharedString, Stateful,
//         StatefulInteractiveElement, Styled, Window,
//     };
//     use gpui_component::{
//         button::Button,
//         list::ListItem,
//         menu::{DropdownMenu, PopupMenuItem},
//         resizable::{h_resizable, resizable_panel},
//         table::{Column, Table, TableDelegate, TableState},
//         theme::ActiveTheme,
//         tree::{tree, TreeItem, TreeState},
//     };
//     use std::collections::{BTreeMap, BTreeSet};
//     use std::path::{Path, PathBuf};
//     use std::sync::mpsc::Receiver;
//     use std::time::{Duration, Instant};

//     // ── Color palette ───────────────────────────────────────────────────

//     #[derive(Debug, Clone, Copy)]
//     struct Palette {
//         bg: u32,
//         panel: u32,
//         ink: u32,
//         muted: u32,
//         border: u32,
//         accent: u32,
//         accent_dark: u32,
//         button_bg: u32,
//         disabled_bg: u32,
//         accent_text: u32,
//         active_bg: u32,
//         table_header: u32,
//         alt_row: u32,
//         progress_track: u32,
//     }

//     impl Palette {
//         /// Warm neutral light theme — easy on the eyes, good contrast.
//         /// Base: warm off-white / linen tones.
//         /// Accent: muted slate-teal — professional, not neon.
//         fn light() -> Self {
//             Self {
//                 bg: 0xf5f3ef,             // warm off-white background
//                 panel: 0xfcfaf7,          // slightly brighter panel surface
//                 ink: 0x2c2f33,            // dark charcoal text (softer than pure black)
//                 muted: 0x7a7f85,          // mid-gray for secondary text
//                 border: 0xd4d6d1,         // subtle warm gray border
//                 accent: 0x3d7a6a,         // muted teal — clear but gentle
//                 accent_dark: 0x2e5f52,    // darker teal for pressed / borders
//                 button_bg: 0xf0eeea,      // button resting state
//                 disabled_bg: 0xe4e2de,    // disabled controls
//                 accent_text: 0xffffff,    // white text on accent backgrounds
//                 active_bg: 0xddeee7,      // light teal tint for active/selected
//                 table_header: 0xeae8e3,   // warm header row
//                 alt_row: 0xf8f6f2,        // very subtle alternating row
//                 progress_track: 0xdddbd6, // progress bar track
//             }
//         }

//         /// Dark theme — deep charcoal, not pure black. Comfortable for
//         /// extended use; accent stands out without being glaring.
//         fn dark() -> Self {
//             Self {
//                 bg: 0x1c1e21,             // deep charcoal background
//                 panel: 0x252830,          // slightly elevated panel surface
//                 ink: 0xd8dae0,            // light gray text (softer than pure white)
//                 muted: 0x848990,          // de-saturated mid-gray
//                 border: 0x3a3d44,         // subtle dark border
//                 accent: 0x5aab95,         // brighter teal that reads well on dark
//                 accent_dark: 0x468a78,    // pressed / ring teal
//                 button_bg: 0x2e3138,      // button resting state
//                 disabled_bg: 0x33363d,    // disabled controls
//                 accent_text: 0xffffff,    // white text on accent backgrounds
//                 active_bg: 0x293d36,      // dark teal wash for active/selected
//                 table_header: 0x2b2e35,   // header row
//                 alt_row: 0x22252b,        // very subtle alternating row
//                 progress_track: 0x353840, // progress bar track
//             }
//         }
//     }

//     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//     enum Screen {
//         Import,
//         Mapping,
//         Run,
//         Results,
//     }

//     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//     enum ResultTreeSelection {
//         Cluster(usize),
//         Theme { cluster: usize, theme: usize },
//     }

//     pub struct IncidentClusteringApp {
//         screen: Screen,
//         source: Option<SourceTable>,
//         mapping: ColumnMapping,
//         settings: RunSettings,
//         analysis: Option<AnalysisRun>,
//         worker: Option<Receiver<WorkerMessage>>,
//         current_progress: Option<ProgressUpdate>,
//         progress_log: Vec<ProgressLogEntry>,
//         result_filters: ResultFilters,
//         result_tree_selection: Option<ResultTreeSelection>,
//         tree_state: Option<Entity<TreeState>>,
//         detail_table_state: Option<Entity<TableState<GridTableDelegate>>>,
//         tree_panel_width: Pixels,
//         run_started_at: Option<Instant>,
//         last_run_elapsed: Option<Duration>,
//         status: String,
//         worksheets: Vec<String>,
//         selected_worksheet: Option<String>,
//         palette: Palette,
//     }

//     #[derive(Debug, Clone)]
//     struct ProgressLogEntry {
//         elapsed: Duration,
//         update: ProgressUpdate,
//     }

//     #[derive(Debug, Clone, Default)]
//     struct ResultFilters {
//         selected_values: BTreeMap<usize, BTreeSet<String>>,
//     }

//     impl ResultFilters {
//         fn active_count(&self) -> usize {
//             self.selected_values.len()
//         }

//         fn clear(&mut self) {
//             self.selected_values.clear();
//         }

//         fn matches_row(&self, row: &[String]) -> bool {
//             self.selected_values.iter().all(|(column, selected)| {
//                 row.get(*column)
//                     .map(|value| selected.contains(value))
//                     .unwrap_or_else(|| selected.contains(""))
//             })
//         }

//         fn filtered_row_indices(&self, rows: &[Vec<String>], row_indices: &[usize]) -> Vec<usize> {
//             row_indices
//                 .iter()
//                 .copied()
//                 .filter(|row_index| {
//                     rows.get(*row_index)
//                         .map(|row| self.matches_row(row))
//                         .unwrap_or(false)
//                 })
//                 .collect()
//         }

//         fn selected_count(&self, column: usize, total_values: usize) -> usize {
//             self.selected_values
//                 .get(&column)
//                 .map(BTreeSet::len)
//                 .unwrap_or(total_values)
//         }

//         fn value_is_selected(&self, column: usize, value: &str) -> bool {
//             self.selected_values
//                 .get(&column)
//                 .map(|selected| selected.contains(value))
//                 .unwrap_or(true)
//         }

//         fn select_all(&mut self, column: usize) {
//             self.selected_values.remove(&column);
//         }

//         fn select_none(&mut self, column: usize) {
//             self.selected_values.insert(column, BTreeSet::new());
//         }

//         fn set_value_selected(
//             &mut self,
//             column: usize,
//             value: &str,
//             selected: bool,
//             all_values: &BTreeSet<String>,
//         ) {
//             let selected_values = self
//                 .selected_values
//                 .entry(column)
//                 .or_insert_with(|| all_values.clone());

//             if selected {
//                 selected_values.insert(value.to_owned());
//             } else {
//                 selected_values.remove(value);
//             }

//             if selected_values.len() == all_values.len() {
//                 self.selected_values.remove(&column);
//             }
//         }
//     }

//     const DEFAULT_TREE_PANEL_WIDTH: Pixels = px(360.0);

//     impl Default for IncidentClusteringApp {
//         fn default() -> Self {
//             Self {
//                 screen: Screen::Import,
//                 source: None,
//                 mapping: ColumnMapping::default(),
//                 settings: RunSettings::default(),
//                 analysis: None,
//                 worker: None,
//                 current_progress: None,
//                 progress_log: Vec::new(),
//                 result_filters: ResultFilters::default(),
//                 result_tree_selection: None,
//                 tree_state: None,
//                 detail_table_state: None,
//                 tree_panel_width: DEFAULT_TREE_PANEL_WIDTH,
//                 run_started_at: None,
//                 last_run_elapsed: None,
//                 status: "Select a CSV or Excel incident export.".to_owned(),
//                 worksheets: Vec::new(),
//                 selected_worksheet: None,
//                 palette: Palette::light(),
//             }
//         }
//     }

//     // ── Render ───────────────────────────────────────────────────────────

//     impl Render for IncidentClusteringApp {
//         fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
//             // Sync palette with system appearance each frame.
//             self.palette = if cx.theme().is_dark() {
//                 Palette::dark()
//             } else {
//                 Palette::light()
//             };

//             self.poll_worker(window);

//             // Rebuild the persisted tree/table state when dirty (set to None on
//             // analysis/filter/selection changes). Done here so every screen
//             // method can stay &self.
//             if self.screen == Screen::Results && self.analysis.is_some() {
//                 if self.tree_state.is_none() {
//                     self.rebuild_tree_state(cx);
//                 }
//                 if self.detail_table_state.is_none() {
//                     self.rebuild_detail_table_state(window, cx);
//                 }
//             }

//             let status = self.status_line();
//             let screen = self.screen;
//             let pal = self.palette;

//             let screen_content = match screen {
//                 Screen::Import => self.import_screen(window, cx).into_any_element(),
//                 Screen::Mapping => self.mapping_screen(cx).into_any_element(),
//                 Screen::Run => self.run_screen(window, cx).into_any_element(),
//                 Screen::Results => self.results_screen(window, cx).into_any_element(),
//             };

//             // Results screen fills the viewport; other screens scroll.
//             let mut content_area = div().id("main-content").flex_1().min_h_0();
//             if screen == Screen::Results {
//                 content_area = content_area.overflow_hidden().flex().flex_col();
//             } else {
//                 content_area = content_area.overflow_y_scroll().p_4();
//             }

//             div()
//                 .size_full()
//                 .bg(rgb(pal.bg))
//                 .text_color(rgb(pal.ink))
//                 .font_family("Aptos")
//                 .flex()
//                 .flex_col()
//                 .child(self.top_bar(cx))
//                 .child(content_area.child(screen_content))
//                 .child(
//                     div()
//                         .border_t_1()
//                         .border_color(rgb(pal.border))
//                         .bg(rgb(pal.panel))
//                         .px_5()
//                         .py_2()
//                         .text_sm()
//                         .text_color(rgb(pal.muted))
//                         .child(status),
//                 )
//         }
//     }

//     // ── UI screens & components ──────────────────────────────────────────

//     impl IncidentClusteringApp {
//         // -- tree state management ----------------------------------------

//         fn rebuild_tree_state(&mut self, cx: &mut Context<Self>) {
//             let analysis = self.analysis.as_ref().unwrap();
//             let items = build_cluster_tree_items(
//                 &analysis.clusters,
//                 &analysis.source.rows,
//                 &self.result_filters,
//             );
//             self.tree_state = Some(cx.new(|cx| TreeState::new(cx).items(items)));
//         }

//         fn invalidate_tree(&mut self) {
//             self.tree_state = None;
//             self.detail_table_state = None;
//         }

//         fn invalidate_detail_table(&mut self) {
//             self.detail_table_state = None;
//         }

//         fn rebuild_detail_table_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
//             let Some(analysis) = &self.analysis else {
//                 return;
//             };
//             let detail_rows = self.selected_detail_rows(analysis);
//             let headers = analysis.source.headers.clone();
//             let pal = self.palette;
//             self.detail_table_state = Some(cx.new(|cx| {
//                 TableState::new(
//                     GridTableDelegate::new(headers, detail_rows, pal),
//                     window,
//                     cx,
//                 )
//             }));
//         }

//         // -- top bar ------------------------------------------------------

//         fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
//             let pal = self.palette;
//             let rows = self
//                 .source
//                 .as_ref()
//                 .map(|source| format!("{} rows", source.row_count()))
//                 .unwrap_or_else(|| "No source loaded".to_owned());

//             div()
//                 .bg(rgb(pal.panel))
//                 .border_b_1()
//                 .border_color(rgb(pal.border))
//                 .px_5()
//                 .py_3()
//                 .flex()
//                 .flex_col()
//                 .gap_2()
//                 .child(
//                     div()
//                         .flex()
//                         .items_center()
//                         .justify_between()
//                         .child(
//                             div()
//                                 .text_xl()
//                                 .font_weight(FontWeight::BOLD)
//                                 .child("Incident Clustering Analyzer"),
//                         )
//                         .child(div().text_sm().text_color(rgb(pal.muted)).child(rows)),
//                 )
//                 .child(
//                     div()
//                         .flex()
//                         .flex_wrap()
//                         .gap_2()
//                         .child(self.workflow_button(cx, Screen::Import, "1", "Source"))
//                         .child(self.workflow_button(cx, Screen::Mapping, "2", "Mapping"))
//                         .child(self.workflow_button(cx, Screen::Run, "3", "Analysis"))
//                         .child(self.workflow_button(cx, Screen::Results, "4", "Results")),
//                 )
//         }

//         fn workflow_button(
//             &self,
//             cx: &mut Context<Self>,
//             screen: Screen,
//             number: &str,
//             label: &str,
//         ) -> impl IntoElement {
//             let pal = self.palette;
//             let enabled = match screen {
//                 Screen::Import => true,
//                 Screen::Mapping | Screen::Run => self.source.is_some(),
//                 Screen::Results => self.analysis.is_some(),
//             };
//             let selected = self.screen == screen;
//             let title = format!("{number}  {label}  {}", self.step_state(screen));
//             let mut item =
//                 button_base(format!("workflow-{number}"), title, selected, enabled, &pal);
//             if enabled {
//                 item = item.on_click(cx.listener(move |view, _, _window, cx| {
//                     view.screen = screen;
//                     cx.notify();
//                 }));
//             }
//             item
//         }

//         // -- 1. Import screen ---------------------------------------------

//         fn import_screen(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
//             let pal = self.palette;
//             let mut content = screen("1. Source file").child(
//                 div()
//                     .flex()
//                     .flex_wrap()
//                     .gap_2()
//                     .child(action_button(
//                         cx,
//                         "open-source",
//                         "Open CSV/XLSX",
//                         true,
//                         |view, cx| {
//                             view.open_source_file();
//                             cx.notify();
//                         },
//                         &pal,
//                     ))
//                     .child(action_button(
//                         cx,
//                         "load-session",
//                         "Load Session",
//                         true,
//                         |view, cx| {
//                             view.load_session();
//                             cx.notify();
//                         },
//                         &pal,
//                     )),
//             );

//             if let Some(source) = &self.source {
//                 content = content.child(inline_stats(
//                     vec![
//                         ("Rows", source.row_count().to_string()),
//                         ("Columns", source.headers.len().to_string()),
//                         (
//                             "File",
//                             source
//                                 .source_path
//                                 .as_ref()
//                                 .and_then(|path| path.file_name())
//                                 .and_then(|name| name.to_str())
//                                 .unwrap_or("Loaded session")
//                                 .to_owned(),
//                         ),
//                     ],
//                     &pal,
//                 ));
//             }

//             if !self.worksheets.is_empty() {
//                 content = content.child(
//                     div()
//                         .flex()
//                         .items_center()
//                         .flex_wrap()
//                         .gap_2()
//                         .child(
//                             div()
//                                 .text_sm()
//                                 .font_weight(FontWeight::SEMIBOLD)
//                                 .child("Worksheet"),
//                         )
//                         .children(self.worksheets.iter().cloned().map(|sheet| {
//                             let selected = self.selected_worksheet.as_ref() == Some(&sheet);
//                             let label = sheet.clone();
//                             button_base(format!("sheet-{sheet}"), label, selected, true, &pal)
//                                 .on_click(cx.listener(move |view, _, _window, cx| {
//                                     view.selected_worksheet = Some(sheet.clone());
//                                     view.open_selected_worksheet();
//                                     cx.notify();
//                                 }))
//                         })),
//                 );
//             }

//             content = content.child(self.source_preview(window, cx));

//             if self.source.is_some() {
//                 content = content.child(primary_action_button(
//                     cx,
//                     "confirm-source",
//                     "Confirm Source",
//                     true,
//                     |view, cx| {
//                         view.screen = Screen::Mapping;
//                         cx.notify();
//                     },
//                     &pal,
//                 ));
//             }

//             content
//         }

//         // -- 2. Mapping screen --------------------------------------------

//         fn mapping_screen(&self, cx: &mut Context<Self>) -> impl IntoElement {
//             let pal = self.palette;
//             let Some(source) = &self.source else {
//                 return screen("2. Field mapping").child("No source file loaded.");
//             };

//             let headers = source.headers.clone();
//             screen("2. Field mapping")
//                 .child(inline_stats(
//                     vec![
//                         ("Required", self.required_mapping_status()),
//                         (
//                             "Additional text",
//                             self.mapping.additional_text.len().to_string(),
//                         ),
//                         ("Filters", self.filter_mapping_count().to_string()),
//                     ],
//                     &pal,
//                 ))
//                 .child(
//                     div()
//                         .flex()
//                         .flex_col()
//                         .child(section_label("Required fields", &pal))
//                         .child(self.mapping_picker(
//                             cx,
//                             "Incident number",
//                             headers.clone(),
//                             self.mapping.incident_number,
//                             |mapping, value| mapping.incident_number = value,
//                         ))
//                         .child(self.mapping_picker(
//                             cx,
//                             "Short description",
//                             headers.clone(),
//                             self.mapping.short_description,
//                             |mapping, value| mapping.short_description = value,
//                         )),
//                 )
//                 .child(
//                     div()
//                         .flex()
//                         .flex_col()
//                         .child(section_label("Filter and context fields", &pal))
//                         .child(self.mapping_picker(
//                             cx,
//                             "Assignment group",
//                             headers.clone(),
//                             self.mapping.assignment_group,
//                             |mapping, value| mapping.assignment_group = value,
//                         ))
//                         .child(self.mapping_picker(
//                             cx,
//                             "Service",
//                             headers.clone(),
//                             self.mapping.service,
//                             |mapping, value| mapping.service = value,
//                         ))
//                         .child(self.mapping_picker(
//                             cx,
//                             "Category",
//                             headers.clone(),
//                             self.mapping.category,
//                             |mapping, value| mapping.category = value,
//                         ))
//                         .child(self.mapping_picker(
//                             cx,
//                             "Configuration item",
//                             headers.clone(),
//                             self.mapping.configuration_item,
//                             |mapping, value| mapping.configuration_item = value,
//                         ))
//                         .child(self.mapping_picker(
//                             cx,
//                             "Date",
//                             headers.clone(),
//                             self.mapping.date,
//                             |mapping, value| mapping.date = value,
//                         )),
//                 )
//                 .child(
//                     div()
//                         .flex()
//                         .flex_col()
//                         .child(section_label("Additional text for similarity", &pal))
//                         .children(headers.iter().enumerate().map(|(index, header)| {
//                             let selected = self.mapping.additional_text.contains(&index);
//                             div()
//                                 .id(SharedString::from(format!("additional-text-{index}")))
//                                 .border_t_1()
//                                 .border_color(rgb(pal.border))
//                                 .py_1()
//                                 .min_h(px(34.0))
//                                 .flex()
//                                 .items_center()
//                                 .gap_3()
//                                 .cursor_pointer()
//                                 .child(
//                                     div()
//                                         .w(px(14.0))
//                                         .h(px(14.0))
//                                         .border_1()
//                                         .border_color(if selected {
//                                             rgb(pal.accent)
//                                         } else {
//                                             rgb(pal.border)
//                                         })
//                                         .bg(if selected {
//                                             rgb(pal.accent)
//                                         } else {
//                                             rgb(pal.panel)
//                                         })
//                                         .flex_shrink_0(),
//                                 )
//                                 .child(div().text_sm().child(header.clone()))
//                                 .on_click(cx.listener(move |view, _, _window, cx| {
//                                     if view.mapping.additional_text.contains(&index) {
//                                         view.mapping
//                                             .additional_text
//                                             .retain(|value| *value != index);
//                                     } else {
//                                         view.mapping.additional_text.push(index);
//                                         view.mapping.additional_text.sort_unstable();
//                                         view.mapping.additional_text.dedup();
//                                     }
//                                     cx.notify();
//                                 }))
//                         })),
//                 )
//                 .child(
//                     div()
//                         .flex()
//                         .flex_wrap()
//                         .gap_2()
//                         .pt_2()
//                         .child(action_button(
//                             cx,
//                             "save-mapping",
//                             "Save Mapping",
//                             true,
//                             |view, cx| {
//                                 view.save_mapping();
//                                 cx.notify();
//                             },
//                             &pal,
//                         ))
//                         .child(action_button(
//                             cx,
//                             "load-mapping",
//                             "Load Mapping",
//                             true,
//                             |view, cx| {
//                                 view.load_mapping();
//                                 cx.notify();
//                             },
//                             &pal,
//                         ))
//                         .child(primary_action_button(
//                             cx,
//                             "confirm-mapping",
//                             "Confirm Mapping",
//                             self.mapping_ready(),
//                             |view, cx| {
//                                 if let Some(source) = &view.source {
//                                     match validate_mapping(&view.mapping, source) {
//                                         Ok(()) => view.screen = Screen::Run,
//                                         Err(err) => view.status = err.to_string(),
//                                     }
//                                 }
//                                 cx.notify();
//                             },
//                             &pal,
//                         )),
//                 )
//         }

//         fn mapping_picker(
//             &self,
//             cx: &mut Context<Self>,
//             label: &'static str,
//             headers: Vec<String>,
//             selected: Option<usize>,
//             update: fn(&mut ColumnMapping, Option<usize>),
//         ) -> impl IntoElement {
//             let pal = self.palette;
//             let selected_text = selected
//                 .and_then(|index| headers.get(index).cloned())
//                 .unwrap_or_else(|| "Not mapped".to_owned());
//             let app = cx.weak_entity();

//             div()
//                 .border_t_1()
//                 .border_color(rgb(pal.border))
//                 .py_1()
//                 .min_h(px(34.0))
//                 .flex()
//                 .items_center()
//                 .gap_3()
//                 .child(
//                     div()
//                         .w(px(190.0))
//                         .text_sm()
//                         .font_weight(FontWeight::SEMIBOLD)
//                         .child(label),
//                 )
//                 .child(
//                     split_dropdown_button(label, truncate(&selected_text, 36), false, &pal)
//                         .dropdown_menu(move |mut menu, _window, _cx| {
//                             let none_app = app.clone();
//                             menu = menu.item(
//                                 PopupMenuItem::new("Not mapped")
//                                     .checked(selected.is_none())
//                                     .on_click(move |_, _, cx| {
//                                         none_app
//                                             .update(cx, |view, cx| {
//                                                 update(&mut view.mapping, None);
//                                                 cx.notify();
//                                             })
//                                             .ok();
//                                     }),
//                             );

//                             for (index, header) in headers.iter().cloned().enumerate() {
//                                 let item_app = app.clone();
//                                 let is_selected = selected == Some(index);
//                                 menu = menu.item(
//                                     PopupMenuItem::new(truncate(&header, 56))
//                                         .checked(is_selected)
//                                         .on_click(move |_, _, cx| {
//                                             item_app
//                                                 .update(cx, |view, cx| {
//                                                     update(&mut view.mapping, Some(index));
//                                                     cx.notify();
//                                                 })
//                                                 .ok();
//                                         }),
//                                 );
//                             }
//                             menu
//                         }),
//                 )
//                 .child(
//                     div()
//                         .ml_auto()
//                         .text_xs()
//                         .text_color(rgb(pal.muted))
//                         .child(selected_text),
//                 )
//         }

//         // -- 3. Run / progress screen -------------------------------------

//         fn run_screen(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
//             let pal = self.palette;
//             if self.worker.is_some() {
//                 return screen("3. Run analysis").child(self.progress_view(window, cx));
//             }

//             let mut content = screen("3. Run analysis");
//             if let Some(source) = &self.source {
//                 content = content.child(inline_stats(
//                     vec![
//                         ("Source rows", source.row_count().to_string()),
//                         ("Mapped fields", self.mapped_field_count().to_string()),
//                         (
//                             "Minimum cluster size",
//                             self.settings.minimum_cluster_size.to_string(),
//                         ),
//                     ],
//                     &pal,
//                 ));
//             }

//             content
//                 .child(
//                     div()
//                         .flex()
//                         .flex_col()
//                         .child(section_label("Analysis settings", &pal))
//                         .child(self.number_control(
//                             cx,
//                             "min-cluster",
//                             "Minimum useful cluster size",
//                             self.settings.minimum_cluster_size,
//                             2,
//                             1000,
//                             5,
//                             |settings, value| settings.minimum_cluster_size = value,
//                         ))
//                         .child(self.number_control(
//                             cx,
//                             "cluster-threshold",
//                             "Cluster similarity threshold",
//                             self.settings.similarity_threshold_percent as usize,
//                             10,
//                             90,
//                             1,
//                             |settings, value| settings.similarity_threshold_percent = value as u8,
//                         ))
//                         .child(self.number_control(
//                             cx,
//                             "subgroup-threshold",
//                             "Subgroup similarity threshold",
//                             self.settings.subgroup_similarity_threshold_percent as usize,
//                             10,
//                             95,
//                             1,
//                             |settings, value| {
//                                 settings.subgroup_similarity_threshold_percent = value as u8
//                             },
//                         )),
//                 )
//                 .child(div().pt_2().flex().child(primary_action_button(
//                     cx,
//                     "start-analysis",
//                     "Start Analysis",
//                     true,
//                     |view, cx| {
//                         view.start_analysis();
//                         cx.notify();
//                     },
//                     &pal,
//                 )))
//         }

//         fn number_control(
//             &self,
//             cx: &mut Context<Self>,
//             id: &'static str,
//             label: &'static str,
//             value: usize,
//             min: usize,
//             max: usize,
//             step: usize,
//             update: fn(&mut RunSettings, usize),
//         ) -> impl IntoElement {
//             let pal = self.palette;
//             let dec_value = value.saturating_sub(step).max(min);
//             let inc_value = value.saturating_add(step).min(max);
//             div()
//                 .border_t_1()
//                 .border_color(rgb(pal.border))
//                 .py_1()
//                 .min_h(px(38.0))
//                 .flex()
//                 .items_center()
//                 .justify_between()
//                 .gap_3()
//                 .child(
//                     div()
//                         .flex()
//                         .items_center()
//                         .gap_2()
//                         .child(
//                             div()
//                                 .text_sm()
//                                 .font_weight(FontWeight::SEMIBOLD)
//                                 .child(label),
//                         )
//                         .child(
//                             div()
//                                 .text_xs()
//                                 .text_color(rgb(pal.muted))
//                                 .child(format!("Range {min}-{max}, step {step}")),
//                         ),
//                 )
//                 .child(
//                     div()
//                         .flex()
//                         .items_center()
//                         .gap_2()
//                         .child(
//                             button_base(format!("{id}-dec"), "-", false, value > min, &pal)
//                                 .on_click(cx.listener(move |view, _, _window, cx| {
//                                     update(&mut view.settings, dec_value);
//                                     cx.notify();
//                                 })),
//                         )
//                         .child(
//                             div()
//                                 .min_w(px(44.0))
//                                 .text_center()
//                                 .font_weight(FontWeight::BOLD)
//                                 .child(value.to_string()),
//                         )
//                         .child(
//                             button_base(format!("{id}-inc"), "+", false, value < max, &pal)
//                                 .on_click(cx.listener(move |view, _, _window, cx| {
//                                     update(&mut view.settings, inc_value);
//                                     cx.notify();
//                                 })),
//                         ),
//                 )
//         }

//         fn progress_view(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
//             let pal = self.palette;
//             let Some(progress) = &self.current_progress else {
//                 return div()
//                     .p_4()
//                     .child("Starting analysis worker.")
//                     .into_any_element();
//             };

//             let elapsed = self
//                 .run_started_at
//                 .map(|started_at| started_at.elapsed())
//                 .unwrap_or_default();

//             let mut content = div()
//                 .flex()
//                 .flex_col()
//                 .gap_3()
//                 .child(inline_stats(
//                     vec![
//                         ("Stage", progress.stage.clone()),
//                         (
//                             "Step",
//                             format!("{} of {}", progress.step, progress.total_steps),
//                         ),
//                         (
//                             "Sub-step",
//                             progress
//                                 .substep
//                                 .as_ref()
//                                 .map(|substep| format!("{} of {}", substep.current, substep.total))
//                                 .unwrap_or_else(|| "-".to_owned()),
//                         ),
//                         ("Elapsed", format_duration(elapsed)),
//                     ],
//                     &pal,
//                 ))
//                 .child(
//                     container(&pal)
//                         .child(
//                             div()
//                                 .text_sm()
//                                 .font_weight(FontWeight::SEMIBOLD)
//                                 .child(progress.stage.clone()),
//                         )
//                         .child(
//                             div()
//                                 .text_sm()
//                                 .text_color(rgb(pal.muted))
//                                 .child(progress.detail.clone()),
//                         )
//                         .child(progress_bar(progress.fraction(), &pal)),
//                 );

//             if !progress.workers.is_empty() {
//                 content = content.child(
//                     div()
//                         .flex()
//                         .flex_col()
//                         .gap_1()
//                         .child(section_label("Parallel workers", &pal))
//                         .child(data_table(
//                             vec![
//                                 "Worker".to_owned(),
//                                 "Processed".to_owned(),
//                                 "Share".to_owned(),
//                             ],
//                             progress
//                                 .workers
//                                 .iter()
//                                 .map(|worker| {
//                                     vec![
//                                         format!("#{}", worker.worker),
//                                         format!("{}/{}", worker.completed, worker.total),
//                                         format!("{:.0}%", worker.fraction() * 100.0),
//                                     ]
//                                 })
//                                 .collect(),
//                             window,
//                             cx,
//                             &pal,
//                         )),
//                 );
//             }

//             content
//                 .child(
//                     div()
//                         .flex()
//                         .flex_col()
//                         .gap_1()
//                         .child(section_label("Pipeline activity", &pal))
//                         .child(data_table(
//                             vec![
//                                 "Time".to_owned(),
//                                 "Main".to_owned(),
//                                 "Sub".to_owned(),
//                                 "Stage".to_owned(),
//                                 "Detail".to_owned(),
//                             ],
//                             self.progress_log
//                                 .iter()
//                                 .rev()
//                                 .map(|entry| {
//                                     vec![
//                                         format_duration(entry.elapsed),
//                                         format!(
//                                             "{}/{}",
//                                             entry.update.step, entry.update.total_steps
//                                         ),
//                                         entry
//                                             .update
//                                             .substep
//                                             .as_ref()
//                                             .map(|substep| {
//                                                 format!("{}/{}", substep.current, substep.total)
//                                             })
//                                             .unwrap_or_else(|| "-".to_owned()),
//                                         entry.update.stage.clone(),
//                                         entry.update.detail.clone(),
//                                     ]
//                                 })
//                                 .collect(),
//                             window,
//                             cx,
//                             &pal,
//                         )),
//                 )
//                 .into_any_element()
//         }

//         fn source_preview(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
//             let pal = self.palette;
//             let Some(source) = &self.source else {
//                 return div().into_any_element();
//             };

//             div()
//                 .flex()
//                 .flex_col()
//                 .gap_1()
//                 .child(section_label(
//                     format!(
//                         "Preview: {} columns, {} rows",
//                         source.headers.len(),
//                         source.row_count()
//                     ),
//                     &pal,
//                 ))
//                 .child(
//                     div()
//                         .id("source-preview-scroll")
//                         .max_h(px(360.0))
//                         .overflow_scroll()
//                         .border_1()
//                         .border_color(rgb(pal.border))
//                         .child(data_table(
//                             source.headers.clone(),
//                             source.rows.iter().take(25).cloned().collect(),
//                             window,
//                             cx,
//                             &pal,
//                         )),
//                 )
//                 .into_any_element()
//         }

//         // -- 4. Results screen --------------------------------------------

//         fn results_screen(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
//             let pal = self.palette;
//             let Some(analysis) = &self.analysis else {
//                 return div().flex_1().p_4().child("No analysis results available.");
//             };

//             let filtered_count = self.filtered_processed_count(analysis);
//             let filter_options = self.result_filter_options(analysis);

//             div()
//                 .flex_1()
//                 .min_h_0()
//                 .flex()
//                 .flex_col()
//                 // ── header bar: stats + actions + filters ──
//                 .child(
//                     div()
//                         .bg(rgb(pal.panel))
//                         .border_b_1()
//                         .border_color(rgb(pal.border))
//                         .px_4()
//                         .py_2()
//                         .flex()
//                         .flex_col()
//                         .gap_2()
//                         .child(
//                             div()
//                                 .flex()
//                                 .flex_wrap()
//                                 .items_center()
//                                 .justify_between()
//                                 .gap_2()
//                                 .child(inline_stats(
//                                     vec![
//                                         (
//                                             "Processed",
//                                             analysis.processed_incidents.len().to_string(),
//                                         ),
//                                         ("Visible", filtered_count.to_string()),
//                                         ("Clusters", analysis.clusters.len().to_string()),
//                                         (
//                                             "Unclustered",
//                                             analysis.unclustered_row_indices.len().to_string(),
//                                         ),
//                                         ("Ignored", analysis.ignored_rows.len().to_string()),
//                                     ],
//                                     &pal,
//                                 ))
//                                 .child(
//                                     div()
//                                         .flex()
//                                         .gap_2()
//                                         .child(action_button(
//                                             cx,
//                                             "export-excel",
//                                             "Export Excel",
//                                             true,
//                                             |view, cx| {
//                                                 view.export_results();
//                                                 cx.notify();
//                                             },
//                                             &pal,
//                                         ))
//                                         .child(action_button(
//                                             cx,
//                                             "save-session",
//                                             "Save Session",
//                                             true,
//                                             |view, cx| {
//                                                 view.save_session();
//                                                 cx.notify();
//                                             },
//                                             &pal,
//                                         )),
//                                 ),
//                         )
//                         .child(self.result_filter_bar(
//                             cx,
//                             &analysis.source.headers,
//                             &filter_options,
//                             filtered_count,
//                         )),
//                 )
//                 // ── explorer: tree + detail table ──
//                 .child(self.results_explorer(window, cx, analysis))
//         }

//         fn result_filter_bar(
//             &self,
//             cx: &mut Context<Self>,
//             headers: &[String],
//             filter_options: &[BTreeSet<String>],
//             filtered_processed_count: usize,
//         ) -> impl IntoElement {
//             let pal = self.palette;
//             let status = if self.result_filters.active_count() == 0 {
//                 format!("{filtered_processed_count} visible")
//             } else {
//                 format!(
//                     "{} active, {filtered_processed_count} visible",
//                     self.result_filters.active_count()
//                 )
//             };

//             div()
//                 .id("result-filter-bar")
//                 .flex()
//                 .flex_wrap()
//                 .items_center()
//                 .gap_2()
//                 .child(
//                     div()
//                         .font_weight(FontWeight::SEMIBOLD)
//                         .text_sm()
//                         .child("Filters"),
//                 )
//                 .child(div().text_xs().text_color(rgb(pal.muted)).child(status))
//                 .children(headers.iter().enumerate().filter_map(|(column, header)| {
//                     filter_options
//                         .get(column)
//                         .map(|values| self.result_filter_group(cx, column, header, values))
//                 }))
//                 .child(action_button(
//                     cx,
//                     "clear-filters",
//                     "Clear",
//                     true,
//                     |view, cx| {
//                         view.result_filters.clear();
//                         view.invalidate_tree();
//                         cx.notify();
//                     },
//                     &pal,
//                 ))
//         }

//         fn result_filter_group(
//             &self,
//             cx: &mut Context<Self>,
//             column: usize,
//             header: &str,
//             values: &BTreeSet<String>,
//         ) -> impl IntoElement {
//             let pal = self.palette;
//             let selected_count = self.result_filters.selected_count(column, values.len());
//             let result_filters = self.result_filters.clone();
//             let values = values.clone();
//             let app = cx.weak_entity();
//             let is_filtered = selected_count < values.len();
//             let button_label = format!(
//                 "{} ({selected_count}/{})",
//                 truncate(header, 24),
//                 values.len()
//             );

//             split_dropdown_button(
//                 format!("filter-menu-{column}"),
//                 button_label,
//                 is_filtered,
//                 &pal,
//             )
//             .dropdown_menu(move |mut menu, _window, _cx| {
//                 let all_app = app.clone();
//                 menu = menu.item(PopupMenuItem::new("Select all").on_click(move |_, _, cx| {
//                     all_app
//                         .update(cx, |view, cx| {
//                             view.result_filters.select_all(column);
//                             view.invalidate_tree();
//                             cx.notify();
//                         })
//                         .ok();
//                 }));

//                 let none_app = app.clone();
//                 menu = menu.item(PopupMenuItem::new("Select none").on_click(move |_, _, cx| {
//                     none_app
//                         .update(cx, |view, cx| {
//                             view.result_filters.select_none(column);
//                             view.invalidate_tree();
//                             cx.notify();
//                         })
//                         .ok();
//                 }));

//                 menu = menu.item(PopupMenuItem::separator());

//                 #[allow(clippy::unnecessary_to_owned)] // value must be owned for the move closure
//                 for value in values.iter().take(80).cloned() {
//                     let value_app = app.clone();
//                     let all_values = values.clone();
//                     let display = if value.is_empty() {
//                         "(blank)".to_owned()
//                     } else {
//                         truncate(&value, 64)
//                     };
//                     let selected = result_filters.value_is_selected(column, &value);
//                     let label = if selected {
//                         format!("\u{2713}  {display}")
//                     } else {
//                         format!("    {display}")
//                     };
//                     menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
//                         value_app
//                             .update(cx, |view, cx| {
//                                 let selected_now =
//                                     view.result_filters.value_is_selected(column, &value);
//                                 view.result_filters.set_value_selected(
//                                     column,
//                                     &value,
//                                     !selected_now,
//                                     &all_values,
//                                 );
//                                 view.invalidate_tree();
//                                 cx.notify();
//                             })
//                             .ok();
//                     }));
//                 }
//                 menu
//             })
//         }

//         // -- results explorer (tree + detail table) -----------------------

//         fn results_explorer(
//             &self,
//             _window: &mut Window,
//             cx: &mut Context<Self>,
//             _analysis: &AnalysisRun,
//         ) -> impl IntoElement {
//             let pal = self.palette;
//             let tree_state = self
//                 .tree_state
//                 .as_ref()
//                 .expect("tree state built in render");
//             let table_state = self
//                 .detail_table_state
//                 .as_ref()
//                 .expect("detail table state built in render");
//             let detail_title = self.detail_table_title();
//             let selected = self.result_tree_selection;
//             let app = cx.weak_entity();
//             let tree_width = self.tree_panel_width;

//             let tree_pane = div()
//                 .size_full()
//                 .border_r_1()
//                 .border_color(rgb(pal.border))
//                 .bg(rgb(pal.panel))
//                 .flex()
//                 .flex_col()
//                 .child(
//                     div()
//                         .px_3()
//                         .py_2()
//                         .border_b_1()
//                         .border_color(rgb(pal.border))
//                         .text_sm()
//                         .font_weight(FontWeight::SEMIBOLD)
//                         .child("Clusters"),
//                 )
//                 .child(
//                     div()
//                         .id("cluster-tree-scroll")
//                         .flex_1()
//                         .min_h_0()
//                         .overflow_y_scroll()
//                         .child(tree(
//                             tree_state,
//                             move |ix, entry, tree_selected, _window, _cx| {
//                                 let item_selection = selection_from_tree_id(&entry.item().id);
//                                 let app = app.clone();
//                                 let chevron = if entry.is_folder() {
//                                     if entry.is_expanded() {
//                                         "\u{25bc} "
//                                     } else {
//                                         "\u{25b6} "
//                                     }
//                                 } else {
//                                     ""
//                                 };
//                                 ListItem::new(ix)
//                                     .h(px(28.0))
//                                     .pl(px(8.0 + 16.0 * entry.depth() as f32))
//                                     .pr_2()
//                                     .py_0()
//                                     .text_sm()
//                                     .line_height(px(18.0))
//                                     .text_color(rgb(pal.ink))
//                                     .selected(tree_selected || item_selection == selected)
//                                     .on_click(move |_, _, cx| {
//                                         if let Some(sel) = item_selection {
//                                             app.update(cx, |view, cx| {
//                                                 // Toggle: click again to deselect
//                                                 if view.result_tree_selection == Some(sel) {
//                                                     view.result_tree_selection = None;
//                                                 } else {
//                                                     view.result_tree_selection = Some(sel);
//                                                 }
//                                                 view.invalidate_detail_table();
//                                                 cx.notify();
//                                             })
//                                             .ok();
//                                         }
//                                     })
//                                     .child(
//                                         div()
//                                             .w_full()
//                                             .truncate()
//                                             .whitespace_nowrap()
//                                             .flex()
//                                             .items_center()
//                                             .child(
//                                                 div()
//                                                     .text_xs()
//                                                     .w(px(16.0))
//                                                     .flex_shrink_0()
//                                                     .text_color(rgb(pal.muted))
//                                                     .child(chevron),
//                                             )
//                                             .child(entry.item().label.clone()),
//                                     )
//                             },
//                         )),
//                 );

//             let detail_pane = div()
//                 .size_full()
//                 .flex()
//                 .flex_col()
//                 .child(
//                     div()
//                         .px_3()
//                         .py_2()
//                         .border_b_1()
//                         .border_color(rgb(pal.border))
//                         .text_sm()
//                         .font_weight(FontWeight::SEMIBOLD)
//                         .child(detail_title),
//                 )
//                 .child(
//                     div()
//                         .id("detail-table-area")
//                         .flex_1()
//                         .min_h_0()
//                         .overflow_hidden()
//                         .bg(rgb(pal.panel))
//                         .child(
//                             Table::new(table_state)
//                                 .stripe(false)
//                                 .bordered(true)
//                                 .scrollbar_visible(true, true),
//                         ),
//                 );

//             div().flex_1().min_h_0().flex().child(
//                 h_resizable("results-explorer")
//                     .child(
//                         resizable_panel()
//                             .size(tree_width)
//                             .size_range(px(200.0)..px(600.0))
//                             .child(tree_pane),
//                     )
//                     .child(resizable_panel().child(detail_pane)),
//             )
//         }

//         fn selected_detail_rows(&self, analysis: &AnalysisRun) -> Vec<Vec<String>> {
//             let selected_indices = match self.result_tree_selection {
//                 Some(ResultTreeSelection::Cluster(cluster_id)) => analysis
//                     .clusters
//                     .iter()
//                     .find(|cluster| cluster.id.0 == cluster_id)
//                     .map(|cluster| cluster.incident_row_indices.as_slice()),
//                 Some(ResultTreeSelection::Theme { cluster, theme }) => analysis
//                     .clusters
//                     .iter()
//                     .find(|item| item.id.0 == cluster)
//                     .and_then(|cluster| {
//                         cluster
//                             .subgroups
//                             .iter()
//                             .find(|subgroup| subgroup.id == theme)
//                             .map(|subgroup| subgroup.incident_row_indices.as_slice())
//                     }),
//                 None => None,
//             };

//             match selected_indices {
//                 Some(indices) => self
//                     .result_filters
//                     .filtered_row_indices(&analysis.source.rows, indices)
//                     .into_iter()
//                     .filter_map(|row_index| analysis.source.rows.get(row_index).cloned())
//                     .take(500)
//                     .collect(),
//                 None => analysis
//                     .processed_incidents
//                     .iter()
//                     .filter_map(|record| analysis.source.rows.get(record.source_row_index))
//                     .filter(|row| self.result_filters.matches_row(row))
//                     .take(500)
//                     .cloned()
//                     .collect(),
//             }
//         }

//         fn detail_table_title(&self) -> String {
//             match self.result_tree_selection {
//                 Some(ResultTreeSelection::Cluster(cluster)) => {
//                     format!("Cluster C{cluster:04}")
//                 }
//                 Some(ResultTreeSelection::Theme { cluster, theme }) => {
//                     format!("C{cluster:04} > Theme {theme}")
//                 }
//                 None => "All incidents".to_owned(),
//             }
//         }

//         // -- shared helpers -----------------------------------------------

//         fn step_state(&self, screen: Screen) -> &'static str {
//             match screen {
//                 Screen::Import if self.source.is_some() => "done",
//                 Screen::Mapping if self.mapping_ready() => "done",
//                 Screen::Run if self.worker.is_some() => "running",
//                 Screen::Run if self.analysis.is_some() => "done",
//                 Screen::Results if self.analysis.is_some() => "ready",
//                 _ => "pending",
//             }
//         }

//         fn status_line(&self) -> String {
//             if let Some(started_at) = self.run_started_at {
//                 format!(
//                     "{}    Elapsed: {}",
//                     self.status,
//                     format_duration(started_at.elapsed())
//                 )
//             } else if let Some(elapsed) = self.last_run_elapsed {
//                 format!("{}    Last run: {}", self.status, format_duration(elapsed))
//             } else {
//                 self.status.clone()
//             }
//         }

//         fn filtered_processed_count(&self, analysis: &AnalysisRun) -> usize {
//             analysis
//                 .processed_incidents
//                 .iter()
//                 .filter(|record| {
//                     analysis
//                         .source
//                         .rows
//                         .get(record.source_row_index)
//                         .map(|row| self.result_filters.matches_row(row))
//                         .unwrap_or(false)
//                 })
//                 .count()
//         }

//         fn result_filter_options(&self, analysis: &AnalysisRun) -> Vec<BTreeSet<String>> {
//             let mut values = vec![BTreeSet::new(); analysis.source.headers.len()];

//             for record in &analysis.processed_incidents {
//                 let Some(row) = analysis.source.rows.get(record.source_row_index) else {
//                     continue;
//                 };

//                 for (column, column_values) in values.iter_mut().enumerate() {
//                     column_values.insert(row.get(column).cloned().unwrap_or_default());
//                 }
//             }

//             values
//         }

//         fn mapping_ready(&self) -> bool {
//             self.mapping.incident_number.is_some() && self.mapping.short_description.is_some()
//         }

//         fn required_mapping_status(&self) -> String {
//             if self.mapping_ready() {
//                 "2 of 2".to_owned()
//             } else {
//                 let mapped = [self.mapping.incident_number, self.mapping.short_description]
//                     .into_iter()
//                     .flatten()
//                     .count();
//                 format!("{mapped} of 2")
//             }
//         }

//         fn filter_mapping_count(&self) -> usize {
//             [
//                 self.mapping.assignment_group,
//                 self.mapping.service,
//                 self.mapping.category,
//                 self.mapping.configuration_item,
//                 self.mapping.date,
//             ]
//             .into_iter()
//             .flatten()
//             .count()
//         }

//         fn mapped_field_count(&self) -> usize {
//             self.filter_mapping_count()
//                 + self.mapping.additional_text.len()
//                 + [self.mapping.incident_number, self.mapping.short_description]
//                     .into_iter()
//                     .flatten()
//                     .count()
//         }

//         // -- business logic (unchanged) -----------------------------------

//         fn open_source_file(&mut self) {
//             let Some(path) = rfd::FileDialog::new()
//                 .add_filter("Incident exports", &["csv", "xlsx", "xlsm", "xls"])
//                 .pick_file()
//             else {
//                 return;
//             };

//             self.worksheets.clear();
//             self.selected_worksheet = None;

//             if is_excel(&path) {
//                 match list_worksheets(&path) {
//                     Ok(worksheets) => {
//                         self.worksheets = worksheets;
//                         self.selected_worksheet = self.worksheets.first().cloned();
//                         if let Some(sheet) = self.selected_worksheet.clone() {
//                             match import_xlsx_sheet(&path, &sheet) {
//                                 Ok(source) => self.accept_source(source),
//                                 Err(err) => self.status = err.to_string(),
//                             }
//                         }
//                     }
//                     Err(err) => self.status = err.to_string(),
//                 }
//             } else {
//                 match import_source(&path) {
//                     Ok(source) => self.accept_source(source),
//                     Err(err) => self.status = err.to_string(),
//                 }
//             }
//         }

//         fn open_selected_worksheet(&mut self) {
//             let Some(source_path) = self
//                 .source
//                 .as_ref()
//                 .and_then(|source| source.source_path.clone())
//                 .or_else(|| self.last_excel_path())
//             else {
//                 return;
//             };
//             let Some(sheet) = self.selected_worksheet.clone() else {
//                 return;
//             };

//             match import_xlsx_sheet(&source_path, &sheet) {
//                 Ok(source) => self.accept_source(source),
//                 Err(err) => self.status = err.to_string(),
//             }
//         }

//         fn last_excel_path(&self) -> Option<PathBuf> {
//             self.source
//                 .as_ref()
//                 .and_then(|source| source.source_path.clone())
//         }

//         fn accept_source(&mut self, source: SourceTable) {
//             self.mapping = suggest_mapping(&source.headers);
//             self.status = format!(
//                 "Loaded {} rows from {}.",
//                 source.row_count(),
//                 source
//                     .source_path
//                     .as_ref()
//                     .map(|path| path.display().to_string())
//                     .unwrap_or_else(|| "source".to_owned())
//             );
//             self.source = Some(source);
//             self.invalidate_tree();
//             self.screen = Screen::Mapping;
//         }

//         fn start_analysis(&mut self) {
//             let Some(source) = self.source.clone() else {
//                 self.status = "Load a source file first.".to_owned();
//                 return;
//             };
//             if let Err(err) = validate_mapping(&self.mapping, &source) {
//                 self.status = err.to_string();
//                 return;
//             }
//             self.worker = Some(spawn_analysis(
//                 source,
//                 self.mapping.clone(),
//                 self.settings.clone(),
//             ));
//             self.current_progress = None;
//             self.progress_log.clear();
//             self.run_started_at = Some(Instant::now());
//             self.last_run_elapsed = None;
//             self.invalidate_tree();
//             self.status = "Started clustering analysis.".to_owned();
//         }

//         fn poll_worker(&mut self, window: &mut Window) {
//             let Some(worker) = self.worker.take() else {
//                 return;
//             };

//             let mut keep_worker = true;
//             while let Ok(message) = worker.try_recv() {
//                 match message {
//                     WorkerMessage::Started => {
//                         self.status = "Analysis worker started.".to_owned();
//                     }
//                     WorkerMessage::Progress(progress) => {
//                         self.status = format!("{}: {}", progress.stage, progress.detail);
//                         self.current_progress = Some(progress.clone());
//                         self.progress_log.push(ProgressLogEntry {
//                             elapsed: self
//                                 .run_started_at
//                                 .map(|started_at| started_at.elapsed())
//                                 .unwrap_or_default(),
//                             update: progress,
//                         });
//                         if self.progress_log.len() > 40 {
//                             self.progress_log.remove(0);
//                         }
//                     }
//                     WorkerMessage::Finished(Ok(run)) => {
//                         let elapsed = self.finish_run_timer();
//                         self.status = format!(
//                             "Analysis complete in {}: {} clusters, {} ignored rows.",
//                             format_duration(elapsed),
//                             run.clusters.len(),
//                             run.ignored_rows.len()
//                         );
//                         self.result_filters.clear();
//                         self.result_tree_selection = None;
//                         self.invalidate_tree();
//                         self.analysis = Some(*run);
//                         self.current_progress = None;
//                         self.screen = Screen::Results;
//                         keep_worker = false;
//                     }
//                     WorkerMessage::Finished(Err(err)) => {
//                         let elapsed = self.finish_run_timer();
//                         self.status = format!(
//                             "Analysis failed after {}: {}",
//                             format_duration(elapsed),
//                             err
//                         );
//                         self.current_progress = None;
//                         keep_worker = false;
//                     }
//                 }
//             }

//             if keep_worker {
//                 self.worker = Some(worker);
//                 window.request_animation_frame();
//             }
//         }

//         fn export_results(&mut self) {
//             let Some(analysis) = &self.analysis else {
//                 return;
//             };
//             let Some(path) = rfd::FileDialog::new()
//                 .add_filter("Excel workbook", &["xlsx"])
//                 .set_file_name("clustered_incidents.xlsx")
//                 .save_file()
//             else {
//                 return;
//             };

//             match export_analysis(analysis, &path) {
//                 Ok(()) => self.status = format!("Exported {}", path.display()),
//                 Err(err) => self.status = err.to_string(),
//             }
//         }

//         fn save_mapping(&mut self) {
//             let Some(path) = rfd::FileDialog::new()
//                 .add_filter("Mapping profile", &["json"])
//                 .set_file_name("incident_mapping.json")
//                 .save_file()
//             else {
//                 return;
//             };
//             match save_mapping_profile(&path, &self.mapping) {
//                 Ok(()) => self.status = format!("Saved mapping {}", path.display()),
//                 Err(err) => self.status = err.to_string(),
//             }
//         }

//         fn load_mapping(&mut self) {
//             let Some(path) = rfd::FileDialog::new()
//                 .add_filter("Mapping profile", &["json"])
//                 .pick_file()
//             else {
//                 return;
//             };
//             match load_mapping_profile(&path) {
//                 Ok(mapping) => {
//                     self.mapping = mapping;
//                     self.status = format!("Loaded mapping {}", path.display());
//                 }
//                 Err(err) => self.status = err.to_string(),
//             }
//         }

//         fn save_session(&mut self) {
//             let Some(analysis) = &self.analysis else {
//                 return;
//             };
//             let Some(path) = rfd::FileDialog::new()
//                 .add_filter("Analysis session", &["json"])
//                 .set_file_name("incident_analysis_session.json")
//                 .save_file()
//             else {
//                 return;
//             };
//             match save_analysis_session(&path, analysis) {
//                 Ok(()) => self.status = format!("Saved session {}", path.display()),
//                 Err(err) => self.status = err.to_string(),
//             }
//         }

//         fn load_session(&mut self) {
//             let Some(path) = rfd::FileDialog::new()
//                 .add_filter("Analysis session", &["json"])
//                 .pick_file()
//             else {
//                 return;
//             };
//             match load_analysis_session(&path) {
//                 Ok(run) => {
//                     self.source = Some(run.source.clone());
//                     self.mapping = run.mapping.clone();
//                     self.settings = run.settings.clone();
//                     self.result_filters.clear();
//                     self.result_tree_selection = None;
//                     self.invalidate_tree();
//                     self.analysis = Some(run);
//                     self.screen = Screen::Results;
//                     self.status = format!("Loaded session {}", path.display());
//                 }
//                 Err(err) => self.status = err.to_string(),
//             }
//         }

//         fn finish_run_timer(&mut self) -> Duration {
//             let elapsed = self
//                 .run_started_at
//                 .take()
//                 .map(|started_at| started_at.elapsed())
//                 .or(self.last_run_elapsed)
//                 .unwrap_or_default();
//             self.last_run_elapsed = Some(elapsed);
//             elapsed
//         }
//     }

//     // ── Free functions & helpers ─────────────────────────────────────────

//     /// Builds tree items from cluster data. Free function so that
//     /// `rebuild_tree_state` can use split borrows on `IncidentClusteringApp`.
//     fn build_cluster_tree_items(
//         clusters: &[crate::model::Cluster],
//         source_rows: &[Vec<String>],
//         result_filters: &ResultFilters,
//     ) -> Vec<TreeItem> {
//         clusters
//             .iter()
//             .filter_map(|cluster| {
//                 let filtered_cluster_rows =
//                     result_filters.filtered_row_indices(source_rows, &cluster.incident_row_indices);
//                 if filtered_cluster_rows.is_empty() {
//                     return None;
//                 }

//                 let children: Vec<TreeItem> = cluster
//                     .subgroups
//                     .iter()
//                     .filter_map(|subgroup| {
//                         let filtered_theme_rows = result_filters
//                             .filtered_row_indices(source_rows, &subgroup.incident_row_indices);
//                         (!filtered_theme_rows.is_empty()).then(|| {
//                             TreeItem::new(
//                                 format!("cluster-{}-theme-{}", cluster.id.0, subgroup.id),
//                                 format!(
//                                     "Theme {} - {} ({})",
//                                     subgroup.id,
//                                     subgroup.label,
//                                     filtered_theme_rows.len()
//                                 ),
//                             )
//                         })
//                     })
//                     .collect();

//                 Some(
//                     TreeItem::new(
//                         format!("cluster-{}", cluster.id.0),
//                         format!(
//                             "{} - {} ({})",
//                             cluster.id,
//                             cluster.label,
//                             filtered_cluster_rows.len()
//                         ),
//                     )
//                     .expanded(false)
//                     .children(children),
//                 )
//             })
//             .collect()
//     }

//     fn action_button<F>(
//         cx: &mut Context<IncidentClusteringApp>,
//         id: impl Into<SharedString>,
//         label: impl Into<SharedString>,
//         enabled: bool,
//         action: F,
//         pal: &Palette,
//     ) -> impl IntoElement
//     where
//         F: Fn(&mut IncidentClusteringApp, &mut Context<IncidentClusteringApp>) + 'static,
//     {
//         let mut button = button_base(id, label, false, enabled, pal);
//         if enabled {
//             button = button.on_click(cx.listener(move |view, _, _window, cx| {
//                 action(view, cx);
//             }));
//         }
//         button
//     }

//     fn primary_action_button<F>(
//         cx: &mut Context<IncidentClusteringApp>,
//         id: impl Into<SharedString>,
//         label: impl Into<SharedString>,
//         enabled: bool,
//         action: F,
//         pal: &Palette,
//     ) -> impl IntoElement
//     where
//         F: Fn(&mut IncidentClusteringApp, &mut Context<IncidentClusteringApp>) + 'static,
//     {
//         let mut button = div()
//             .id(id.into())
//             .px_4()
//             .py_2()
//             .rounded_md()
//             .border_1()
//             .border_color(rgb(pal.accent_dark))
//             .bg(if enabled {
//                 rgb(pal.accent)
//             } else {
//                 rgb(pal.disabled_bg)
//             })
//             .text_color(if enabled {
//                 rgb(pal.accent_text)
//             } else {
//                 rgb(pal.muted)
//             })
//             .text_sm()
//             .font_weight(FontWeight::BOLD)
//             .cursor_pointer()
//             .child(label.into());

//         if enabled {
//             button = button.on_click(cx.listener(move |view, _, _window, cx| {
//                 action(view, cx);
//             }));
//         }

//         button
//     }

//     fn button_base(
//         id: impl Into<SharedString>,
//         label: impl Into<SharedString>,
//         selected: bool,
//         enabled: bool,
//         pal: &Palette,
//     ) -> Stateful<Div> {
//         let id = id.into();
//         let bg = if selected {
//             rgb(pal.accent)
//         } else if enabled {
//             rgb(pal.button_bg)
//         } else {
//             rgb(pal.disabled_bg)
//         };
//         let color = if selected {
//             rgb(pal.accent_text)
//         } else {
//             rgb(pal.ink)
//         };

//         div()
//             .id(id)
//             .px_2()
//             .py_1()
//             .rounded_md()
//             .border_1()
//             .border_color(if selected {
//                 rgb(pal.accent_dark)
//             } else {
//                 rgb(pal.border)
//             })
//             .bg(bg)
//             .text_color(color)
//             .text_sm()
//             .cursor_pointer()
//             .child(label.into())
//     }

//     fn split_dropdown_button(
//         id: impl Into<SharedString>,
//         label: impl Into<SharedString>,
//         active: bool,
//         pal: &Palette,
//     ) -> Button {
//         let id = id.into();
//         let label = label.into();

//         let (bg_color, border_color, text_color) = if active {
//             (rgb(pal.active_bg), rgb(pal.accent), rgb(pal.accent))
//         } else {
//             (rgb(pal.button_bg), rgb(pal.border), rgb(pal.ink))
//         };

//         Button::new(id).child(
//             div()
//                 .h(px(28.0))
//                 .flex()
//                 .items_center()
//                 .rounded_md()
//                 .border_1()
//                 .border_color(border_color)
//                 .overflow_hidden()
//                 .bg(bg_color)
//                 .child(
//                     div()
//                         .px_2()
//                         .min_w(px(86.0))
//                         .text_sm()
//                         .text_color(text_color)
//                         .font_weight(if active {
//                             FontWeight::SEMIBOLD
//                         } else {
//                             FontWeight::NORMAL
//                         })
//                         .child(label),
//                 )
//                 .child(
//                     div()
//                         .h_full()
//                         .px_2()
//                         .flex()
//                         .items_center()
//                         .border_l_1()
//                         .border_color(rgb(pal.accent_dark))
//                         .bg(rgb(pal.accent))
//                         .text_color(rgb(pal.accent_text))
//                         .font_weight(FontWeight::BOLD)
//                         .child("v"),
//                 ),
//         )
//     }

//     // ── Layout primitives ────────────────────────────────────────────────

//     fn screen(title: impl Into<SharedString>) -> Div {
//         div().flex().flex_col().gap_3().child(
//             div()
//                 .text_xl()
//                 .font_weight(FontWeight::BOLD)
//                 .child(title.into()),
//         )
//     }

//     /// Bordered container — used only for the progress bar section.
//     fn container(pal: &Palette) -> Div {
//         div()
//             .bg(rgb(pal.panel))
//             .border_1()
//             .border_color(rgb(pal.border))
//             .p_3()
//             .flex()
//             .flex_col()
//             .gap_2()
//     }

//     /// Subtle section heading with a bottom border.
//     fn section_label(title: impl Into<SharedString>, pal: &Palette) -> impl IntoElement {
//         div()
//             .pt_2()
//             .pb_1()
//             .border_b_1()
//             .border_color(rgb(pal.border))
//             .text_sm()
//             .font_weight(FontWeight::SEMIBOLD)
//             .text_color(rgb(pal.muted))
//             .child(title.into())
//     }

//     /// Compact inline stat strip: `Label: Value  Label: Value  ...`
//     fn inline_stats(items: Vec<(&'static str, String)>, pal: &Palette) -> impl IntoElement {
//         div()
//             .flex()
//             .flex_wrap()
//             .items_center()
//             .gap_x_4()
//             .gap_y_1()
//             .children(items.into_iter().map(|(label, value)| {
//                 div()
//                     .flex()
//                     .items_center()
//                     .gap_1()
//                     .child(
//                         div()
//                             .text_sm()
//                             .text_color(rgb(pal.muted))
//                             .child(format!("{label}:")),
//                     )
//                     .child(
//                         div()
//                             .text_sm()
//                             .font_weight(FontWeight::SEMIBOLD)
//                             .child(value),
//                     )
//             }))
//     }

//     // ── Data table ───────────────────────────────────────────────────────

//     struct GridTableDelegate {
//         columns: Vec<Column>,
//         rows: Vec<Vec<String>>,
//         pal: Palette,
//     }

//     impl GridTableDelegate {
//         fn new(headers: Vec<String>, rows: Vec<Vec<String>>, pal: Palette) -> Self {
//             let columns = headers
//                 .into_iter()
//                 .enumerate()
//                 .map(|(index, header)| {
//                     Column::new(format!("col-{index}"), header)
//                         .width(px(190.0))
//                         .resizable(true)
//                         .movable(true)
//                 })
//                 .collect();
//             Self { columns, rows, pal }
//         }
//     }

//     impl TableDelegate for GridTableDelegate {
//         fn columns_count(&self, _cx: &App) -> usize {
//             self.columns.len()
//         }

//         fn rows_count(&self, _cx: &App) -> usize {
//             self.rows.len()
//         }

//         fn column(&self, col_ix: usize, _cx: &App) -> &Column {
//             &self.columns[col_ix]
//         }

//         fn render_header(
//             &mut self,
//             _window: &mut Window,
//             _cx: &mut Context<TableState<Self>>,
//         ) -> Stateful<Div> {
//             div()
//                 .id("header")
//                 .bg(rgb(self.pal.table_header))
//                 .text_color(rgb(self.pal.ink))
//                 .font_weight(FontWeight::BOLD)
//         }

//         fn render_th(
//             &mut self,
//             col_ix: usize,
//             _window: &mut Window,
//             cx: &mut Context<TableState<Self>>,
//         ) -> impl IntoElement {
//             div()
//                 .size_full()
//                 .px_2()
//                 .py_1()
//                 .text_sm()
//                 .text_color(rgb(self.pal.ink))
//                 .bg(rgb(self.pal.table_header))
//                 .child(truncate(&self.column(col_ix, cx).name, 80))
//         }

//         fn render_tr(
//             &mut self,
//             row_ix: usize,
//             _window: &mut Window,
//             _cx: &mut Context<TableState<Self>>,
//         ) -> Stateful<Div> {
//             div()
//                 .id(("row", row_ix))
//                 .bg(rgb(if row_ix.is_multiple_of(2) {
//                     self.pal.panel
//                 } else {
//                     self.pal.alt_row
//                 }))
//                 .text_color(rgb(self.pal.ink))
//         }

//         fn render_td(
//             &mut self,
//             row_ix: usize,
//             col_ix: usize,
//             _window: &mut Window,
//             _cx: &mut Context<TableState<Self>>,
//         ) -> impl IntoElement {
//             div()
//                 .size_full()
//                 .px_2()
//                 .py_1()
//                 .text_sm()
//                 .text_color(rgb(self.pal.ink))
//                 .child(truncate(
//                     self.rows
//                         .get(row_ix)
//                         .and_then(|row| row.get(col_ix))
//                         .map(String::as_str)
//                         .unwrap_or_default(),
//                     96,
//                 ))
//         }
//     }

//     fn data_table(
//         headers: Vec<String>,
//         rows: Vec<Vec<String>>,
//         window: &mut Window,
//         cx: &mut Context<IncidentClusteringApp>,
//         pal: &Palette,
//     ) -> impl IntoElement {
//         let table_pal = *pal;
//         let state = cx.new(|cx| {
//             TableState::new(GridTableDelegate::new(headers, rows, table_pal), window, cx)
//         });
//         div()
//             .size_full()
//             .min_w(px(520.0))
//             .bg(rgb(pal.panel))
//             .overflow_hidden()
//             .child(
//                 Table::new(&state)
//                     .stripe(false)
//                     .bordered(true)
//                     .scrollbar_visible(true, true),
//             )
//     }

//     // ── Utilities ────────────────────────────────────────────────────────

//     fn selection_from_tree_id(id: &str) -> Option<ResultTreeSelection> {
//         let tail = id.strip_prefix("cluster-")?;
//         if let Some((cluster, theme)) = tail.split_once("-theme-") {
//             return Some(ResultTreeSelection::Theme {
//                 cluster: cluster.parse().ok()?,
//                 theme: theme.parse().ok()?,
//             });
//         }

//         Some(ResultTreeSelection::Cluster(tail.parse().ok()?))
//     }

//     fn progress_bar(fraction: f32, pal: &Palette) -> impl IntoElement {
//         let pct = (fraction.clamp(0.0, 1.0) * 100.0).round();
//         div()
//             .w_full()
//             .h(px(22.0))
//             .rounded_md()
//             .bg(rgb(pal.progress_track))
//             .overflow_hidden()
//             .child(
//                 div()
//                     .h_full()
//                     .w(px((pct * 4.0).max(6.0)))
//                     .bg(rgb(pal.accent))
//                     .child(
//                         div()
//                             .px_2()
//                             .text_xs()
//                             .text_color(rgb(pal.accent_text))
//                             .child(format!("{pct:.0}%")),
//                     ),
//             )
//     }

//     fn truncate(value: &str, max_chars: usize) -> String {
//         if value.chars().count() <= max_chars {
//             return value.to_owned();
//         }
//         let mut truncated = value
//             .chars()
//             .take(max_chars.saturating_sub(1))
//             .collect::<String>();
//         truncated.push_str("...");
//         truncated
//     }

//     fn is_excel(path: &Path) -> bool {
//         matches!(
//             path.extension()
//                 .and_then(|extension| extension.to_str())
//                 .map(str::to_ascii_lowercase)
//                 .as_deref(),
//             Some("xlsx" | "xlsm" | "xls")
//         )
//     }

//     fn format_duration(duration: Duration) -> String {
//         let total_seconds = duration.as_secs();
//         let minutes = total_seconds / 60;
//         let seconds = total_seconds % 60;
//         let millis = duration.subsec_millis();

//         if minutes > 0 {
//             format!("{minutes}m {seconds:02}s")
//         } else if seconds > 0 {
//             format!("{seconds}.{millis:03}s")
//         } else {
//             format!("{millis}ms")
//         }
//     }
// }

// pub use gpui_app::IncidentClusteringApp;
