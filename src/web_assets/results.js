import {editColumnTypes} from "./columns.js";
import { reviewerRows, reviewerLabel, bindReviewerFilter } from "./reviewers.js";
import { captureView, restoreView, labelMatches, clearResultsFilters } from "./view-state.js";
import { bindWorkflow, openWorkflow, renderWorkflow, renderSaveStatus, applyReviewResponse, effectiveWorkflow, selectionKey, statusLabels, stateMatches, workflowRows, displayLabel } from "./workflow.js";
import { exportClusterViewExcel, exportExcel, exportPivotExcel, fetchPivot, fetchResult, fetchRows, restoreSession, fetchReview, saveSession } from "./api.js";
import { state } from "./state.js";
import { hideOverlay, setStatus, showBusy, showError, showStep, statsHtml } from "./ui.js";
import { clusterId, clusterKey, escapeHtml, sameSelection } from "./utils.js";

export function bindResultsEvents() {
  document.getElementById("editColumnTypes").onclick=()=>editColumnTypes(snapshot=>{applyReviewResponse(snapshot);renderResults();});
  document.getElementById("clearAllFilters").onclick=()=>{clearResultsFilters(state);renderResults();};
  document.getElementById("treeLabelFilter").oninput=e=>{state.treeLabelFilter=e.target.value;renderClusterList();};
  const top=document.getElementById("detailScrollTop"),table=document.getElementById("detailTable");
  top.onscroll=()=>{if(table.scrollLeft!==top.scrollLeft)table.scrollLeft=top.scrollLeft;};
  table.onscroll=()=>{if(top.scrollLeft!==table.scrollLeft)top.scrollLeft=table.scrollLeft;};
  const resize=new ResizeObserver(()=>syncDetailScrollbar());resize.observe(table);
  window.addEventListener("resize",syncDetailScrollbar);
  bindReviewerFilter(() => renderResults());
  bindWorkflow(() => renderResults());
  async function saving(title, action) {
    showBusy(title, "Preparing your download. Large sessions may take a while.");
    try { await action(); setStatus("Download prepared."); hideOverlay(); }
    catch (error) { showError(title, error.message); }
  }
  document.getElementById("downloadSession").addEventListener("click", () => {
    if (!state.analysis) return;
    const revision = state.review.annotations.revision;
    saving("Saving session", async () => {
      await saveSession(state.jobId, captureView(state));
      state.savedReviewRevision = revision; renderSaveStatus();
    });
  });
  document.getElementById("exportExcel").addEventListener("click", () => {
    if (state.jobId) saving("Exporting incidents", async () => { await ensureRowData(); await exportExcel(state.jobId, visibleDetailRowIndices()); });
  });

  document.getElementById("exportClusterView").addEventListener("click", async () => {
    if (!state.jobId || !state.analysis) return;
    showBusy("Exporting cluster view", "Preparing the cluster tree Excel export.");
    try {
      await ensureRowData();
      await exportClusterViewExcel(state.jobId, {
        drilldownRowIndices: treeVisibleRows() ? [...treeVisibleRows()] : null,
        workflowStates: state.workflowStates,
      });
      setStatus("Cluster view export prepared.");
      setTimeout(hideOverlay, 600);
    } catch (error) {
      setStatus(error.message, true);
      showError("Cluster view export failed", error.message);
    }
  });

  document.getElementById("exportPivotExcel").addEventListener("click", async () => {
    if (!state.jobId || !state.analysis) return;
    if (!state.pivotRows.length && !state.pivotColumns.length) {
      showError("Pivot export failed", "Add at least one field to Rows or Columns before exporting.");
      return;
    }
    showBusy("Exporting pivot", "Preparing the pivot Excel export.");
    try {
      await ensureRowData();
      await exportPivotExcel(state.jobId, visibleDetailRowIndices(), state.pivotRows, state.pivotColumns);
      setStatus("Pivot export prepared.");
      setTimeout(hideOverlay, 600);
    } catch (error) {
      setStatus(error.message, true);
      showError("Pivot export failed", error.message);
    }
  });

  document.getElementById("clearDrilldown").addEventListener("click", () => {
    clearDetailDrilldown();
  });

  document.getElementById("showPivotRecords").addEventListener("click", () => {
    applySelectedPivotDrilldown();
  });

  document.getElementById("pageSize").addEventListener("change", (event) => {
    state.detailPageSize = Number(event.target.value);
    state.detailPage = 1;
    renderDetailRows();
  });

  document.getElementById("previousPage").addEventListener("click", () => {
    if (state.detailPage > 1) {
      state.detailPage -= 1;
      renderDetailRows();
    }
  });

  document.getElementById("nextPage").addEventListener("click", () => {
    const totalPages = Math.max(1, Math.ceil(visibleDetailRowIndices().length / state.detailPageSize));
    if (state.detailPage < totalPages) {
      state.detailPage += 1;
      renderDetailRows();
    }
  });

  document.querySelector('[data-step="pivot"]').addEventListener("click", () => renderPivot());
  bindPivotDropZones();

  document.addEventListener("click", (event) => {
    if (!event.target.closest(".pivot-context-menu")) hidePivotContextMenu();
    if (state.detailOpenFilterColumn == null || event.target.closest(".column-filter-menu, .column-filter-button")) return;
    state.detailOpenFilterColumn = null;
    renderDetailRows();
  });

  window.addEventListener("resize", () => applyResultsPaneWidth());
}

let loadGeneration = 0, navigationEpoch = 0;
if(typeof window!=="undefined")window.addEventListener("pane-change",()=>navigationEpoch++);
export async function loadResult(jobId) {
  const generation = ++loadGeneration, epoch = navigationEpoch;
  state.loadingAnalysis=true;
  try {
  if (jobId.startsWith("job-")) state.analysisId = null;
  const [run, metadata] = await Promise.all([fetchResult(jobId), fetchReview(jobId)]);
  if(generation !== loadGeneration || epoch !== navigationEpoch) return false;
  initializeResult(run, jobId, metadata);
  // A restored view may filter or sort: have those rows before anything reads the view.
  await ensureRowData();
  if(generation !== loadGeneration || epoch !== navigationEpoch) return false;
  renderResults(); showStep("results");
  return true;
  } finally {if(generation === loadGeneration) state.loadingAnalysis=false;}
}
export async function loadSavedSession(file) {
  const restored = await restoreSession(file);
  await loadResult(restored.jobId);
}

