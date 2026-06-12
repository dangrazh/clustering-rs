import { exportExcel, fetchPivot, fetchResult, restoreSession } from "./api.js";
import { state } from "./state.js";
import { downloadJson, setStatus, showStep, statsHtml } from "./ui.js";
import { clusterId, clusterKey, escapeHtml, sameSelection } from "./utils.js";

export function bindResultsEvents() {
  document.getElementById("downloadSession").addEventListener("click", () => {
    if (!state.analysis) return;
    downloadJson("incident_analysis_session.json", { version: 2, run: state.analysis, reviewState: reviewStatePayload() });
  });

  document.getElementById("downloadReviewState").addEventListener("click", () => {
    if (!state.analysis) return;
    downloadJson("incident_review_state.json", { version: 1, reviewState: reviewStatePayload() });
  });

  document.getElementById("reviewStateInput").addEventListener("change", async (event) => {
    const file = event.target.files?.[0];
    if (!file) return;
    try {
      const payload = JSON.parse(await file.text());
      applyReviewState(payload.reviewState || payload);
      renderClusterList();
      setStatus(`Loaded review state from ${file.name}.`);
    } catch (error) {
      setStatus(error.message, true);
    } finally {
      event.target.value = "";
    }
  });

  document.getElementById("exportExcel").addEventListener("click", () => {
    if (state.jobId) exportExcel(state.jobId);
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

  document.addEventListener("click", (event) => {
    if (!event.target.closest(".pivot-context-menu")) hidePivotContextMenu();
    if (state.detailOpenFilterColumn == null || event.target.closest(".column-filter-menu, .column-filter-button")) return;
    state.detailOpenFilterColumn = null;
    renderDetailRows();
  });

  window.addEventListener("resize", () => applyResultsPaneWidth());
}

export async function loadResult(jobId) {
  initializeResult(await fetchResult(jobId), jobId);
  renderResults();
  showStep("results");
}

export async function loadSavedSession(payload) {
  const run = payload?.run;
  if (!run?.source?.headers || !Array.isArray(run.source.rows)) throw new Error("Session file is invalid.");
  const restored = await restoreSession(run);
  initializeResult(run, restored.jobId, payload.reviewState);
  renderResults();
  showStep("results");
}

function initializeResult(run, jobId, reviewState = null) {
  state.analysis = run;
  state.source = null;
  state.mapping = run.mapping || null;
  state.jobId = jobId;
  state.selection = { type: "all" };
  state.expandedClusters = new Set();
  state.reviewedClusters = new Set();
  state.reviewedThemes = new Set();
  if (reviewState) applyReviewState(reviewState);
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
  document.querySelector('[data-step="mapping"]').disabled = true;
  document.querySelector('[data-step="analysis"]').disabled = true;
  document.querySelector('[data-step="results"]').disabled = false;
  document.querySelector('[data-step="pivot"]').disabled = false;
}

function renderResults() {
  const run = state.analysis;
  applyResultsPaneWidth();
  document.getElementById("resultStats").innerHTML = statsHtml([
    ["Clusters", run.clusters.length],
    ["Processed", run.processed_incidents.length],
    ["Ignored", run.ignored_rows.length],
    ["Unclustered", run.unclustered_row_indices.length],
  ]);
  renderClusterList();
  renderDetailRows();
  renderPivot();
}

function renderClusterList() {
  const list = document.getElementById("clusterList");
  const drilldownRows = activeDrilldownRows();
  list.innerHTML = "";
  addClusterButton(list, drilldownRows ? `All incidents (${drilldownRows.size})` : "All incidents", { type: "all" });
  state.analysis.clusters.forEach((cluster) => {
    const clusterCount = filteredIncidentCount(cluster.incident_row_indices, drilldownRows);
    if (drilldownRows && clusterCount === 0) return;
    const key = clusterKey(cluster.id);
    const expanded = state.expandedClusters.has(key);
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
      `${clusterId(cluster.id)} - ${cluster.label} (${clusterCount})`,
      clusterSelection,
      "cluster-label"
    );
    row.appendChild(reviewToggleButton("cluster", clusterSelection, isReviewed("cluster", clusterSelection)));
    list.appendChild(row);

    if (expanded) {
      cluster.subgroups.forEach((theme) => {
        const themeCount = filteredIncidentCount(theme.incident_row_indices, drilldownRows);
        if (drilldownRows && themeCount === 0) return;
        const themeSelection = { type: "theme", cluster: cluster.id, theme: theme.id };
        const themeRow = document.createElement("div");
        themeRow.className = "tree-row theme-row";
        addClusterButton(
          themeRow,
          `Theme ${theme.id} - ${theme.label} (${themeCount})`,
          themeSelection,
          "theme"
        );
        themeRow.appendChild(reviewToggleButton("theme", themeSelection, isReviewed("theme", themeSelection)));
        list.appendChild(themeRow);
      });
    }
  });
}

function activeDrilldownRows() {
  return Array.isArray(state.detailDrilldownRowIndices) ? new Set(state.detailDrilldownRowIndices) : null;
}

function filteredIncidentCount(rowIndices, drilldownRows) {
  if (!drilldownRows) return rowIndices.length;
  return rowIndices.reduce((count, rowIndex) => count + Number(drilldownRows.has(rowIndex)), 0);
}

function addClusterButton(list, text, selection, extraClass = "") {
  const button = document.createElement("button");
  button.className = `cluster-item ${extraClass}`;
  if (isReviewed(selection.type, selection)) button.classList.add("reviewed");
  if (sameSelection(selection, state.selection)) button.classList.add("active");
  button.textContent = text;
  button.addEventListener("click", () => {
    state.selection = selection;
    state.detailPage = 1;
    renderClusterList();
    renderDetailRows();
    renderPivot();
  });
  list.appendChild(button);
  return button;
}

function reviewToggleButton(type, selection, reviewed) {
  const button = document.createElement("button");
  button.className = `review-toggle${reviewed ? " reviewed" : ""}`;
  button.type = "button";
  button.textContent = reviewed ? "Reviewed" : "Review";
  button.setAttribute("aria-pressed", String(reviewed));
  button.setAttribute("aria-label", `${reviewed ? "Clear reviewed state for" : "Mark reviewed"} ${type}`);
  button.addEventListener("click", (event) => {
    event.stopPropagation();
    toggleReviewed(type, selection);
  });
  return button;
}

function isReviewed(type, selection) {
  if (type === "cluster") return state.reviewedClusters.has(reviewClusterKey(selection.cluster));
  if (type === "theme") return state.reviewedThemes.has(reviewThemeKey(selection.cluster, selection.theme));
  return false;
}

function toggleReviewed(type, selection) {
  const set = type === "cluster" ? state.reviewedClusters : state.reviewedThemes;
  const key = type === "cluster" ? reviewClusterKey(selection.cluster) : reviewThemeKey(selection.cluster, selection.theme);
  if (set.has(key)) {
    set.delete(key);
  } else {
    set.add(key);
  }
  renderClusterList();
}

function reviewClusterKey(cluster) {
  return clusterKey(cluster);
}

function reviewThemeKey(cluster, theme) {
  return `${clusterKey(cluster)}:${theme}`;
}

function reviewStatePayload() {
  return {
    reviewedClusters: [...state.reviewedClusters].sort(),
    reviewedThemes: [...state.reviewedThemes].sort(),
  };
}

function applyReviewState(payload) {
  if (!payload || typeof payload !== "object") throw new Error("Review state file is invalid.");
  state.reviewedClusters = new Set(Array.isArray(payload.reviewedClusters) ? payload.reviewedClusters.map(String) : []);
  state.reviewedThemes = new Set(Array.isArray(payload.reviewedThemes) ? payload.reviewedThemes.map(String) : []);
}

function renderDetailRows(focusFilterColumn = null, focusTarget = "search") {
  const run = state.analysis;
  syncDetailColumnState(run.source.headers.length);
  const rowIndices = visibleDetailRowIndices();
  const totalRows = rowIndices.length;
  const pageSize = state.detailPageSize;
  const totalPages = Math.max(1, Math.ceil(totalRows / pageSize));
  state.detailPage = Math.min(Math.max(1, state.detailPage), totalPages);
  const start = (state.detailPage - 1) * pageSize;
  const pageRows = rowIndices.slice(start, start + pageSize);
  const rows = pageRows.map((index) => run.source.rows[index]).filter(Boolean);
  renderResultsTable("detailTable", run.source.headers, rows);
  if (focusFilterColumn != null) restoreFilterFocus(focusFilterColumn, focusTarget);
  renderPagination(totalRows, start, rows.length, totalPages);
  renderDrilldownState();
  renderPivot();
}

function detailRowIndices() {
  const run = state.analysis;
  if (!run) return [];
  let rowIndices = run.processed_incidents.map((record) => record.source_row_index);
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
  rowIndices = applyDetailFilters(rowIndices, run.source.rows);
  rowIndices = applyDetailDrilldown(rowIndices);
  return applyDetailSort(rowIndices, run.source.rows);
}

function applyDetailFilters(rowIndices, rows) {
  const filters = state.detailColumnFilters
    .map((filter, column) => ({ column, selectedValues: activeFilterValues(filter) }))
    .filter((filter) => filter.selectedValues);
  if (!filters.length) return rowIndices;
  return rowIndices.filter((rowIndex) => {
    const row = rows[rowIndex] || [];
    return filters.every(({ column, selectedValues }) => selectedValues.has(detailCellValue(row[column])));
  });
}

function applyDetailDrilldown(rowIndices) {
  if (!Array.isArray(state.detailDrilldownRowIndices)) return rowIndices;
  const drilldownRows = new Set(state.detailDrilldownRowIndices);
  return rowIndices.filter((rowIndex) => drilldownRows.has(rowIndex));
}

function applyDetailSort(rowIndices, rows) {
  const sort = state.detailSort;
  if (!sort || sort.direction === "none") return rowIndices;
  const direction = sort.direction === "desc" ? -1 : 1;
  return [...rowIndices].sort((leftIndex, rightIndex) => {
    const left = rows[leftIndex]?.[sort.column] ?? "";
    const right = rows[rightIndex]?.[sort.column] ?? "";
    const comparison = compareDetailValues(left, right);
    return comparison === 0 ? leftIndex - rightIndex : comparison * direction;
  });
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

async function renderPivotTable() {
  const rowIndices = visibleDetailRowIndices();
  const requestId = ++state.pivotRequestId;
  document.getElementById("pivotStatus").textContent = `${rowIndices.length} records summarized`;
  const target = document.getElementById("pivotTable");
  if (!state.pivotRows.length && !state.pivotColumns.length) {
    target.innerHTML = `<div class="pivot-placeholder">Drag fields to Rows or Columns.</div>`;
    return;
  }

  target.innerHTML = `<div class="pivot-placeholder">Calculating pivot...</div>`;
  try {
    const pivot = await fetchPivot(state.jobId, rowIndices, state.pivotRows, state.pivotColumns);
    if (requestId !== state.pivotRequestId) return;
    document.getElementById("pivotStatus").textContent = `${pivot.recordCount} records summarized`;
    renderPivotResponse(target, pivot);
  } catch (error) {
    if (requestId !== state.pivotRequestId) return;
    target.innerHTML = `<div class="pivot-placeholder">Pivot failed: ${escapeHtml(error.message)}</div>`;
  }
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
              `<td class="${numericColumns.has(column) ? "numeric" : ""}" title="${escapeHtml(cell)}">${escapeHtml(cell)}</td>`
          )
          .join("")}</tr>`
    )
    .join("")}</tbody></table>`;
  bindPivotRowSelection(target);
}

