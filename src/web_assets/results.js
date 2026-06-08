import { exportExcel, fetchResult } from "./api.js";
import { state } from "./state.js";
import { downloadJson, showStep, statsHtml } from "./ui.js";
import { clusterId, clusterKey, escapeHtml, sameSelection } from "./utils.js";

export function bindResultsEvents() {
  document.getElementById("downloadSession").addEventListener("click", () => {
    if (!state.analysis) return;
    downloadJson("incident_analysis_session.json", { version: 1, run: state.analysis });
  });

  document.getElementById("exportExcel").addEventListener("click", () => {
    if (state.jobId) exportExcel(state.jobId);
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

  document.addEventListener("click", (event) => {
    if (state.detailOpenFilterColumn == null || event.target.closest(".column-filter-menu, .column-filter-button")) return;
    state.detailOpenFilterColumn = null;
    renderDetailRows();
  });

  window.addEventListener("resize", () => applyResultsPaneWidth());
}

export async function loadResult(jobId) {
  state.analysis = await fetchResult(jobId);
  state.selection = { type: "all" };
  state.expandedClusters = new Set();
  state.detailPage = 1;
  state.detailColumnWidths = [];
  state.detailColumnOrder = [];
  state.detailColumnFilters = [];
  state.detailOpenFilterColumn = null;
  state.detailSort = null;
  document.querySelector('[data-step="results"]').disabled = false;
  renderResults();
  showStep("results");
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
}

function renderClusterList() {
  const list = document.getElementById("clusterList");
  list.innerHTML = "";
  addClusterButton(list, "All incidents", { type: "all" });
  state.analysis.clusters.forEach((cluster) => {
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
    addClusterButton(
      row,
      `${clusterId(cluster.id)} - ${cluster.label} (${cluster.incident_row_indices.length})`,
      { type: "cluster", cluster: cluster.id },
      "cluster-label"
    );
    list.appendChild(row);

    if (expanded) {
      cluster.subgroups.forEach((theme) => {
        addClusterButton(
          list,
          `Theme ${theme.id} - ${theme.label} (${theme.incident_row_indices.length})`,
          { type: "theme", cluster: cluster.id, theme: theme.id },
          "theme"
        );
      });
    }
  });
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
  });
  list.appendChild(button);
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

function renderResultsTable(targetId, headers, rows) {
  const target = document.getElementById(targetId);
  if (!headers.length) {
    target.innerHTML = "";
    return;
  }
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
          .map((column) => `<td title="${escapeHtml(row[column] || "")}">${escapeHtml(row[column] || "")}</td>`)
          .join("")}</tr>`
    )
    .join("")}</tbody></table>`;
  bindColumnControls(target);
  bindColumnResizers(target);
  bindColumnDrag(target);
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

  target.querySelectorAll(".column-filter-option input").forEach((checkbox) => {
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