function initializeResult(run, jobId, metadata) {
  normalizeRunSettings(run);
  state.analysis = run;
  state.rowData = emptyRowData();
  state.source = null;
  state.mapping = run.mapping || null;
  state.settings = run.settings || state.settings;
  state.jobId = jobId;
  state.selection = { type: "all" };
  state.expandedClusters = new Set();
  state.workflowStates = [];
  state.reviewerFilter = {};
  applyReviewResponse(metadata);
  state.savedReviewRevision = state.review.annotations.revision;
  state.detailPage = 1;
  state.detailColumnWidths = [];
  state.detailColumnOrder = [];
  state.detailColumnFilters = [];
  state.detailOpenFilterColumn = null;
  state.detailSort = null;
  state.detailDrilldownRowIndices = null;
  state.detailDrilldownLabel = "";
  state.pivotRows = [];
  state.pivotColumns = [];
  state.currentPivotRows = [];
  state.selectedPivotRowIndices = null;
  state.selectedPivotRowLabel = "";
  restoreView(state, metadata.view);
  document.querySelector('[data-step="mapping"]').disabled = true;
  document.querySelector('[data-step="analysis"]').disabled = true;
  document.querySelector('[data-step="results"]').disabled = false;
  document.querySelector('[data-step="pivot"]').disabled = false;
}

function normalizeRunSettings(run) {
  if (!run.settings) run.settings = {};
  if (!run.settings.label_terms) run.settings.label_terms = { boosted: [], suppressed: [], excluded: [] };
  if (!run.settings.label_terms.boosted) run.settings.label_terms.boosted = [];
  if (!run.settings.label_terms.suppressed) run.settings.label_terms.suppressed = [];
  if (!run.settings.label_terms.excluded) run.settings.label_terms.excluded = [];
}

export function renderResults(preserveEditor = false) {
  const run = state.analysis;
  applyResultsPaneWidth();
  document.getElementById("resultStats").innerHTML = statsHtml([
    ["Clusters", run.clusters.filter(c=>c.id!==0).length],
    ["Processed", run.processed_row_indices.length],
    ["Ignored", run.ignored_rows.length],
    ["Unclustered", run.unclustered_row_indices.length],
  ]);
  window.dispatchEvent(new Event("reviewer-render"));
  renderClusterList();
  renderDetailRows();
  if (!preserveEditor) renderWorkflow(() => renderResults());
}

function syncDetailScrollbar(){
  const table=document.getElementById("detailTable"),top=document.getElementById("detailScrollTop");
  if(!table||!top)return;top.firstElementChild.style.width=`${table.scrollWidth}px`;
  top.hidden=table.scrollWidth<=table.clientWidth;top.scrollLeft=table.scrollLeft;
}
function renderClusterList() {
  if (rowDataReady()) return drawClusterList();
  const run = state.analysis;
  ensureRowData().then(ready => { if (ready && state.analysis === run) drawClusterList(); }, reportRowError);
}

function drawClusterList() {
  document.getElementById("treeLabelFilter").value=state.treeLabelFilter||"";
  const list = document.getElementById("clusterList");
  const visibleRows = treeVisibleRows();
  list.innerHTML = "";
  addClusterButton(list, visibleRows ? `All incidents (${visibleRows.size})` : "All incidents", { type: "all" });
  state.analysis.clusters.forEach((cluster) => {
    const clusterCount = filteredIncidentCount(cluster.incident_row_indices, visibleRows);
    if (visibleRows && clusterCount === 0) return;
    const query=state.treeLabelFilter||"";
    const parentMatches=labelMatches(displayLabel(state.review,String(cluster.id),cluster.label),query);
    const themeMatches=theme=>labelMatches(displayLabel(state.review,`${cluster.id}:${theme.id}`,theme.label),query);
    if(!parentMatches&&!cluster.subgroups.some(themeMatches))return;
    const key = clusterKey(cluster.id);
    const expanded = state.expandedClusters.has(key)||!!query;
    const row = document.createElement("div");
    row.className = "tree-row";

    const toggle = document.createElement("button");
    toggle.className = "tree-toggle";
    toggle.type = "button";
    toggle.textContent = expanded ? "v" : ">";
    toggle.setAttribute("aria-label", expanded ? "Collapse cluster" : "Expand cluster");
    toggle.setAttribute("aria-expanded", String(expanded));
    toggle.disabled = cluster.subgroups.length === 0;
    toggle.addEventListener("click", () => {
      if (expanded) {
        state.expandedClusters.delete(key);
      } else {
        state.expandedClusters.add(key);
      }
      renderClusterList();
    });

    row.appendChild(toggle);
    const clusterSelection = { type: "cluster", cluster: cluster.id };
    addClusterButton(
      row,
      `${clusterId(cluster.id)} - ${displayLabel(state.review, String(cluster.id), cluster.label)} (${clusterCount})`,
      clusterSelection,
      "cluster-label"
    );
    row.appendChild(workflowBadge(clusterSelection));
    row.appendChild(reviewerLabel(cluster.id));
    if (!stateMatches(state.review, String(cluster.id), state.workflowStates)) row.classList.add("context-parent");
    list.appendChild(row);

    if (expanded) {
      cluster.subgroups.forEach((theme) => {
        if(!parentMatches&&!themeMatches(theme))return;
        if (!stateMatches(state.review, `${cluster.id}:${theme.id}`, state.workflowStates)) return;
        const themeCount = filteredIncidentCount(theme.incident_row_indices, visibleRows);
        if (visibleRows && themeCount === 0) return;
        const themeSelection = { type: "theme", cluster: cluster.id, theme: theme.id };
        const themeRow = document.createElement("div");
        themeRow.className = "tree-row theme-row";
        addClusterButton(
          themeRow,
          `Theme ${theme.id} - ${displayLabel(state.review, `${cluster.id}:${theme.id}`, theme.label)} (${themeCount})`,
          themeSelection,
          "theme"
        );
        themeRow.appendChild(workflowBadge(themeSelection));
        themeRow.appendChild(reviewerLabel(`${cluster.id}:${theme.id}`));
        list.appendChild(themeRow);
      });
    }
  });
  if (!list.querySelector(".tree-row")) {
    const message = document.createElement("p");
    message.className = "dashboard-empty";
    message.textContent = "No clusters match the current filters.";
    list.append(message);
  }
}