function bindPivotRowSelection(target) {
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

  document.querySelectorAll(".pivot-remove").forEach((button) => {
    button.addEventListener("click", () => removePivotField(Number(button.dataset.column), button.dataset.area));
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

function detailFilterValues(column) {
  const run = state.analysis;
  if (!run) return [];
  const rows = run.source.rows;
  const rowIndices = applyDetailFiltersExcept(detailRowIndices(), rows, column);
  const values = new Set();
  rowIndices.forEach((rowIndex) => values.add(detailCellValue(rows[rowIndex]?.[column])));
  return [...values].sort(compareDetailValues);
}

function applyDetailFiltersExcept(rowIndices, rows, ignoredColumn) {
  const filters = state.detailColumnFilters
    .map((filter, column) => ({ column, selectedValues: activeFilterValues(filter) }))
    .filter((filter) => filter.selectedValues && filter.column !== ignoredColumn);
  if (!filters.length) return rowIndices;
  return rowIndices.filter((rowIndex) => {
    const row = rows[rowIndex] || [];
    return filters.every(({ column, selectedValues }) => selectedValues.has(detailCellValue(row[column])));
  });
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
      renderDetailRows(column, "search");
    });
  });

  target.querySelectorAll(".column-filter-option input:not(.filter-select-visible)").forEach((checkbox) => {
    checkbox.addEventListener("change", () => {
      const column = Number(checkbox.dataset.column);
      setDetailFilterValue(column, checkbox.value, checkbox.checked);
      renderDetailRows(column);
    });
  });

  target.querySelectorAll(".filter-select-visible").forEach((checkbox) => {
    checkbox.addEventListener("change", () => {
      const column = Number(checkbox.dataset.column);
      toggleVisibleFilterValues(column);
      renderDetailRows(column);
    });
  });

  target.querySelectorAll(".filter-clear").forEach((button) => {
    button.addEventListener("click", () => {
      const column = Number(button.dataset.column);
      state.detailColumnFilters[column] = { selected: null, query: "", searchDeselected: false };
      state.detailPage = 1;
      state.detailOpenFilterColumn = column;
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