export function treeVisibleRows() {
  const run = state.analysis;
  const hasColumnFilters = state.detailColumnFilters.some(isDetailFilterActive);
  const hasDrilldown = Array.isArray(state.detailDrilldownRowIndices);
  if (!run || (!hasColumnFilters && !hasDrilldown && !state.workflowStates.length && !state.reviewerFilter?.selected)) return null;

  let rowIndices = applyDetailFilters(run.processed_row_indices);
  rowIndices = applyDetailDrilldown(rowIndices);
  const reviewers = reviewerRows(state);
  if (reviewers) rowIndices = rowIndices.filter(row => reviewers.has(row));
  const workflow = workflowRows(run, state.review, state.workflowStates);
  return new Set(workflow ? rowIndices.filter(row => workflow.has(row)) : rowIndices);
}

export function filteredIncidentCount(rowIndices, visibleRows) {
  if (!visibleRows) return rowIndices.length;
  return rowIndices.reduce((count, rowIndex) => count + Number(visibleRows.has(rowIndex)), 0);
}

function addClusterButton(list, text, selection, extraClass = "") {
  const button = document.createElement("button");
  button.className = `cluster-item ${extraClass}`;

  if (sameSelection(selection, state.selection)) button.classList.add("active");
  button.textContent = text;
  button.addEventListener("click", () => {
    state.selection = selection;
    state.detailPage = 1;
    renderClusterList();
    renderDetailRows();
    renderWorkflow(() => renderResults());
  });
  list.appendChild(button);
  return button;
}

function workflowBadge(selection) {
  const key = selectionKey(selection), workflow = effectiveWorkflow(state.review, key);
  const badge = document.createElement("button");
  badge.type = "button";
  badge.dataset.workflowKey = key;
  badge.setAttribute("aria-haspopup", "dialog");
  badge.setAttribute("aria-controls", "workflowDialog");
  badge.setAttribute("aria-label", `Open status and comments for ${selection.type} ${key}: ${statusLabels[workflow.status]}`);
  badge.addEventListener("click", event => {
    event.stopPropagation();
    openWorkflow(selection, () => renderResults());
  });
  badge.className = "state-badge";
  badge.textContent = statusLabels[workflow.status];
  badge.title = state.review?.annotations.entries[key]?.workflow ? "Independent workflow" : "Inherited from cluster";
  return badge;
}

// ---- Row data held by the server ----
// The browser knows which rows belong to which cluster and theme, but not what they say.
// It asks the server for the rows matching the column filters, the sort rank of each row,
// the text of the visible page and the values offered by the open filter menu, and keeps
// the answers here until the view changes. Everything that reads the view stays synchronous.
function emptyRowData() {
  return { filterKey: "[]", match: null, sortColumn: null, ranks: null, page: new Map(), valuesKey: null, values: [] };
}

function rowData() {
  return (state.rowData ||= emptyRowData());
}

function activeFilters(except = null) {
  return (state.detailColumnFilters || []).flatMap((filter, column) =>
    Array.isArray(filter?.selected) && column !== except
      ? [{
          column,
          selected: filter.selected.map(detailCellValue),
          mode: state.dateFilterModes?.[column] || "exact",
          columnType: state.analysis?.column_types?.[column] || null,
        }]
      : []
  );
}

// Identifies the column filters the matching rows were asked for.
export function detailFilterKey() {
  return JSON.stringify(activeFilters());
}

function sortColumn() {
  const sort = state.detailSort;
  return sort && sort.direction !== "none" ? sort.column : null;
}

function rowDataReady() {
  const data = rowData(), column = sortColumn();
  return data.filterKey === detailFilterKey() && (column == null || data.sortColumn === column);
}

let rowLoad = null;
// Resolves once the matching rows and sort ranks fit the current view; false if another
// analysis was opened meanwhile.
export function ensureRowData() {
  if (!state.analysis || rowDataReady()) return Promise.resolve(!!state.analysis);
  return (rowLoad ||= loadRowData().finally(() => { rowLoad = null; }));
}

async function loadRowData() {
  for (;;) {
    const run = state.analysis, jobId = state.jobId, data = rowData(), key = detailFilterKey(), column = sortColumn();
    if (data.filterKey !== key) {
      const filters = activeFilters();
      const match = filters.length ? new Set((await fetchRows(jobId, { query: "match", filters })).rowIndices) : null;
      if (state.analysis !== run) return false;
      data.match = match;
      data.filterKey = key;
    } else if (column != null && data.sortColumn !== column) {
      const { ranks } = await fetchRows(jobId, { query: "order", column });
      if (state.analysis !== run) return false;
      const byRow = [];
      run.processed_row_indices.forEach((row, position) => { byRow[row] = ranks[position]; });
      data.ranks = byRow;
      data.sortColumn = column;
    } else {
      return true;
    }
    // The view may have changed while the server answered: check again.
  }
}

function detailPage() {
  const rowIndices = visibleDetailRowIndices();
  const totalRows = rowIndices.length;
  const pageSize = state.detailPageSize;
  const totalPages = Math.max(1, Math.ceil(totalRows / pageSize));
  state.detailPage = Math.min(Math.max(1, state.detailPage), totalPages);
  const start = (state.detailPage - 1) * pageSize;
  return { totalRows, totalPages, start, pageRows: rowIndices.slice(start, start + pageSize) };
}

// The values of the open filter menu's column: among the selected cluster or theme, for
// rows passing the other columns' filters.
function valuesQuery() {
  const column = state.detailOpenFilterColumn;
  if (column == null || !state.analysis) return null;
  const query = {
    query: "values",
    column,
    mode: state.dateFilterModes?.[column] || "exact",
    columnType: state.analysis.column_types?.[column] || null,
    filters: activeFilters(column),
  };
  if (state.selection?.type === "cluster" || state.selection?.type === "theme") query.cluster = state.selection.cluster;
  if (state.selection?.type === "theme") query.theme = state.selection.theme;
  return query;
}

function detailDataReady() {
  if (!rowDataReady()) return false;
  const data = rowData(), query = valuesQuery();
  return detailPage().pageRows.every((row) => data.page.has(row)) && (!query || data.valuesKey === JSON.stringify(query));
}

async function loadDetailData() {
  const run = state.analysis, jobId = state.jobId;
  if (!(await ensureRowData())) return false;
  const data = rowData(), { pageRows } = detailPage();
  const missing = pageRows.filter((row) => !data.page.has(row));
  const query = valuesQuery(), valuesKey = query && JSON.stringify(query);
  const [page, values] = await Promise.all([
    missing.length ? fetchRows(jobId, { query: "page", rowIndices: missing }) : null,
    query && data.valuesKey !== valuesKey ? fetchRows(jobId, query) : null,
  ]);
  if (state.analysis !== run) return false;
  if (page) {
    // Only the visible page is kept.
    const kept = new Map(pageRows.filter((row) => data.page.has(row)).map((row) => [row, data.page.get(row)]));
    missing.forEach((row, position) => { if (page.rows[position]) kept.set(row, page.rows[position]); });
    data.page = kept;
  }
  if (values) {
    data.values = values.values;
    data.valuesKey = valuesKey;
  }
  return true;
}

function reportRowError(error) {
  setStatus(`Could not load records: ${error.message}`, true);
}

let detailRender = 0;
function renderDetailRows(focusFilterColumn = null, focusTarget = "search") {
  const run = state.analysis;
  syncDetailColumnState(run.source.headers.length);
  const render = ++detailRender;
  if (detailDataReady()) return drawDetailRows(focusFilterColumn, focusTarget);
  // A later render supersedes this one; its own data is already on the way.
  loadDetailData().then(
    (ready) => { if (ready && render === detailRender && state.analysis === run) drawDetailRows(focusFilterColumn, focusTarget); },
    (error) => { if (render === detailRender) reportRowError(error); }
  );
}

function drawDetailRows(focusFilterColumn, focusTarget) {
  const run = state.analysis;
  const { totalRows, totalPages, start, pageRows } = detailPage();
  const page = rowData().page;
  const rows = pageRows.map((index) => page.get(index)).filter(Boolean);
  renderResultsTable("detailTable", run.source.headers, rows);
  requestAnimationFrame(syncDetailScrollbar);
  if (focusFilterColumn != null) restoreFilterFocus(focusFilterColumn, focusTarget);
  renderPagination(totalRows, start, rows.length, totalPages);
  renderDrilldownState();
  renderPivot();
}

function detailRowIndices() {
  const run = state.analysis;
  if (!run) return [];
  let rowIndices = run.processed_row_indices;
  if (state.selection?.type === "cluster") {
    const cluster = run.clusters.find((item) => item.id === state.selection.cluster);
    rowIndices = cluster?.incident_row_indices || [];
  }
  if (state.selection?.type === "theme") {
    const cluster = run.clusters.find((item) => item.id === state.selection.cluster);
    const theme = cluster?.subgroups.find((item) => item.id === state.selection.theme);
    rowIndices = theme?.incident_row_indices || [];
  }
  return rowIndices;
}

function visibleDetailRowIndices() {
  const run = state.analysis;
  if (!run) return [];
  let rowIndices = detailRowIndices();
  rowIndices = applyDetailFilters(rowIndices);
  rowIndices = applyDetailDrilldown(rowIndices);
  const reviewers = reviewerRows(state);
  if (reviewers) rowIndices = rowIndices.filter(row => reviewers.has(row));
  const workflow = workflowRows(run, state.review, state.workflowStates);
  if (workflow) rowIndices = rowIndices.filter(row => workflow.has(row));
  return applyDetailSort(rowIndices);
}

// Until the server has answered for the current filters, no row counts as matching: showing
// or exporting nothing is safer than showing or exporting rows the filters exclude.
function applyDetailFilters(rowIndices) {
  if (!activeFilters().length) return rowIndices;
  const data = rowData();
  if (data.filterKey !== detailFilterKey() || !data.match) return [];
  return rowIndices.filter((rowIndex) => data.match.has(rowIndex));
}

function applyDetailDrilldown(rowIndices) {
  if (!Array.isArray(state.detailDrilldownRowIndices)) return rowIndices;
  const drilldownRows = new Set(state.detailDrilldownRowIndices);
  return rowIndices.filter((rowIndex) => drilldownRows.has(rowIndex));
}

// Rows with equal values share a rank and keep their source order in either direction.
function applyDetailSort(rowIndices) {
  const column = sortColumn(), data = rowData();
  if (column == null || data.sortColumn !== column) return rowIndices;
  const direction = state.detailSort.direction === "desc" ? -1 : 1, ranks = data.ranks;
  return [...rowIndices].sort(
    (leftIndex, rightIndex) => ((ranks[leftIndex] ?? 0) - (ranks[rightIndex] ?? 0)) * direction || leftIndex - rightIndex
  );
}

function compareDetailValues(left, right) {
  const leftText = String(left ?? "").trim();
  const rightText = String(right ?? "").trim();
  const leftNumber = Number(leftText);
  const rightNumber = Number(rightText);
  if (leftText && rightText && Number.isFinite(leftNumber) && Number.isFinite(rightNumber)) {
    return leftNumber - rightNumber;
  }
  return leftText.localeCompare(rightText, undefined, { numeric: true, sensitivity: "base" });
}

function renderPagination(totalRows, start, shownRows, totalPages) {
  const first = totalRows === 0 ? 0 : start + 1;
  const last = start + shownRows;
  document.getElementById("pageStatus").textContent =
    `${first}-${last} of ${totalRows} records, page ${state.detailPage} of ${totalPages}`;
  document.getElementById("previousPage").disabled = state.detailPage <= 1;
  document.getElementById("nextPage").disabled = state.detailPage >= totalPages;
}

function renderDrilldownState() {
  const button = document.getElementById("clearDrilldown");
  const active = Array.isArray(state.detailDrilldownRowIndices);
  button.classList.toggle("hidden", !active);
  if (active) {
    button.textContent = `Clear Detail Filter (${state.detailDrilldownRowIndices.length})`;
    button.title = state.detailDrilldownLabel ? `Clear ${state.detailDrilldownLabel}` : "Clear detail filter";
  } else {
    button.textContent = "Clear Detail Filter";
    button.removeAttribute("title");
  }
}

function renderResultsTable(targetId, headers, rows) {
  const target = document.getElementById(targetId);
  if (!headers.length) {
    target.innerHTML = "";
    return;
  }
  const incidentNumberColumn = state.analysis?.mapping?.incident_number;
  const columnOrder = detailColumnOrder(headers.length);
  const colgroup = `<colgroup>${columnOrder
    .map((column) => {
      const width = state.detailColumnWidths[column];
      return `<col${width ? ` style="width:${width}px"` : ""}>`;
    })
    .join("")}</colgroup>`;
  target.innerHTML = `<table>${colgroup}<thead><tr>${columnOrder
    .map((column, position) => {
      const dragAttrs = ` draggable="true" data-column="${column}" data-position="${position}"`;
      const handle = `<span class="column-resizer" data-column="${column}" data-position="${position}"></span>`;
      const filter = state.detailColumnFilters[column];
      const filterSummary = detailFilterSummary(filter);
      const filterActive = isDetailFilterActive(filter) ? " active" : "";
      const filterOpen = state.detailOpenFilterColumn === column;
      const sortLabel = detailSortLabel(column);
      const sortActive = state.detailSort?.column === column && state.detailSort.direction !== "none" ? " active" : "";
      return `<th class="${filterOpen ? "filter-open" : ""}"${dragAttrs}>
        <div class="column-head">
          <span class="column-title" title="${escapeHtml(headers[column])}">${escapeHtml(headers[column])}</span>
          <button class="column-sort${sortActive}" type="button" data-column="${column}" title="Sort by ${escapeHtml(
        headers[column]
      )}">${sortLabel}</button>
          <button class="column-filter-button${filterActive}" type="button" data-column="${column}" title="Filter ${escapeHtml(
        headers[column]
      )}" aria-haspopup="true" aria-expanded="${filterOpen ? "true" : "false"}">${escapeHtml(filterSummary)}</button>
        </div>
        ${filterOpen ? renderDetailFilterMenu(column, headers[column]) : ""}
        ${handle}
      </th>`;
    })
    .join("")}</tr></thead><tbody>${rows
    .map(
      (row) =>
        `<tr>${columnOrder
          .map((column) => renderDetailCell(row, column, incidentNumberColumn))
          .join("")}</tr>`
    )
    .join("")}</tbody></table>`;
  bindColumnControls(target);
  bindColumnResizers(target);
  bindColumnDrag(target);
}

function renderDetailCell(row, column, incidentNumberColumn) {
  const value = row[column] || "";
  const text = escapeHtml(value);
  if (column !== incidentNumberColumn || !value) {
    return `<td title="${text}">${text}</td>`;
  }
  const href = `https://goto/snow-id/${encodeURIComponent(value)}`;
  return `<td title="${text}"><a href="${escapeHtml(href)}" target="_blank" rel="noopener noreferrer">${text}</a></td>`;
}

function renderPivot() {
  const run = state.analysis;
  if (!run) return;
  syncPivotState(run.source.headers.length);
  renderPivotFields(run.source.headers);
  renderPivotBuckets(run.source.headers);
  renderPivotTable();
  bindPivotDrag();
}

function syncPivotState(columnCount) {
  const valid = (column) => Number.isInteger(column) && column >= 0 && column < columnCount;
  state.pivotRows = state.pivotRows.filter(valid);
  state.pivotColumns = state.pivotColumns.filter(valid).filter((column) => !state.pivotRows.includes(column));
}

function renderPivotFields(headers) {
  const usedColumns = new Set([...state.pivotRows, ...state.pivotColumns]);
  document.getElementById("pivotFields").innerHTML = headers
    .map(
      (header, column) =>
        `<button class="pivot-field${usedColumns.has(column) ? " used" : ""}" type="button" draggable="true" data-column="${column}" title="${escapeHtml(
          header
        )}">${escapeHtml(header)}</button>`
    )
    .join("");
}

function renderPivotBuckets(headers) {
  document.getElementById("pivotRows").innerHTML = renderPivotBucketItems(state.pivotRows, headers, "rows");
  document.getElementById("pivotColumns").innerHTML = renderPivotBucketItems(state.pivotColumns, headers, "columns");
}

function renderPivotBucketItems(columns, headers, area) {
  if (!columns.length) return `<div class="pivot-empty">Drop fields here</div>`;
  return columns
    .map(
      (column) =>
        `<span class="pivot-chip" draggable="true" data-column="${column}" data-area="${area}" title="${escapeHtml(headers[column])}">
          <span>${escapeHtml(headers[column])}</span>
          <button class="pivot-remove" type="button" data-column="${column}" data-area="${area}" aria-label="Remove ${escapeHtml(
          headers[column]
        )}">x</button>
        </span>`
    )
    .join("");
}

let pivotController=null;
async function renderPivotTable() {
  pivotController?.abort();pivotController=new AbortController();
  const rowIndices = visibleDetailRowIndices();
  const requestId = ++state.pivotRequestId;
  document.getElementById("pivotStatus").textContent = `${rowIndices.length} records summarized`;
  const target = document.getElementById("pivotTable");
  if (!state.pivotRows.length && !state.pivotColumns.length) {
    target.innerHTML = `<div class="pivot-placeholder">Drag fields to Rows or Columns.</div>`;
    return;
  }

  target.innerHTML = `<div class="pivot-placeholder">Calculating pivot...</div>`;
  const started=performance.now();let latest={phase:"Starting calculation",completed:0,total:0};
  const progressTimer=setInterval(()=>{if(requestId!==state.pivotRequestId||performance.now()-started<5000)return;
    const elapsed=Math.floor((performance.now()-started)/1000),count=latest.total?` — ${latest.completed.toLocaleString()} of ${latest.total.toLocaleString()} (${Math.floor(latest.completed/latest.total*100)}%)`:"";
    document.getElementById("pivotStatus").textContent=`${latest.phase}${count} · ${elapsed}s elapsed`;
  },250);
  try {
    const pivot = await fetchPivot(state.jobId, rowIndices, state.pivotRows, state.pivotColumns,p=>latest=p,pivotController.signal);
    if (requestId !== state.pivotRequestId) return;
    document.getElementById("pivotStatus").textContent = `${pivot.recordCount} records summarized`;
    renderPivotResponse(target, pivot);
  } catch (error) {
    if (requestId !== state.pivotRequestId) return;
    target.innerHTML = `<div class="pivot-placeholder">Pivot failed: ${escapeHtml(error.message)}</div>`;
  } finally {clearInterval(progressTimer);}
}

function renderPivotResponse(target, pivot) {
  const numericColumns = new Set(pivot.numericColumns || []);
  state.currentPivotRows = pivot.rows || [];
  target.innerHTML = `<table><thead><tr>${pivot.headers
    .map((header, column) => `<th class="${numericColumns.has(column) ? "numeric" : ""}">${escapeHtml(header)}</th>`)
    .join("")}</tr></thead><tbody>${pivot.rows
    .map(
      (row, rowIndex) =>
        `<tr class="pivot-data-row${row.total ? " pivot-total-row" : ""}" data-pivot-row="${rowIndex}" tabindex="0">${row.cells
          .map(
            (cell, column) =>
              `<td class="${numericColumns.has(column) ? "numeric pivot-value" : ""}" data-pivot-column="${column}" title="${
                numericColumns.has(column) ? "Double-click to filter records" : escapeHtml(cell)
              }">${escapeHtml(cell)}</td>`
          )
          .join("")}</tr>`
    )
    .join("")}</tbody></table>`;
  bindPivotRowSelection(target, pivot);
}

function bindPivotRowSelection(target, pivot) {
  target.querySelectorAll(".pivot-data-row").forEach((row) => {
    row.addEventListener("click", () => selectPivotRow(row));
    row.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      selectPivotRow(row);
      showPivotContextMenu(event.clientX, event.clientY);
    });
    row.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        selectPivotRow(row);
        applySelectedPivotDrilldown();
      }
    });
  });
  target.querySelectorAll(".pivot-value").forEach((cell) => {
    cell.addEventListener("dblclick", (event) => {
      event.preventDefault();
      const rowIndex = Number(cell.closest(".pivot-data-row")?.dataset.pivotRow);
      const tableColumn = Number(cell.dataset.pivotColumn);
      applyPivotCellFilters(pivot, rowIndex, tableColumn);
    });
  });
}

export function pivotCellFilters(
  pivot,
  rowIndex,
  tableColumn,
  rowColumns = state.pivotRows,
  columnColumns = state.pivotColumns
) {
  const row = pivot.rows?.[rowIndex];
  if (!row || !(pivot.numericColumns || []).includes(tableColumn)) return [];

  const filters = [];
  if (!row.total) {
    rowColumns.forEach((column, index) => {
      const selected = row.rowFilterValues?.[index];
      if (selected) filters.push({ column, selected });
    });
  }

  const numericStart = rowColumns.length || 1;
  const columnBucket = tableColumn - numericStart;
  const columnFilterValues = pivot.columnFilterValues?.[columnBucket];
  if (columnFilterValues) {
    columnColumns.forEach((column, index) => {
      const selected = columnFilterValues[index];
      if (selected) filters.push({ column, selected });
    });
  }
  return filters;
}

function applyPivotCellFilters(pivot, rowIndex, tableColumn) {
  const filters = pivotCellFilters(pivot, rowIndex, tableColumn);
  syncDetailColumnState(state.analysis.source.headers.length);
  filters.forEach(({ column, selected }) => {
    (state.dateFilterModes||=[])[column]="exact";
    state.detailColumnFilters[column] = {
      selected: selected.map(detailCellValue).sort(compareDetailValues),
      query: "",
      searchDeselected: false,
    };
  });
  state.detailOpenFilterColumn = null;
  state.detailPage = 1;
  hidePivotContextMenu();
  renderClusterList();
  renderDetailRows();
  showStep("results");
}

function selectPivotRow(row) {
  document.querySelectorAll(".pivot-data-row.selected").forEach((item) => item.classList.remove("selected"));
  row.classList.add("selected");
  const pivotRow = state.currentPivotRows[Number(row.dataset.pivotRow)];
  state.selectedPivotRowIndices = pivotRow?.rowIndices || [];
  state.selectedPivotRowLabel = pivotRowLabel(pivotRow);
}

function pivotRowLabel(row) {
  if (!row) return "selected pivot row";
  if (row.total) return "pivot total";
  const label = row.cells.filter((cell) => String(cell || "").trim()).slice(0, 3).join(" / ");
  return label || "selected pivot row";
}

function showPivotContextMenu(x, y) {
  const menu = document.getElementById("pivotContextMenu");
  menu.classList.remove("hidden");
  const bounds = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(x, window.innerWidth - bounds.width - 8)}px`;
  menu.style.top = `${Math.min(y, window.innerHeight - bounds.height - 8)}px`;
}

function hidePivotContextMenu() {
  document.getElementById("pivotContextMenu")?.classList.add("hidden");
}

function applySelectedPivotDrilldown() {
  if (!Array.isArray(state.selectedPivotRowIndices)) return;
  state.detailDrilldownRowIndices = [...state.selectedPivotRowIndices];
  state.detailDrilldownLabel = state.selectedPivotRowLabel;
  state.detailPage = 1;
  hidePivotContextMenu();
  renderClusterList();
  renderDetailRows();
  showStep("results");
}

function clearDetailDrilldown() {
  state.detailDrilldownRowIndices = null;
  state.detailDrilldownLabel = "";
  state.detailPage = 1;
  renderClusterList();
  renderDetailRows();
}

// Fields, chips and remove buttons are recreated on every render, so they are bound each time.
function bindPivotDrag() {
  document.querySelectorAll(".pivot-field, .pivot-chip").forEach((item) => {
    item.addEventListener("dragstart", (event) => {
      if (event.target.closest(".pivot-remove")) {
        event.preventDefault();
        return;
      }
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", item.dataset.column);
    });
  });

  document.querySelectorAll(".pivot-remove").forEach((button) => {
    button.addEventListener("click", () => removePivotField(Number(button.dataset.column), button.dataset.area));
  });
}

// The Rows and Columns drop zones are static page elements: bind them once, at start-up.
// Binding them on every render stacked listeners, so one drop sent 2, 4, 8… pivot requests.
function bindPivotDropZones() {
  document.querySelectorAll(".pivot-drop-zone").forEach((zone) => {
    zone.addEventListener("dragover", (event) => {
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      zone.classList.add("drag-over");
    });
    zone.addEventListener("dragleave", () => zone.classList.remove("drag-over"));
    zone.addEventListener("drop", (event) => {
      event.preventDefault();
      zone.classList.remove("drag-over");
      const column = Number(event.dataTransfer.getData("text/plain"));
      movePivotField(column, zone.dataset.pivotArea);
    });
  });
}

function movePivotField(column, area) {
  if (!Number.isInteger(column)) return;
  state.pivotRows = state.pivotRows.filter((item) => item !== column);
  state.pivotColumns = state.pivotColumns.filter((item) => item !== column);
  if (area === "rows") state.pivotRows.push(column);
  if (area === "columns") state.pivotColumns.push(column);
  renderPivot();
}

function removePivotField(column, area) {
  if (area === "rows") state.pivotRows = state.pivotRows.filter((item) => item !== column);
  if (area === "columns") state.pivotColumns = state.pivotColumns.filter((item) => item !== column);
  renderPivot();
}

function detailColumnOrder(columnCount) {
  if (state.detailColumnOrder.length !== columnCount) {
    state.detailColumnOrder = Array.from({ length: columnCount }, (_, index) => index);
  }
  return state.detailColumnOrder;
}

function syncDetailColumnState(columnCount) {
  detailColumnOrder(columnCount);
  state.detailColumnFilters = Array.from({ length: columnCount }, (_, index) =>
    normalizeDetailFilter(state.detailColumnFilters[index])
  );
  if (state.detailOpenFilterColumn != null && state.detailOpenFilterColumn >= columnCount) state.detailOpenFilterColumn = null;
  if (state.detailSort && state.detailSort.column >= columnCount) state.detailSort = null;
}

function normalizeDetailFilter(filter) {
  if (filter && typeof filter === "object") {
    return {
      selected: Array.isArray(filter.selected) ? [...new Set(filter.selected.map(detailCellValue))] : null,
      query: String(filter.query || ""),
      searchDeselected: Boolean(filter.searchDeselected),
    };
  }
  const query = String(filter || "");
  return { selected: null, query, searchDeselected: false };
}

function isDetailFilterActive(filter) {
  return Array.isArray(filter?.selected);
}

function activeFilterValues(filter) {
  if (!Array.isArray(filter?.selected)) return null;
  return new Set(filter.selected.map(detailCellValue));
}

function detailCellValue(value) {
  return String(value ?? "");
}

function detailFilterSummary(filter) {
  if (!isDetailFilterActive(filter)) return "All";
  if (filter.selected.length === 0) return "None";
  if (filter.selected.length === 1) return filter.selected[0] || "(blank)";
  return `${filter.selected.length} selected`;
}

function renderDetailFilterMenu(column, header) {
  const filter = state.detailColumnFilters[column];
  const query = String(filter.query || "");
  const queryText = query.trim().toLocaleLowerCase();
  const values = detailFilterValues(column);
  const visibleValues = queryText
    ? values.filter((value) => detailFilterLabel(value).toLocaleLowerCase().includes(queryText))
    : values;
  const selectedValues = activeFilterValues(filter);
  const allVisibleSelected =
    visibleValues.length > 0 && visibleValues.every((value) => !selectedValues || selectedValues.has(value));

  return `<div class="column-filter-menu" data-column="${column}" role="dialog" aria-label="Filter ${escapeHtml(header)}">
    ${state.analysis.column_types?.[column]?.dataType==="Date/Time"?`<label>Filter date/time by <select class="date-filter-mode" data-column="${column}" aria-label="Date filter mode">${[["exact","Exact value"],["date","Date only"],["month","Month only (year-month)"]].map(([value,label])=>`<option value="${value}" ${(state.dateFilterModes?.[column]||"exact")===value?"selected":""}>${label}</option>`).join("")}</select></label>`:""}
    <input class="column-filter-search" data-column="${column}" type="search" value="${escapeHtml(
    query
  )}" placeholder="Search values" aria-label="Search ${escapeHtml(header)} values" />
    <div class="column-filter-actions">
      <button class="filter-clear" type="button" data-column="${column}" ${
    isDetailFilterActive(filter) ? "" : "disabled"
  }>Clear</button>
    </div>
    <div class="column-filter-options">
      <label class="column-filter-option column-filter-select-all" title="${
        allVisibleSelected ? "Deselect all visible values" : "Select all visible values"
      }">
        <input class="filter-select-visible" type="checkbox" data-column="${column}" ${
    visibleValues.length ? "" : "disabled"
  } ${allVisibleSelected ? "checked" : ""} />
        <span>Select All</span>
      </label>
      ${
        visibleValues.length
          ? visibleValues
              .map((value) => {
                const checked = !selectedValues || selectedValues.has(value);
                return `<label class="column-filter-option" title="${escapeHtml(detailFilterLabel(value))}">
                  <input type="checkbox" data-column="${column}" value="${escapeHtml(value)}" ${checked ? "checked" : ""} />
                  <span>${escapeHtml(detailFilterLabel(value))}</span>
                </label>`;
              })
              .join("")
          : `<div class="column-filter-empty">No matching values</div>`
      }
    </div>
  </div>`;
}

function detailFilterLabel(value) {
  return value === "" ? "(blank)" : value;
}

// Values of the open filter menu's column, sorted by the server.
function detailFilterValues(column) {
  const query = valuesQuery(), data = rowData();
  return query && query.column === column && data.valuesKey === JSON.stringify(query) ? data.values : [];
}

function detailSortLabel(column) {
  if (state.detailSort?.column !== column) return "▴▾";
  if (state.detailSort.direction === "asc") return "▴";
  if (state.detailSort.direction === "desc") return "▾";
  return "▴▾";
}

export function bindResultsSplitter() {
  const splitter = document.getElementById("resultsSplitter");
  const layout = document.querySelector(".results-layout");
  if (!splitter || !layout) return;

  splitter.addEventListener("mousedown", (event) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = document.getElementById("clusterList").getBoundingClientRect().width;
    document.body.classList.add("is-resizing");

    const onMove = (moveEvent) => {
      const bounds = layout.getBoundingClientRect();
      const minWidth = 280;
      const maxWidth = Math.max(minWidth, bounds.width - 420);
      state.resultsTreeWidth = Math.min(maxWidth, Math.max(minWidth, startWidth + moveEvent.clientX - startX));
      applyResultsPaneWidth();
    };

    const onUp = () => {
      document.body.classList.remove("is-resizing");
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };

    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  });
}

function applyResultsPaneWidth() {
  const layout = document.querySelector(".results-layout");
  if (!layout || !state.resultsTreeWidth) return;
  const bounds = layout.getBoundingClientRect();
  const minWidth = 280;
  const maxWidth = Math.max(minWidth, bounds.width - 420);
  state.resultsTreeWidth = Math.min(maxWidth, Math.max(minWidth, state.resultsTreeWidth));
  layout.style.gridTemplateColumns = `${state.resultsTreeWidth}px 8px minmax(0, 1fr)`;
}

function bindColumnResizers(target) {
  target.querySelectorAll(".column-resizer").forEach((handle) => {
    handle.addEventListener("mousedown", (event) => {
      event.preventDefault();
      event.stopPropagation();
      const column = Number(handle.dataset.column);
      const position = Number(handle.dataset.position);
      const table = target.querySelector("table");
      const col = table?.querySelectorAll("col")[position];
      const th = handle.closest("th");
      const startX = event.clientX;
      const startWidth = state.detailColumnWidths[column] || th.getBoundingClientRect().width;
      document.body.classList.add("is-resizing");

      const onMove = (moveEvent) => {
        const width = Math.max(80, startWidth + moveEvent.clientX - startX);
        state.detailColumnWidths[column] = width;
        if (col) col.style.width = `${width}px`;
        requestAnimationFrame(syncDetailScrollbar);
      };

      const onUp = () => {
        document.body.classList.remove("is-resizing");
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };

      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    });
  });
}

function bindColumnControls(target) {
  target.querySelectorAll(".date-filter-mode").forEach(select=>select.onchange=()=>{const column=Number(select.dataset.column);(state.dateFilterModes||=[])[column]=select.value;state.detailColumnFilters[column]={selected:null,query:"",searchDeselected:false};state.detailPage=1;renderClusterList();renderDetailRows(column);});
  target.querySelectorAll(".column-sort").forEach((button) => {
    button.addEventListener("mousedown", (event) => event.stopPropagation());
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      const column = Number(button.dataset.column);
      state.detailSort = nextDetailSort(column);
      state.detailPage = 1;
      renderDetailRows();
    });
  });

  target.querySelectorAll(".column-filter-button").forEach((button) => {
    button.addEventListener("mousedown", (event) => event.stopPropagation());
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      const column = Number(button.dataset.column);
      state.detailOpenFilterColumn = state.detailOpenFilterColumn === column ? null : column;
      renderDetailRows(column);
    });
  });

  target.querySelectorAll(".column-filter-menu").forEach((menu) => {
    menu.addEventListener("mousedown", (event) => event.stopPropagation());
    menu.addEventListener("click", (event) => event.stopPropagation());
  });

  target.querySelectorAll(".column-filter-search").forEach((input) => {
    input.addEventListener("mousedown", (event) => event.stopPropagation());
    input.addEventListener("click", (event) => event.stopPropagation());
    input.addEventListener("input", () => {
      const column = Number(input.dataset.column);
      updateDetailFilterQuery(column, input.value);
      state.detailOpenFilterColumn = column;
      renderClusterList();
      renderDetailRows(column, "search");
    });
  });

  target.querySelectorAll(".column-filter-option input:not(.filter-select-visible)").forEach((checkbox) => {
    checkbox.addEventListener("change", () => {
      const column = Number(checkbox.dataset.column);
      setDetailFilterValue(column, checkbox.value, checkbox.checked);
      renderClusterList();
      renderDetailRows(column);
    });
  });

  target.querySelectorAll(".filter-select-visible").forEach((checkbox) => {
    checkbox.addEventListener("change", () => {
      const column = Number(checkbox.dataset.column);
      toggleVisibleFilterValues(column);
      renderClusterList();
      renderDetailRows(column);
    });
  });

  target.querySelectorAll(".filter-clear").forEach((button) => {
    button.addEventListener("click", () => {
      const column = Number(button.dataset.column);
      state.detailColumnFilters[column] = { selected: null, query: "", searchDeselected: false };
      state.detailPage = 1;
      state.detailOpenFilterColumn = column;
      renderClusterList();
      renderDetailRows(column);
    });
  });
}

function updateDetailFilterQuery(column, query) {
  const filter = state.detailColumnFilters[column];
  const previousQuery = String(filter.query || "");
  const nextQuery = String(query || "");
  if (nextQuery && !previousQuery && !filter.searchDeselected) {
    filter.selected = [];
    filter.searchDeselected = true;
    state.detailPage = 1;
  }
  if (!nextQuery) {
    filter.searchDeselected = false;
  }
  filter.query = nextQuery;
}

function setDetailFilterValue(column, value, checked) {
  const filter = state.detailColumnFilters[column];
  const values = activeFilterValues(filter) || new Set(detailFilterValues(column));
  if (checked) {
    values.add(value);
  } else {
    values.delete(value);
  }
  filter.selected = [...values].sort(compareDetailValues);
  state.detailPage = 1;
  state.detailOpenFilterColumn = column;
}

function toggleVisibleFilterValues(column) {
  const filter = state.detailColumnFilters[column];
  const queryText = String(filter.query || "").trim().toLocaleLowerCase();
  const visibleValues = detailFilterValues(column).filter((value) =>
    queryText ? detailFilterLabel(value).toLocaleLowerCase().includes(queryText) : true
  );
  const selectedValues = activeFilterValues(filter) || new Set(detailFilterValues(column));
  const allVisibleSelected = visibleValues.length > 0 && visibleValues.every((value) => selectedValues.has(value));
  visibleValues.forEach((value) => {
    if (allVisibleSelected) {
      selectedValues.delete(value);
    } else {
      selectedValues.add(value);
    }
  });
  filter.selected = [...selectedValues].sort(compareDetailValues);
  state.detailPage = 1;
  state.detailOpenFilterColumn = column;
}

function nextDetailSort(column) {
  if (state.detailSort?.column !== column) return { column, direction: "asc" };
  if (state.detailSort.direction === "asc") return { column, direction: "desc" };
  return null;
}

function restoreFilterFocus(column, focusTarget = "search") {
  if (focusTarget === "button") {
    document.querySelector(`.column-filter-button[data-column="${column}"]`)?.focus();
    return;
  }
  const input = document.querySelector(`.column-filter-search[data-column="${column}"]`);
  if (input) {
    input.focus();
    const length = input.value.length;
    input.setSelectionRange(length, length);
    return;
  }
  document.querySelector(`.column-filter-button[data-column="${column}"]`)?.focus();
}

function bindColumnDrag(target) {
  target.querySelectorAll("th[draggable='true']").forEach((header) => {
    header.addEventListener("dragstart", (event) => {
      if (event.target.closest(".column-filter-menu, .column-filter-button, .column-sort, .column-resizer")) {
        event.preventDefault();
        return;
      }
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", header.dataset.column);
      header.classList.add("dragging");
    });

    header.addEventListener("dragend", () => {
      target.querySelectorAll("th").forEach((item) => item.classList.remove("dragging", "drag-over"));
    });

    header.addEventListener("dragover", (event) => {
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      header.classList.add("drag-over");
    });

    header.addEventListener("dragleave", () => {
      header.classList.remove("drag-over");
    });

    header.addEventListener("drop", (event) => {
      event.preventDefault();
      const sourceColumn = Number(event.dataTransfer.getData("text/plain"));
      const targetColumn = Number(header.dataset.column);
      moveDetailColumn(sourceColumn, targetColumn);
      renderDetailRows();
    });
  });
}

function moveDetailColumn(sourceColumn, targetColumn) {
  if (sourceColumn === targetColumn) return;
  const order = [...state.detailColumnOrder];
  const sourceIndex = order.indexOf(sourceColumn);
  const targetIndex = order.indexOf(targetColumn);
  if (sourceIndex < 0 || targetIndex < 0) return;
  const [moved] = order.splice(sourceIndex, 1);
  order.splice(targetIndex, 0, moved);
  state.detailColumnOrder = order;
}
